//! P2P message dispatch and protocol handlers (status, block sync, tx/block submit).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use base::{ApplyMode, BlockBatch, PipelineOptions};
use field::Hash;
use sys::Rerr;

use crate::P2PNode;
use crate::p2p::msg::v2::{MSG_BLOCKS, MSG_GET_BLOCKS};
use crate::p2p::msg::{
    MSG_BLOCK, MSG_BLOCK_DISCOVER, MSG_BLOCK_HASH, MSG_REQ_BLOCK, MSG_REQ_BLOCK_HASH,
    MSG_REQ_STATUS, MSG_STATUS, MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE,
};
use crate::p2p::source::OneShotBlocks;
use crate::p2p::syncwire::{BLOCKS_HDR_VERSION, BLOCKS_HEADER_SIZE, BlocksHeader, GetBlocks};

const SYNC_SESSION_TAKEOVER_IDLE: std::time::Duration = std::time::Duration::from_secs(10);

fn may_replace_sync_session(
    active_peer: &str,
    last_activity: &std::sync::Mutex<std::time::Instant>,
    candidate_peer: &str,
) -> bool {
    active_peer == candidate_peer
        || last_activity
            .lock()
            .map(|last| last.elapsed() >= SYNC_SESSION_TAKEOVER_IDLE)
            .unwrap_or(true)
}

impl P2PNode {
    fn status_message(&self) -> Vec<u8> {
        // Mainnet HandshakeStatus (78 bytes), field order matches combi_struct.
        let genesis = self.engine.consensus().genesis_block().hash();
        let latest = self.engine.latest_block();
        let height = latest.height();
        let mut body = Vec::with_capacity(78);
        body.extend_from_slice(genesis.as_ref()); // 32
        body.push(1); // block_version
        body.push(2); // transaction_type
        body.extend_from_slice(&12u16.to_be_bytes()); // action_kind
        body.extend_from_slice(&1u16.to_be_bytes()); // repair_serial
        body.extend_from_slice(&[0u8; 3]); // __mark
        // latest_height: BlockHeight = Uint5
        let hb = height.to_be_bytes();
        body.extend_from_slice(&hb[3..8]);
        body.extend_from_slice(latest.hash().as_ref()); // 32
        debug_assert_eq!(body.len(), 78);
        body
    }

    pub(crate) fn try_begin_sync(
        &self,
        peer_id: &str,
        start_height: u64,
        remote_height: u64,
    ) -> bool {
        let node_key = self
            .peertable
            .get_snapshot(peer_id)
            .map(|peer| peer.node_key);
        self.sync_tracker
            .begin_or_refresh(peer_id, node_key, start_height, remote_height)
    }

    pub(crate) fn mark_doing_sync(&self) {
        self.doing_sync.store(sys::curtimes(), Ordering::Relaxed);
    }

    fn request_blocks_after_fork(
        self: &Arc<Self>,
        peer: Arc<dyn base::Peer>,
        start_height: u64,
        remote_height: u64,
    ) -> Rerr {
        // Both v1 and v2 feed the same BlockStream apply thread.
        self.start_sync_pipe(peer, start_height, remote_height)
    }

    /// Start sync when a peer STATUS reports a height ahead of the local tip.
    fn maybe_sync_from_remote_height(
        self: &Arc<Self>,
        peer: Arc<dyn base::Peer>,
        remote_height: u64,
    ) -> Rerr {
        if remote_height <= self.engine.latest_height() {
            return Ok(());
        }
        let peer_id = peer.id();
        let replaced_peer = {
            let mut slot = self.sync_session.lock().unwrap();
            let Some(active) = slot.as_ref() else {
                drop(slot);
                return self.start_sync_from_status(peer, remote_height);
            };
            let may_replace =
                may_replace_sync_session(&active.peer_id, &active.last_activity, &peer_id);
            if !may_replace {
                return Ok(());
            }
            slot.take().map(|session| {
                let old_peer = session.peer_id.clone();
                session.cancel();
                old_peer
            })
        };
        if let Some(old_peer) = replaced_peer {
            self.sync_tracker.clear_peer(&old_peer);
            self.doing_sync.store(0, Ordering::Release);
        }
        self.start_sync_from_status(peer, remote_height)
    }

