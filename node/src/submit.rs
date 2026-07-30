//! Transaction/block submission and admission pipeline.

use base::{
    BlkPkg, BlockAcceptStatus, MempoolPolicy, Peer, TxAdmissionStatus, TxPkg, TxPoolInsertOutcome,
    TxPoolInsertReject, TxRejectReason, TxSubmitResult,
};
use sys::Rerr;

use crate::P2PNode;
use crate::msgqueue::InboundMsg;
use crate::p2p::msg::{MSG_BLOCK_DISCOVER, MSG_REQ_BLOCK, MSG_REQ_BLOCK_HASH, MSG_TX_SUBMIT};

impl P2PNode {
    pub(crate) fn submit_transaction_pkg(
        &self,
        tx: &TxPkg,
        is_async: bool,
        only_pool: bool,
        except_peer: Option<&str>,
    ) -> Rerr {
        if only_pool {
            return self.submit_transaction_direct(tx, true, except_peer);
        }
        if is_async {
            if self.inbound.is_started() {
                return self.inbound.enqueue_tx(
                    except_peer.map(|s| s.to_string()),
                    tx.data().as_ref().to_vec(),
                );
            }
            return self.submit_transaction_direct(tx, false, except_peer);
        }
        if self.inbound.is_started() {
            return self.inbound.submit_and_wait(InboundMsg::Tx {
                peer: except_peer.map(|s| s.to_string()),
                body: tx.data().as_ref().to_vec(),
                ack: None,
            });
        }
        self.submit_transaction_direct(tx, false, except_peer)
    }

    pub(crate) fn submit_transaction_direct(
        &self,
        tx: &TxPkg,
        only_pool: bool,
        except_peer: Option<&str>,
    ) -> Rerr {
        let is_remote = except_peer.is_some();
        let peer = except_peer.and_then(|id| self.peertable.get_snapshot(id));
        let hxfe = tx.tx().hash_with_fee();
        let (already, knowkey) = self.check_know(&hxfe, peer.as_deref());
        // Knowledge is only a relay deduplication hint.  A local submission
        // must remain retryable after a transient/rejected admission; mirror
        // dev by suppressing it only when it came from a peer or is already
        // present in the local pool with the same fee-bearing wire identity.
        let exact_in_pool = || {
            self.txpool
                .find(tx.hash().as_ref())
                .is_some_and(|stored| stored.tx().hash_with_fee() == hxfe)
        };
        if already && (is_remote || exact_in_pool()) {
            return Ok(());
        }
        let reject_tip = self.engine.latest_block().hash().into_array();
        if is_remote && self.tx_rejects.contains(&knowkey, &reject_tip) {
            return Ok(());
        }
        let result = self.admit_transaction_inner(tx, only_pool)?;
        if result.status == TxAdmissionStatus::Rejected && is_remote {
            self.tx_rejects.add(knowkey, reject_tip);
        }
        if should_remember_transaction(result.status) {
            self.remember_know(knowkey);
        }
        match result.status {
            TxAdmissionStatus::AcceptedBroadcast => {
                if result.should_relay() {
                    // Look up whether this tx's pool group is a selective relay
                    // channel declared by the consensus policy. The node does
                    // not name the channel; it only carries the bit.
                    let selective_bit = result.group.and_then(|g| {
                        self.engine
                            .tx_policy()
                            .tx_pool_groups()
                            .into_iter()
                            .find(|spec| spec.id == g)
                            .and_then(|spec| spec.relay_service_bit)
                    });
                    match selective_bit {
                        Some(bit) => self.broadcast_selective(
                            bit,
                            knowkey,
                            tx.data().as_ref().to_vec(),
                            except_peer,
                        )?,
                        None => self.broadcast_unaware(
                            knowkey,
                            MSG_TX_SUBMIT,
                            tx.data().as_ref().to_vec(),
                            except_peer,
                        )?,
                    }
                }
                Ok(())
            }
            TxAdmissionStatus::AcceptedPool
            | TxAdmissionStatus::Duplicate
            | TxAdmissionStatus::Replaced
            | TxAdmissionStatus::Ignored => Ok(()),
            TxAdmissionStatus::Rejected => sys::errf!("tx rejected: {:?}", result.reason),
        }
    }

