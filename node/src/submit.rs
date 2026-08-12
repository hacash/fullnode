//! Transaction/block submission and admission pipeline.

use base::{
    BlkPkg, BlockAcceptStatus, MempoolPolicy, Peer, TxAdmissionStatus, TxPkg, TxPoolInsertOutcome,
    TxPoolInsertReject, TxRejectReason, TxSubmitResult,
};
use sys::Rerr;

use crate::P2PNode;
use crate::msgqueue::InboundMsg;
use crate::p2p::msg::{
    MSG_BLOCK_DISCOVER, MSG_GET_BLOCKS, MSG_REQ_BLOCK_HASH, MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE,
};
use crate::p2p::syncwire::{BLOCKS_HEADER_SIZE, GetBlocks};

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
        let result = self.admit_transaction_inner(tx, only_pool)?;
        match result.status {
            TxAdmissionStatus::AcceptedBroadcast => {
                if result.should_relay() {
                    self.broadcast_unaware(
                        knowkey,
                        MSG_TX_SUBMIT,
                        tx.data().as_ref().to_vec(),
                        except_peer,
                    )?;
                }
                Ok(())
            }
            TxAdmissionStatus::AcceptedPool
            | TxAdmissionStatus::Duplicate
            | TxAdmissionStatus::Replaced
            | TxAdmissionStatus::Ignored => Ok(()),
            TxAdmissionStatus::Rejected => sys::errf!(
                "{}",
                result
                    .reason
                    .as_ref()
                    .map(|r| r.as_message())
                    .unwrap_or_else(|| "transaction rejected".to_owned())
            ),
        }
    }

    pub(crate) fn submit_block_pkg(&self, blk: &BlkPkg, except_peer: Option<&str>) -> Rerr {
        let is_remote = except_peer.is_some();
        let peer = except_peer.and_then(|id| self.peertable.get_snapshot(id));
        let (already, knowkey) = self.check_know(&blk.hash(), peer.as_deref());
        // As with transactions, global knowledge must not prevent a local
        // block from being retried when an earlier attempt was rejected or
        // held before persistence.
        if already
            && (is_remote
                || self
                    .engine
                    .store()
                    .block_hash(blk.height())?
                    .is_some_and(|hash| hash == blk.hash()))
        {
            return Ok(());
        }
        self.engine
            .consensus()
            .check_block_data(blk.data().as_ref(), self.engine.as_ref())?;

        let local_height = self.engine.latest_height();
        let heispan = self.engine.config().unstable_block;
        if blk.height() > local_height + 1 {
            if let Some(ref p) = peer {
                let num = (heispan + 1) as u8;
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
        let result = match self.engine.discover_block(blk.clone()) {
            Ok(r) => {
                println!("ok.");
                r
            }
            Err(e) => {
                println!("Error: {}", e);
                return Err(e);
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
                    let mut request = GetBlocks::new(0, parent_hei);
                    request.max_blocks = 1;
                    // Raise to the frame ceiling: the responder's collect_blocks
                    // rejects a first block larger than the requested max_bytes,
                    // so an oversized parent would otherwise never be served.
                    request.max_bytes = (P2P_MSG_DATA_MAX_SIZE - BLOCKS_HEADER_SIZE) as u32;
                    let _ = p.send_msg(MSG_GET_BLOCKS as u16, request.encode());
                }
            }
            return Ok(());
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
        // An active sync session is the only sanctioned downloader: drop
        // broadcast blocks without decoding, locking or caching. The sync
        // stream covers the vast majority of them; a tip block arriving at
        // the tail is recovered by later broadcasts and orphan retries.
        if peer.is_some() && self.sync_session.lock().ok().is_some_and(|g| g.is_some()) {
            return Ok(());
        }
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
        self.engine
            .consensus()
            .check_block_data(&body, self.engine.as_ref())?;
        let block = BlkPkg::from_bytes(self.engine.services().as_ref(), body, source)?;
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
        // 6. The original node relays only after a successful pool insertion.
        Ok(admission_after_pool_insert(
            tx.hash(),
            g,
            only_pool,
            outcome,
        ))
    }

    /// Periodic / on-demand execution of consensus-owned deferred candidates.
    pub fn drain_deferred_blocks(&self) -> bool {
        // Skip while a sync session runs: the periodic worker would only
        // block on `inserting` until the pipeline ends, and the sync's own
        // post-processing drains deferred blocks anyway.
        if self.sync_session.lock().ok().is_some_and(|g| g.is_some()) {
            return false;
        }
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
    match outcome {
        TxPoolInsertOutcome::Stored => TxSubmitResult::accepted(hash, group, !only_pool),
        TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity) => {
            TxSubmitResult::rejected(hash, TxRejectReason::PoolFull)
        }
        TxPoolInsertOutcome::NotStored(TxPoolInsertReject::UnderpricedReplacement) => {
            TxSubmitResult::rejected(
                hash,
                TxRejectReason::Policy(
                    "a higher-priority version already exists in the local pool".to_owned(),
                ),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::admission_after_pool_insert;
    use base::{
        TxAdmissionStatus, TxGroupId, TxPoolInsertOutcome, TxPoolInsertReject, TxRejectReason,
    };
    use field::Hash;

    #[test]
    fn pool_capacity_rejects_in_all_submission_modes() {
        for only_pool in [false, true] {
            let result = admission_after_pool_insert(
                Hash::default(),
                TxGroupId::DEFAULT,
                only_pool,
                TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity),
            );
            assert_eq!(result.status, TxAdmissionStatus::Rejected);
            assert_eq!(result.reason, Some(TxRejectReason::PoolFull));
        }
    }

    #[test]
    fn underpriced_replacement_rejects_in_all_submission_modes() {
        for only_pool in [false, true] {
            let result = admission_after_pool_insert(
                Hash::default(),
                TxGroupId::DEFAULT,
                only_pool,
                TxPoolInsertOutcome::NotStored(TxPoolInsertReject::UnderpricedReplacement),
            );
            assert_eq!(result.status, TxAdmissionStatus::Rejected);
            assert!(matches!(result.reason, Some(TxRejectReason::Policy(_))));
        }
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