    fn start_sync_from_status(
        self: &Arc<Self>,
        peer: Arc<dyn base::Peer>,
        remote_height: u64,
    ) -> Rerr {
        let local_height = self.engine.latest_height();
        if remote_height <= local_height {
            return Ok(());
        }
        if local_height == 0 {
            return self.request_blocks_after_fork(peer, 1, remote_height);
        }
        let peer_id = peer.id();
        if !self.try_begin_sync(&peer_id, local_height + 1, remote_height) {
            return Ok(());
        }
        self.mark_doing_sync();
        let num = self.engine.config().unstable_block.min(80) as u8;
        let mut req = Vec::with_capacity(9);
        req.push(num);
        req.extend_from_slice(&local_height.to_be_bytes());
        let _ = peer.send_msg(MSG_REQ_BLOCK_HASH, req);
        Ok(())
    }

    fn handle_status_message(self: &Arc<Self>, peer: Arc<dyn base::Peer>, body: Vec<u8>) -> Rerr {
        if body.len() != 78 {
            peer.disconnect();
            return Ok(());
        }
        let genesis = Hash::from(body[0..32].try_into().unwrap());
        let local_genesis = self.engine.consensus().genesis_block().hash();
        if genesis != local_genesis {
            peer.disconnect();
            return Ok(());
        }
        let mut height_buf = [0u8; 8];
        height_buf[3..8].copy_from_slice(&body[41..46]);
        let remote_height = u64::from_be_bytes(height_buf);
        self.maybe_sync_from_remote_height(peer, remote_height)
    }

    fn handle_req_block_hash_message(&self, peer: Arc<dyn base::Peer>, body: Vec<u8>) -> Rerr {
        if body.len() != 9 {
            return sys::errf!("p2p req block hash message length invalid: {}", body.len());
        }
        let num = body[0] as u64;
        let end_height = u64::from_be_bytes(body[1..9].try_into().unwrap());
        let latest_height = self.engine.latest_height();
        if num > 80 || end_height > latest_height {
            return Ok(());
        }
        // Legacy fullnodedev treats the request count as the number of
        // preceding links and includes the endpoint itself, so a request for
        // `num` returns `num + 1` hashes whenever height zero is not reached.
        // Keep that off-by-one wire behavior for v1 compatibility.
        let start_height = if num >= end_height {
            1
        } else {
            end_height - num
        };
        let store = self.engine.store();
        let hash_count = end_height
            .checked_sub(start_height)
            .map(|count| count.saturating_add(1))
            .unwrap_or(0);
        let mut res = Vec::with_capacity(8 + hash_count as usize * 32);
        res.extend_from_slice(&end_height.to_be_bytes());
        for height in (start_height..=end_height).rev() {
            let Some(hash) = store.block_hash(height) else {
                return Ok(());
            };
            res.extend_from_slice(hash.as_ref());
        }
        peer.send_msg(MSG_BLOCK_HASH, res)
    }

    fn handle_block_hash_message(
        self: &Arc<Self>,
        peer: Arc<dyn base::Peer>,
        body: Vec<u8>,
    ) -> Rerr {
        if body.len() < 8 {
            return sys::errf!("p2p block hash message length invalid: {}", body.len());
        }
        let end_height = u64::from_be_bytes(body[0..8].try_into().unwrap());
        let hashes = &body[8..];
        if hashes.is_empty() || hashes.len() % Hash::SIZE != 0 {
            return sys::errf!("p2p block hash list length invalid: {}", hashes.len());
        }
        let hash_num = (hashes.len() / Hash::SIZE) as u64;
        if end_height == 0 || end_height > self.engine.latest_height() {
            return Ok(());
        }
        let max_num = (self.engine.config().unstable_block as u64 + 1).min(hash_num);
        let start_height = end_height.saturating_sub(max_num);
        let store = self.engine.store();
        for (idx, height) in ((start_height + 1)..=end_height).rev().enumerate() {
            let Some(local_hash) = store.block_hash(height) else {
                return Ok(());
            };
            let off = idx * Hash::SIZE;
            let remote_hash = Hash::from(hashes[off..off + Hash::SIZE].try_into().unwrap());
            if remote_hash == local_hash {
                let next = height + 1;
                let remote_tip = self
                    .sync_tracker
                    .active_remote_height()
                    .unwrap_or(end_height)
                    .max(next);
                return self.request_blocks_after_fork(peer, next, remote_tip);
            }
        }
        Ok(())
    }