    pub(crate) fn submit_block_pkg(&self, blk: &BlkPkg, except_peer: Option<&str>) -> Rerr {
        let is_remote = except_peer.is_some();
        let peer = except_peer.and_then(|id| self.peertable.get_snapshot(id));
        let (already, knowkey) = self.check_block_know(&blk.hash(), peer.as_deref());
        // As with transactions, global knowledge must not prevent a local
        // block from being retried when an earlier attempt was rejected or
        // held before persistence.
        if already
            && (is_remote
                || self
                    .engine
                    .store()
                    .block_hash(blk.height())
                    .is_some_and(|hash| hash == blk.hash()))
        {
            return Ok(());
        }
        let reject_tip = self.engine.latest_block().hash().into_array();
        if is_remote && self.block_rejects.contains(&knowkey, &reject_tip) {
            return Ok(());
        }

        if let Err(e) = self
            .engine
            .consensus()
            .check_block_data(blk.data().as_ref(), self.engine.as_ref())
        {
            if is_remote {
                self.block_rejects.add(knowkey, reject_tip);
            }
            return Err(e);
        }

        let admission = match self
            .engine
            .consensus()
            .check_block_admission(blk, self.engine.as_ref())
        {
            Ok(admission) => admission,
            Err(e) => {
                if is_remote {
                    self.block_rejects.add(knowkey, reject_tip);
                }
                return Err(e);
            }
        };
        let deferred = matches!(admission, base::BlockAdmissionDecision::Defer(_));
        let local_height = self.engine.latest_height();
        let heispan = self.engine.config().unstable_block;
        if !deferred && blk.height() > local_height + 1 {
            if let Some(ref p) = peer {
                let num = (heispan + 1).min(255) as u8;
                let mut req = Vec::with_capacity(9);
                req.push(num);
                req.extend_from_slice(&local_height.to_be_bytes());
                let _ = p.send_msg(MSG_REQ_BLOCK_HASH, req);
            }
            if local_height + heispan + 1 < blk.height() {
                println!(
                    "[P2P] ignore future block height={} local_head={}",
                    blk.height(),
                    local_height
                );
            }
            return sys::errf!(
                "future block height {} exceeds local head {}",
                blk.height(),
                local_height
            );
        }

        if let Err(e) = self
            .engine
            .consensus()
            .check_block_arrive_data(blk.data().as_ref(), self.engine.as_ref())
        {
            if is_remote {
                self.block_rejects.add(knowkey, reject_tip);
            }
            return Err(e);
        }

        let _insert_guard = self.inserting.lock().unwrap();
        let block_hash = blk.hash();
        let hx = block_hash.as_bytes();
        let hxstrt = if hx.len() >= 12 { &hx[4..12] } else { hx };
        let hxtail = if hx.len() >= 2 {
            &hx[hx.len() - 2..]
        } else {
            hx
        };
        let txs = blk.block().transaction_count().saturating_sub(1);
        let mshow = may_show_miner_detail(self.engine.config().show_miner_name, blk);
        print!(
            "block {} ...{}...{} txs{:2} insert at {} {}",
            blk.height(),
            to_hex(hxstrt),
            to_hex(hxtail),
            txs,
            &sys::ctshow()[11..],
            mshow
        );
        let result = if deferred {
            println!("deferred.");
            base::BlockAcceptResult::deferred()
        } else {
            match self.engine.discover_block(blk.clone()) {
                Ok(r) => {
                    println!("ok.");
                    r
                }
                Err(e) => {
                    println!("Error: {}", e);
                    if is_remote && e.code() != Some("deferred_sync") {
                        self.block_rejects.add(knowkey, reject_tip);
                    }
                    return Err(e);
                }
            }
        };
        drop(_insert_guard);

        if result.status == BlockAcceptStatus::Orphan {
            if let Some(parent) = result.requested_parents.first() {
                self.cache_orphan_block(*parent, blk.clone());
            }
            if let Some(ref p) = peer {
                if blk.height() > 1 {
                    let parent_hei = blk.height() - 1;
                    let _ = p.send_msg(MSG_REQ_BLOCK, parent_hei.to_be_bytes().to_vec());
                }
            }
            return Ok(());
        }

        if matches!(
            result.status,
            BlockAcceptStatus::Accepted
                | BlockAcceptStatus::Duplicate
                | BlockAcceptStatus::Deferred
        ) {
            self.remember_know(knowkey);
        }

        // Accepted and deferred blocks may both be relayable by policy.
        if result.should_relay() {
            self.broadcast_unaware(
                knowkey,
                MSG_BLOCK_DISCOVER,
                blk.data().as_ref().to_vec(),
                except_peer,
            )?;
        }
        if !result.confirmed_txs.is_empty() {
            self.txpool.drain(&result.confirmed_txs);
            self.engine.tx_policy().on_txs_confirmed(
                self.engine.as_ref(),
                self.txpool.as_ref(),
                result.confirmed_txs,
                blk.height(),
            );
        }
        self.drain_deferred_blocks();
        self.drain_orphans_for(&blk.hash());
        Ok(())
    }