    /// v1 (and orphan) REQ_BLOCK: 8-byte start → MSG_BLOCK response.
    fn handle_req_block_message(&self, peer: Arc<dyn base::Peer>, body: Vec<u8>) -> Rerr {
        if body.len() != 8 {
            return sys::errf!("p2p req block message length invalid: {}", body.len());
        }
        let start_height = u64::from_be_bytes(body[0..8].try_into().unwrap());
        let latest_height = self.engine.latest_height();
        if start_height == 0 || start_height > latest_height {
            return Ok(());
        }
        let max_send_size = 20 * 1024 * 1024usize;
        let max_send_num = 10_000usize;
        let (end_height, total_num, blocks) =
            self.collect_blocks(start_height, max_send_num, max_send_size)?;
        if blocks.is_empty() {
            return Ok(());
        }
        // v1: 24B header; transitional v2 peers still speaking MSG_BLOCK get 32B.
        let header_size = if peer.protocol_version() >= 2 { 32 } else { 24 };
        let mut res = Vec::with_capacity(header_size + blocks.len());
        res.extend_from_slice(&latest_height.to_be_bytes());
        res.extend_from_slice(&start_height.to_be_bytes());
        res.extend_from_slice(&end_height.to_be_bytes());
        if peer.protocol_version() >= 2 {
            res.extend_from_slice(&(total_num as u64).to_be_bytes());
        }
        res.extend_from_slice(&blocks);
        peer.send_msg(MSG_BLOCK, res)
    }

    /// v2 GET_BLOCKS → MSG_BLOCKS (pipelined).
    fn handle_get_blocks_message(&self, peer: Arc<dyn base::Peer>, body: Vec<u8>) -> Rerr {
        let req = GetBlocks::decode(&body)?;
        if req.start_height == 0 {
            return Ok(());
        }
        let latest_height = self.engine.latest_height();
        if req.start_height > latest_height {
            return Ok(());
        }
        let max_blocks = (req.max_blocks as usize).clamp(1, 10_000);
        let max_bytes = (req.max_bytes as usize).clamp(64 * 1024, 32 * 1024 * 1024);
        let max_bytes = max_bytes.min(P2P_MSG_DATA_MAX_SIZE.saturating_sub(BLOCKS_HEADER_SIZE));
        let (end_height, total_num, blocks) =
            self.collect_blocks(req.start_height, max_blocks, max_bytes)?;
        if blocks.is_empty() {
            return Ok(());
        }
        let more = end_height < latest_height;
        let hdr = BlocksHeader {
            remote_tip: latest_height,
            start_height: req.start_height,
            end_height,
            count: total_num as u64,
            request_id: req.request_id,
            more,
            flags: 0,
            hdr_version: BLOCKS_HDR_VERSION,
        };
        let mut res = Vec::with_capacity(44 + blocks.len());
        res.extend_from_slice(&hdr.encode());
        res.extend_from_slice(&blocks);
        peer.send_msg(MSG_BLOCKS as u16, res)
    }

    fn collect_blocks(
        &self,
        start_height: u64,
        max_num: usize,
        max_bytes: usize,
    ) -> sys::Ret<(u64, usize, Vec<u8>)> {
        let latest_height = self.engine.latest_height();
        let mut total_size = 0usize;
        let mut total_num = 0usize;
        let mut end_height = 0u64;
        let mut blocks = Vec::new();
        let store = self.engine.store();
        for height in start_height..=latest_height {
            let Some((_, data)) = store.block_data_by_height(height) else {
                break;
            };
            let next_size = total_size.saturating_add(data.len());
            if total_num > 0 && next_size > max_bytes {
                break;
            }
            if total_num == 0 && data.len() > max_bytes {
                return sys::errf!(
                    "block at height {} exceeds requested sync payload limit {}",
                    height,
                    max_bytes
                );
            }
            total_size = next_size;
            total_num += 1;
            end_height = height;
            blocks.extend_from_slice(data.as_ref());
            if total_num >= max_num || total_size == max_bytes {
                break;
            }
        }
        Ok((end_height, total_num, blocks))
    }