    fn drain_orphans_for(&self, parent: &field::Hash) {
        for orphan in self.take_orphan_blocks(parent) {
            if let Err(e) = self.submit_block_pkg(&orphan, None) {
                eprintln!("[P2P] orphan retry failed: {}", e);
            }
        }
    }

    pub(crate) fn drain_all_orphans(&self) {
        for orphan in self.take_all_orphan_blocks() {
            if let Err(e) = self.submit_block_pkg(&orphan, None) {
                eprintln!("[P2P] orphan retry failed: {}", e);
            }
        }
    }

    pub(crate) fn handle_transaction_bytes(
        &self,
        body: Vec<u8>,
        peer: Option<String>,
        only_pool: bool,
    ) -> Rerr {
        let mut source =
            base::PkgSource::new(base::PkgOrigin::Broadcast).with_received_at(sys::curtimes());
        if let Some(peer) = peer {
            source = source.with_peer(peer);
        }
        let except_peer = source.peer.clone();
        let tx = TxPkg::from_bytes(self.engine.services().as_ref(), body, source)?;
        self.submit_transaction_direct(&tx, only_pool, except_peer.as_deref())
    }

    pub(crate) fn handle_block_bytes(&self, body: Vec<u8>, peer: Option<String>) -> Rerr {
        let is_remote = peer.is_some();
        let max = self.engine.consensus().mint_params().max_block_size;
        if max > 0 && body.len() > max.saturating_add(100) {
            return sys::errf!(
                "block wire size {} exceeds max payload {} plus header allowance",
                body.len(),
                max
            );
        }
        let mut source =
            base::PkgSource::new(base::PkgOrigin::Broadcast).with_received_at(sys::curtimes());
        if let Some(peer) = peer {
            source = source.with_peer(peer);
        }
        let except_peer = source.peer.clone();
        // This validation intentionally precedes full decoding. Use a separate
        // wire-key cache because the canonical block hash is not available yet.
        let wire_key = sys::calculate_hash(&body);
        let reject_tip = self.engine.latest_block().hash().into_array();
        if is_remote && self.block_wire_rejects.contains(&wire_key, &reject_tip) {
            return Ok(());
        }
        if let Err(e) = self
            .engine
            .consensus()
            .check_block_data(&body, self.engine.as_ref())
        {
            if is_remote {
                self.block_wire_rejects.add(wire_key, reject_tip);
            }
            return Err(e);
        }
        let block = match BlkPkg::from_bytes(self.engine.services().as_ref(), body, source) {
            Ok(block) => block,
            Err(e) => {
                if is_remote {
                    self.block_wire_rejects.add(wire_key, reject_tip);
                }
                return Err(e);
            }
        };
        self.submit_block_pkg(&block, except_peer.as_deref())
    }

    pub(crate) fn admit_transaction_inner(
        &self,
        tx: &TxPkg,
        only_pool: bool,
    ) -> sys::Ret<TxSubmitResult> {
        // 1. validate TxPkg
        // 2. profile / size / fee checks
        let mempool_policy = tx.tx().mempool_policy();
        if mempool_policy == MempoolPolicy::Forbidden {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::MempoolForbidden,
            ));
        }
        if mempool_policy == MempoolPolicy::OnlyLocal && !only_pool {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::Policy("transaction is restricted to local admission".to_owned()),
            ));
        }
        let params = self.engine.consensus().mint_params();
        if tx.size() > params.max_tx_size && params.max_tx_size > 0 {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::TooLarge {
                    size: tx.size(),
                    max: params.max_tx_size,
                },
            ));
        }
        let min_fee_purity = self.txpool.min_fee_purity();
        if tx.fee_purity() < min_fee_purity {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::FeeTooLow {
                    got: tx.fee_purity(),
                    min: min_fee_purity,
                },
            ));
        }
        // 3. try execute
        if let Err(e) = self.engine.try_execute_tx(tx.tx_ref()) {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::ExecutionFailed(e.to_string()),
            ));
        }
        // 4. protocol tx policy (mint check_tx — default Ok until mint fills in)
        if let Err(e) = self.engine.tx_policy().check_tx(self.engine.as_ref(), tx) {
            return Ok(TxSubmitResult::rejected(
                tx.hash(),
                TxRejectReason::Policy(e.to_string()),
            ));
        }
        // 5. insert into txpool
        let g = self.engine.tx_policy().tx_pool_group(tx);
        let outcome = self.txpool.insert(g, tx.clone())?;
        // 6. Local retention is independent from relay eligibility. A bounded
        // non-mining pool may decline storage without becoming a relay sink.
        Ok(admission_after_pool_insert(
            tx.hash(),
            g,
            only_pool,
            outcome,
        ))
    }

    /// Periodic / on-demand execution of consensus-owned deferred candidates.
    pub fn drain_deferred_blocks(&self) -> bool {
        let _insert_guard = self.inserting.lock().unwrap();
        let mut progressed = false;
        let mut accepted_hashes = Vec::new();
        for batch in self
            .engine
            .node_hooks()
            .poll_deferred_batches(self.engine.as_ref())
        {
            let mut batch_result = base::DeferredBatchResult::Exhausted;
            for (candidate_index, candidate) in batch.candidates.into_iter().enumerate() {
                let mut candidate_ok = true;
                for pkg in candidate.blocks {
                    let block_hash = pkg.hash();
                    match self.engine.discover_block(pkg) {
                        Ok(result) => {
                            if !matches!(
                                result.status,
                                BlockAcceptStatus::Accepted | BlockAcceptStatus::Duplicate
                            ) {
                                candidate_ok = false;
                                break;
                            }
                            if matches!(
                                result.status,
                                BlockAcceptStatus::Accepted | BlockAcceptStatus::Duplicate
                            ) {
                                progressed = true;
                            }
                            if !result.confirmed_txs.is_empty() {
                                self.txpool.drain(&result.confirmed_txs);
                                self.engine.tx_policy().on_txs_confirmed(
                                    self.engine.as_ref(),
                                    self.txpool.as_ref(),
                                    result.confirmed_txs,
                                    result.height.unwrap_or_else(|| self.engine.latest_height()),
                                );
                            }
                            if result.status == BlockAcceptStatus::Accepted {
                                accepted_hashes.push(block_hash);
                            }
                        }
                        Err(e) => {
                            eprintln!("[node] deferred block candidate failed: {}", e);
                            candidate_ok = false;
                            break;
                        }
                    }
                }
                if candidate_ok {
                    batch_result = base::DeferredBatchResult::Accepted {
                        candidate: candidate_index,
                    };
                    break;
                }
            }
            self.engine
                .node_hooks()
                .on_deferred_batch_result(batch.id, batch_result);
        }
        if progressed {
            for peer in self.peertable.values_snapshot() {
                let _ = peer.send_msg(crate::p2p::msg::MSG_REQ_STATUS, Vec::new());
            }
        }
        drop(_insert_guard);
        for hash in accepted_hashes {
            self.drain_orphans_for(&hash);
        }
        progressed
    }
}