    /// v1 MSG_BLOCK: prefer unified sync ingress; oneshot fallback for orphan fetches.
    async fn handle_blocks_message(
        self: &Arc<Self>,
        peer: Arc<dyn base::Peer>,
        body: Vec<u8>,
    ) -> Rerr {
        // Transitional: some v2 peers may still speak MSG_BLOCK (32B header).
        let is_v2_hdr = peer.protocol_version() >= 2;
        let header_size = if is_v2_hdr { 32 } else { 24 };
        if body.len() < header_size {
            return sys::errf!("p2p blocks message length invalid: {}", body.len());
        }
        let remote_height = u64::from_be_bytes(body[0..8].try_into().unwrap());
        let start_height = u64::from_be_bytes(body[8..16].try_into().unwrap());
        let end_height = u64::from_be_bytes(body[16..24].try_into().unwrap());
        if start_height == 0 || end_height < start_height {
            return sys::errf!(
                "p2p blocks invalid height span {}..{}",
                start_height,
                end_height
            );
        }
        let expected_count = end_height - start_height + 1;
        if is_v2_hdr {
            let block_count = u64::from_be_bytes(body[24..32].try_into().unwrap());
            if block_count == 0 || block_count > 10_000 {
                return sys::errf!("p2p blocks count out of range: {}", block_count);
            }
            if block_count != expected_count {
                return sys::errf!(
                    "p2p blocks count mismatch: declared {} span {}",
                    block_count,
                    expected_count
                );
            }
        }
        let blocks = body[header_size..].to_vec();
        if blocks.is_empty() {
            return sys::errf!(
                "p2p blocks response for {}..{} contains no block payload",
                start_height,
                end_height
            );
        }
        // v1 has no explicit count field; derive it from the advertised
        // height span and validate the concatenated frames before advancing
        // the sync cursor. This keeps a malformed response from desynchronizing
        // the session even though the legacy wire header is accepted.
        let (offsets, decoded_blocks) = match crate::sync_pipeline::validate_blocks_payload(
            self.engine.services().as_ref(),
            &blocks,
            expected_count,
            start_height,
        ) {
            Ok(validated) => validated,
            Err(e) => {
                self.stop_sync_session(&peer.id(), &format!("invalid v1 block response: {}", e));
                return Err(e);
            }
        };
        let batch = BlockBatch {
            bytes: Arc::new(blocks),
            remote_height,
            block_count: u32::try_from(expected_count)
                .map_err(|_| sys::Error::fault("v1 block count exceeds u32".to_owned()))?,
            block_offsets: Arc::new(offsets),
            decoded_blocks: Arc::new(decoded_blocks),
        };

        if self
            .try_ingest_v1_sync_batch(
                peer.clone(),
                start_height,
                end_height,
                remote_height,
                batch.clone(),
            )
            .await?
        {
            return Ok(());
        }

        // During an active sync session, unmatched responses are stale or an
        // orphan race. Drop them rather than replaying them after cancellation.
        if self.sync_session.lock().unwrap().is_some() {
            return Ok(());
        }

        // No sync session (e.g. orphan parent fetch) - oneshot apply.
        let node = self.clone();
        tokio::task::spawn_blocking(move || {
            node.apply_oneshot_blocks(peer, start_height, end_height, remote_height, batch)
        })
        .await
        .map_err(|e| sys::Error::fault(format!("p2p one-shot apply task failed: {}", e)))?
    }