fn admission_after_pool_insert(
    hash: field::Hash,
    group: base::TxGroupId,
    only_pool: bool,
    outcome: TxPoolInsertOutcome,
) -> TxSubmitResult {
    match (only_pool, outcome) {
        (false, _) => TxSubmitResult::accepted(hash, group, true),
        (true, TxPoolInsertOutcome::Stored) => TxSubmitResult::accepted(hash, group, false),
        (true, TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity)) => {
            TxSubmitResult::rejected(hash, TxRejectReason::PoolFull)
        }
        (true, TxPoolInsertOutcome::NotStored(TxPoolInsertReject::UnderpricedReplacement)) => {
            TxSubmitResult::rejected(
                hash,
                TxRejectReason::Policy(
                    "a higher-priority version already exists in the local pool".to_owned(),
                ),
            )
        }
    }
}

fn should_remember_transaction(status: TxAdmissionStatus) -> bool {
    status != TxAdmissionStatus::Rejected
}

#[cfg(test)]
mod tests {
    use super::{admission_after_pool_insert, should_remember_transaction};
    use base::{
        TxAdmissionStatus, TxGroupId, TxPoolInsertOutcome, TxPoolInsertReject, TxRejectReason,
    };
    use field::Hash;

    #[test]
    fn pool_capacity_does_not_block_relay() {
        let result = admission_after_pool_insert(
            Hash::default(),
            TxGroupId::DEFAULT,
            false,
            TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity),
        );
        assert_eq!(result.status, TxAdmissionStatus::AcceptedBroadcast);
        assert_eq!(result.group, Some(TxGroupId::DEFAULT));
        assert!(result.reason.is_none());
    }

    #[test]
    fn pool_replacement_policy_does_not_block_relay() {
        let result = admission_after_pool_insert(
            Hash::default(),
            TxGroupId::DEFAULT,
            false,
            TxPoolInsertOutcome::NotStored(TxPoolInsertReject::UnderpricedReplacement),
        );
        assert_eq!(result.status, TxAdmissionStatus::AcceptedBroadcast);
    }

    #[test]
    fn only_pool_keeps_strict_storage_semantics() {
        let result = admission_after_pool_insert(
            Hash::default(),
            TxGroupId::DEFAULT,
            true,
            TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity),
        );
        assert_eq!(result.status, TxAdmissionStatus::Rejected);
        assert_eq!(result.reason, Some(TxRejectReason::PoolFull));
    }

    #[test]
    fn rejected_admission_stays_retryable() {
        assert!(!should_remember_transaction(TxAdmissionStatus::Rejected));
        assert!(should_remember_transaction(
            TxAdmissionStatus::AcceptedBroadcast
        ));
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn may_show_miner_detail(show: bool, blk: &BlkPkg) -> String {
    if !show {
        return String::new();
    }
    let Ok(ptx) = blk.block().prelude_transaction() else {
        return String::new();
    };
    let Some(author) = ptx.author() else {
        return String::new();
    };
    let readable = author.to_readable();
    let adrt: String = readable.chars().take(9).collect();
    let message = ptx
        .block_message()
        .map(|msg| {
            sys::bytes_to_readable_string(msg.as_bytes())
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    format!("miner: {}...<{}> ", adrt, message)
}