    /// Ad-hoc apply when no SyncSession is active (orphan / one-off REQ_BLOCK).
    pub(crate) fn apply_oneshot_blocks(
        &self,
        peer: Arc<dyn base::Peer>,
        start_height: u64,
        _end_height: u64,
        remote_height: u64,
        batch: BlockBatch,
    ) -> Rerr {
        let peer_id = peer.id();
        if !self.sync_tracker.claim_batch(&peer_id, start_height) {
            // If tracker has no state, allow a soft begin for orphan.
            if !self.try_begin_sync(&peer_id, start_height, remote_height)
                || !self.sync_tracker.claim_batch(&peer_id, start_height)
            {
                return Ok(());
            }
        }
        let cfg = self.engine.config();
        let opts = PipelineOptions::default();
        let sync_mode = if cfg.fast_sync {
            ApplyMode::FastSync
        } else {
            ApplyMode::Strict
        };
        let sync_result = {
            let _lk = self.inserting.lock().unwrap();
            self.engine
                .run_sync(Box::new(OneShotBlocks::from_batch(batch)), sync_mode, opts)
        };
        let (held, final_height) = match sync_result.and_then(|h| h.wait()) {
            Ok(report) => {
                let held = !report.held_blocks.is_empty();
                let final_height = report.final_height;
                self.drain_all_orphans();
                for (height, txs) in report.confirmed_txs {
                    self.txpool.drain(&txs);
                    self.engine.tx_policy().on_txs_confirmed(
                        self.engine.as_ref(),
                        self.txpool.as_ref(),
                        txs,
                        height,
                    );
                }
                if held {
                    self.drain_deferred_blocks();
                }
                (held, final_height)
            }
            Err(e) => {
                self.sync_tracker.release_batch(&peer_id);
                return Err(e);
            }
        };
        if held {
            self.sync_tracker.clear_peer(&peer_id);
        } else {
            self.sync_tracker.finish_if_done(
                &peer_id,
                final_height.saturating_add(1),
                remote_height,
            );
        }
        Ok(())
    }

    pub(crate) async fn handle_message(
        self: &Arc<Self>,
        peer: Option<Arc<dyn base::Peer>>,
        ty: u16,
        body: Vec<u8>,
    ) -> Rerr {
        match ty {
            MSG_TX_SUBMIT => self.handle_transaction_bytes(body, peer.map(|p| p.id()), false),
            MSG_BLOCK_DISCOVER => self.handle_block_bytes(body, peer.map(|p| p.id())),
            MSG_REQ_STATUS => {
                let Some(peer) = peer else {
                    return sys::errf!("status request missing peer");
                };
                peer.send_msg(MSG_STATUS, self.status_message())
            }
            MSG_STATUS => {
                let Some(peer) = peer else {
                    return sys::errf!("status message missing peer");
                };
                self.handle_status_message(peer, body)
            }
            MSG_REQ_BLOCK_HASH => {
                let Some(peer) = peer else {
                    return sys::errf!("block hash request missing peer");
                };
                self.handle_req_block_hash_message(peer, body)
            }
            MSG_BLOCK_HASH => {
                let Some(peer) = peer else {
                    return sys::errf!("block hash message missing peer");
                };
                self.handle_block_hash_message(peer, body)
            }
            MSG_REQ_BLOCK => {
                let Some(peer) = peer else {
                    return sys::errf!("block request missing peer");
                };
                self.handle_req_block_message(peer, body)
            }
            MSG_BLOCK => {
                let Some(peer) = peer else {
                    return sys::errf!("blocks message missing peer");
                };
                self.handle_blocks_message(peer, body).await
            }
            ty if ty == MSG_GET_BLOCKS as u16 => {
                let Some(peer) = peer else {
                    return sys::errf!("get_blocks missing peer");
                };
                self.handle_get_blocks_message(peer, body)
            }
            ty if ty == MSG_BLOCKS as u16 => {
                let Some(peer) = peer else {
                    return sys::errf!("blocks(v2) missing peer");
                };
                self.handle_v2_blocks_message(peer, body).await
            }
            _ => sys::errf!("p2p message type {} not implemented", ty),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::may_replace_sync_session;

    #[test]
    fn same_connection_can_restart_its_sync_session() {
        assert!(may_replace_sync_session(
            "peer-a",
            &Mutex::new(Instant::now()),
            "peer-a"
        ));
    }

    #[test]
    fn another_connection_can_only_take_over_a_stale_session() {
        assert!(!may_replace_sync_session(
            "peer-a",
            &Mutex::new(Instant::now()),
            "peer-b"
        ));
        assert!(may_replace_sync_session(
            "peer-a",
            &Mutex::new(Instant::now() - Duration::from_secs(10)),
            "peer-b"
        ));
    }
}
