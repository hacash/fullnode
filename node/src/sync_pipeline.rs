//! Window downloader → one BlockStream → engine; protocol differences stay in
//! the downloader, engine only consumes `BlockSource`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base::{
    ApplyMode, BlockBatch, BlockSender, BlockStream, Peer, PipelineOptions, PipelineReport,
    ProgressSink,
};
use sys::Rerr;

use crate::P2PNode;
use crate::p2p::msg::{MSG_GET_BLOCKS, MSG_REQ_STATUS};
use crate::p2p::syncwire::{DEFAULT_MAX_BLOCKS, GetBlocks, SYNC_WINDOW};

const MAX_BLOCKING_ENQUEUE_TASKS: usize = 4;
const SYNC_WATCH_INTERVAL: Duration = Duration::from_secs(5);
const SYNC_STALL_TIMEOUT: Duration = Duration::from_secs(60);

fn blocking_enqueue_gate() -> &'static Arc<tokio::sync::Semaphore> {
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_BLOCKING_ENQUEUE_TASKS)))
}

struct SyncBatchPrint {
    peer_name: String,
    start_height: u64,
    end_height: u64,
    remote_tip: u64,
    inserting_printed: bool,
}

struct SyncProgressPrinter {
    last_activity: Arc<Mutex<Instant>>,
    pending_batches: Arc<Mutex<VecDeque<SyncBatchPrint>>>,
}

impl SyncProgressPrinter {
    fn new(
        last_activity: Arc<Mutex<Instant>>,
        pending_batches: Arc<Mutex<VecDeque<SyncBatchPrint>>>,
    ) -> Self {
        Self {
            last_activity,
            pending_batches,
        }
    }
}

impl ProgressSink for SyncProgressPrinter {
    fn on_pipeline_progress(&self, report: &PipelineReport) {
        if let Ok(mut last_activity) = self.last_activity.lock() {
            *last_activity = Instant::now();
        }
        if let Ok(mut batches) = self.pending_batches.lock() {
            // Keep output ordered by completed application batches.
            while let Some(batch) = batches.front_mut() {
                if report.final_height < batch.start_height {
                    break;
                }
                if !batch.inserting_printed {
                    sys::flush!(
                        "sync blocks from {} {}...",
                        batch.peer_name,
                        batch.start_height
                    );
                    let percent = batch.end_height as f64 / batch.remote_tip.max(1) as f64 * 100.0;
                    sys::flush!("{}({:.2}%) inserting...", batch.end_height, percent);
                    batch.inserting_printed = true;
                }
                if batch.end_height > report.final_height {
                    break;
                }
                batches.pop_front();
                println!("ok.");
            }
        }
    }
}

pub(crate) struct SyncSession {
    pub generation: u64,
    pub peer_id: String,
    pub sender: BlockSender,
    pub cancel: Arc<AtomicBool>,
    pub next_req_id: u64,
    pub next_start: u64,
    pub remote_tip: u64,
    /// request_id -> planned start.
    pub inflight: BTreeMap<u64, (u64, u32)>,
    /// Out-of-order responses (each is dispatched independently) buffered here
    /// until the next contiguous height is ready for the ordered chain pipeline.
    pub pending: BTreeMap<
        u64,
        (
            crate::p2p::syncwire::BlocksHeader,
            Vec<u8>,
            Arc<Vec<u32>>,
            Arc<Vec<base::BlockRef>>,
        ),
    >,
    pub next_apply_start: u64,
    /// Updated on either a network response or applying another 200 blocks.
    /// The watchdog must treat both as liveness during queue backpressure.
    pub last_activity: Arc<Mutex<Instant>>,
    /// Sync output is serialized after each batch is applied.
    pending_print_batches: Arc<Mutex<VecDeque<SyncBatchPrint>>>,
}

impl SyncSession {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.sender.finish();
    }

    fn window_limit(&self) -> usize {
        SYNC_WINDOW
    }

    fn can_fill_wire_window(&self) -> bool {
        let in_flight = self.inflight.len();
        in_flight < self.window_limit() && self.next_start <= self.remote_tip
    }

    fn stall_reason(&self, timeout: Duration) -> Option<String> {
        let elapsed = self
            .last_activity
            .lock()
            .map(|last| last.elapsed())
            .unwrap_or(timeout);
        if elapsed < timeout {
            return None;
        }
        Some(format!(
            "no response or apply progress for {}s; next_request={} next_apply={} remote_tip={} in_flight={} pending={} apply_queue={}/{}",
            elapsed.as_secs(),
            self.next_start,
            self.next_apply_start,
            self.remote_tip,
            self.inflight.len(),
            self.pending.len(),
            self.sender.len(),
            self.sender.capacity(),
        ))
    }
}

pub type SyncSlot = Mutex<Option<SyncSession>>;

impl P2PNode {
    /// Ask peers to advertise their tips after a downloader failure; others are
    /// queried before the failed source so the first valid STATUS moves the retry.
    pub(crate) fn request_sync_status_candidates(&self, retry_last: Option<&str>) -> usize {
        if self.stopping.load(Ordering::Acquire) {
            return 0;
        }
        let mut peers = self.peertable.values_snapshot();
        peers.sort_by_key(|peer| retry_last.is_some_and(|id| peer.id == id));
        peers
            .into_iter()
            .filter(|peer| peer.send_msg(MSG_REQ_STATUS, Vec::new()).is_ok())
            .count()
    }

    pub(crate) fn mark_sync_failure(&self, peer_id: &str, reason: &str) {
        self.sync_tracker.clear_peer(peer_id);
        if !self.stopping.load(Ordering::Acquire) {
            eprintln!("[P2P] sync with peer {} ended: {}", peer_id, reason);
        }
    }

    /// Stop and release the current downloader. A later STATUS or normal peer
    /// reconnect may acquire the tracker again, matching fullnodedev.
    pub(crate) fn stop_sync_session(&self, peer_id: &str, reason: &str) {
        let stopped = self
            .sync_session
            .lock()
            .ok()
            .and_then(|mut g| {
                if g.as_ref().is_some_and(|session| session.peer_id == peer_id) {
                    g.take().map(|session| {
                        session.cancel();
                    })
                } else {
                    None
                }
            })
            .is_some();
        if stopped {
            self.mark_sync_failure(peer_id, reason);
        }
    }

    /// Start bulk sync: shared apply thread + window downloader.
    pub(crate) fn start_sync_pipe(
        self: &Arc<Self>,
        peer: Arc<dyn Peer>,
        start_height: u64,
        remote_tip: u64,
    ) -> Rerr {
        if self.stopping.load(Ordering::Acquire) || start_height == 0 || remote_tip < start_height {
            return Ok(());
        }
        let peer_id = peer.id();
        let cap = self.config.block_queue_cap.clamp(2, SYNC_WINDOW);
        let (stream, sender) = BlockStream::with_capacity(cap);
        let cancel = Arc::new(AtomicBool::new(false));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let pending_print_batches = Arc::new(Mutex::new(VecDeque::new()));
        let mut opts = PipelineOptions::default();
        opts.cancel = Some(cancel.clone());
        opts.progress_sink = Some(Arc::new(SyncProgressPrinter::new(
            last_activity.clone(),
            pending_print_batches.clone(),
        )));
        let sync_mode = if self.engine.config().fast_sync {
            ApplyMode::FastSync
        } else {
            ApplyMode::Strict
        };
        let generation = {
            // Claim the downloader slot and tracker as one operation. STATUS or
            // fork-hash replies must never replace a healthy in-flight source.
            let mut slot = self.sync_session.lock().unwrap();
            if self.stopping.load(Ordering::Acquire) || slot.is_some() {
                return Ok(());
            }
            self.try_begin_sync(&peer_id, remote_tip);
            let generation = self.sync_generation.fetch_add(1, Ordering::AcqRel) + 1;
            *slot = Some(SyncSession {
                generation,
                peer_id: peer_id.clone(),
                sender: sender.clone(),
                cancel: cancel.clone(),
                next_req_id: 1,
                next_start: start_height,
                remote_tip,
                inflight: BTreeMap::new(),
                pending: BTreeMap::new(),
                next_apply_start: start_height,
                last_activity,
                pending_print_batches,
            });
            generation
        };
        let engine = self.engine.clone();
        let txpool = self.txpool.clone();
        let sync_session = self.sync_session.clone();
        let sync_tracker = self.sync_tracker.clone();
        let node_for_apply = self.clone();
        let cleanup_peer_id = peer_id.clone();
        let inserting = self.inserting.clone();
        let spawn_result = std::thread::Builder::new()
            .name("node-sync-apply".into())
            .spawn(move || {
                // Serialize block application; release the guard before post-processing
                // since orphan retries and deferred one-shot batches acquire the same lock.
                let result = {
                    let _insert_guard = inserting.lock().unwrap();
                    engine
                        .run_sync(Box::new(stream), sync_mode, opts)
                        .and_then(|handle| handle.wait())
                };
                match result {
                    Ok(report) => {
                        let current = sync_session.lock().ok().is_some_and(|g| {
                            g.as_ref().is_some_and(|s| {
                                s.generation == generation && s.peer_id == cleanup_peer_id
                            })
                        });
                        if !current {
                            return;
                        }
                        let remote_tip = sync_tracker
                            .active_remote_height()
                            .unwrap_or(report.final_height);
                        // The apply pipeline may stop at a held external block while
                        // responses are in flight: stop that session before replaying.
                        if let Ok(mut g) = sync_session.lock() {
                            if g.as_ref().is_some_and(|s| {
                                s.generation == generation && s.peer_id == cleanup_peer_id
                            }) {
                                if let Some(session) = g.take() {
                                    session.cancel();
                                }
                            }
                        }
                        node_for_apply.drain_all_orphans();
                        if report.held_blocks.is_empty() {
                            sync_tracker.finish(&cleanup_peer_id, remote_tip);
                        } else {
                            sync_tracker.clear_peer(&cleanup_peer_id);
                        }
                        if report.held_blocks.is_empty() && report.final_height >= remote_tip {
                            println!("all blocks sync finished.");
                        }
                        for (height, txs) in report.confirmed_txs {
                            txpool.drain(&txs);
                            engine.tx_policy().on_txs_confirmed(
                                engine.as_ref(),
                                txpool.as_ref(),
                                txs,
                                height,
                            );
                        }
                        if !report.held_blocks.is_empty() {
                            eprintln!(
                                "[P2P] sync held {} external blocks",
                                report.held_blocks.len()
                            );
                        }
                        let _ = node_for_apply.drain_deferred_blocks();
                    }
                    Err(e) => {
                        let current = sync_session.lock().ok().is_some_and(|g| {
                            g.as_ref().is_some_and(|s| {
                                s.generation == generation && s.peer_id == cleanup_peer_id
                            })
                        });
                        if !current {
                            return;
                        }
                        println!("{}", e);
                        node_for_apply
                            .stop_sync_session(&cleanup_peer_id, "apply pipeline failure");
                    }
                }
            });
        if spawn_result.is_err() {
            if let Ok(mut g) = self.sync_session.lock() {
                *g = None;
            }
            self.mark_sync_failure(&peer_id, "failed to spawn sync apply thread");
            return sys::errf!("failed to spawn sync apply thread");
        }

        let sync_session_watch = self.sync_session.clone();
        let watch_node = self.clone();
        let watch_peer = peer.clone();
        let watch_peer_id = peer_id.clone();
        let watcher_name = format!("node-sync-watch-{generation}");
        if let Err(e) = std::thread::Builder::new()
            .name(watcher_name)
            .spawn(move || loop {
                std::thread::sleep(SYNC_WATCH_INTERVAL);
                if watch_node.stopping.load(Ordering::Acquire) {
                    return;
                }

                let (stalled, refill) = {
                    let mut slot = sync_session_watch.lock().unwrap();
                    let Some(session) = slot.as_ref() else {
                        return;
                    };
                    if session.peer_id != watch_peer_id || session.generation != generation {
                        return;
                    }
                    if let Some(reason) = session.stall_reason(SYNC_STALL_TIMEOUT) {
                        if let Some(session) = slot.take() {
                            session.cancel();
                        }
                        (Some(reason), false)
                    } else {
                        (None, session.can_fill_wire_window())
                    }
                };

                if let Some(reason) = stalled {
                    let reason = format!(
                        "{} local_head={}",
                        reason,
                        watch_node.engine.latest_height()
                    );
                    watch_node.mark_sync_failure(&watch_peer_id, &reason);
                    if watch_node.request_sync_status_candidates(Some(&watch_peer_id)) == 0 {
                        eprintln!(
                            "[P2P] sync recovery has no connected STATUS candidates after peer {} stalled",
                            watch_peer_id
                        );
                    }
                    return;
                }

                if refill
                    && let Err(e) = watch_node.sync_fill_window(watch_peer.clone())
                {
                    eprintln!(
                        "[P2P] sync watchdog refill with peer {} failed: {}",
                        watch_peer_id, e
                    );
                }
            })
        {
            eprintln!("[P2P] failed to spawn sync watchdog: {}", e);
        }

        self.sync_fill_window(peer)
    }

    /// Issue download requests until the wire window is full or tip is covered.
    pub(crate) fn sync_fill_window(&self, peer: Arc<dyn Peer>) -> Rerr {
        let peer_id = peer.id();
        loop {
            let work = {
                let mut g = self.sync_session.lock().unwrap();
                if self.stopping.load(Ordering::Acquire) {
                    return Ok(());
                }
                let Some(sess) = g.as_mut() else {
                    return Ok(());
                };
                if sess.peer_id != peer_id {
                    return Ok(());
                }
                // Response enqueueing already backpressures: stopping refill on a
                // full queue can strand the downloader, since dequeueing has no refill callback.
                if !sess.can_fill_wire_window() {
                    return Ok(());
                }
                let req_id = sess.next_req_id;
                sess.next_req_id = sess.next_req_id.saturating_add(1);
                let start = sess.next_start;
                sess.next_start = start.saturating_add(DEFAULT_MAX_BLOCKS as u64);
                if sess.next_start <= start {
                    sess.next_start = start + 1;
                }
                sess.inflight.insert(req_id, (start, DEFAULT_MAX_BLOCKS));
                Some((req_id, start, GetBlocks::new(req_id, start).encode()))
            };
            let Some((req_id, start, body)) = work else {
                return Ok(());
            };
            let ty = MSG_GET_BLOCKS as u16;
            if let Err(e) = peer.send_msg(ty, body) {
                let cancelled = {
                    let mut g = self.sync_session.lock().unwrap();
                    if let Some(sess) = g.as_mut() {
                        sess.inflight.remove(&req_id);
                        if sess.next_start > start {
                            sess.next_start = start;
                        }
                    }
                    g.take()
                        .map(|session| {
                            session.cancel();
                        })
                        .is_some()
                };
                if cancelled {
                    self.mark_sync_failure(
                        &peer_id,
                        &format!("request at height {} failed: {}", start, e),
                    );
                }
                return Err(e);
            }
        }
    }

    /// Handle MSG_BLOCKS: push into apply queue and refill window.
    pub(crate) async fn handle_blocks_message(
        self: &Arc<Self>,
        peer: Arc<dyn Peer>,
        body: Vec<u8>,
    ) -> Rerr {
        let peer_id = peer.id();
        let (hdr, blocks) = match crate::p2p::syncwire::BlocksHeader::decode(&body) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.stop_sync_session(&peer_id, &format!("invalid response header: {}", e));
                return Err(e);
            }
        };
        let peer_name = peer.name();
        let blocks = blocks.to_vec();
        let (offsets, decoded_blocks) = match validate_blocks_payload(
            self.engine.services().as_ref(),
            &blocks,
            hdr.count,
            hdr.start_height,
        ) {
            Ok((offsets, decoded_blocks)) => (Arc::new(offsets), Arc::new(decoded_blocks)),
            Err(e) => {
                self.stop_sync_session(&peer_id, &format!("invalid block response: {}", e));
                return Err(e);
            }
        };

        // Orphan recovery uses one-block GET_BLOCKS id 0 (session ids start at 1, so no
        // collision); skipped while a session runs — the cached parent is replayed after.
        if hdr.request_id == 0 && hdr.count == 1 {
            if self.sync_session.lock().ok().is_some_and(|g| g.is_some()) {
                return Ok(());
            }
            let batch = base::BlockBatch {
                bytes: Arc::new(blocks),
                remote_height: hdr.remote_tip,
                block_count: 1,
                block_offsets: offsets,
                decoded_blocks,
            };
            let node = self.clone();
            return tokio::task::spawn_blocking(move || {
                node.apply_oneshot_blocks(hdr.start_height, batch)
            })
            .await
            .map_err(|e| sys::Error::fault(format!("p2p one-shot apply task failed: {}", e)))?;
        }

        let (sender, ready, caught_up, pending_print_batches) = {
            let mut g = self.sync_session.lock().unwrap();
            let Some(sess) = g.as_mut() else {
                return Ok(());
            };
            if sess.peer_id != peer_id {
                return Ok(());
            }
            let Some((planned_start, planned_max_blocks)) =
                sess.inflight.get(&hdr.request_id).copied()
            else {
                return Ok(());
            };
            if hdr.start_height != planned_start {
                let e = sys::Error::fault(format!(
                    "BLOCKS start mismatch req={} planned={} got={}",
                    hdr.request_id, planned_start, hdr.start_height
                ));
                drop(g);
                self.stop_sync_session(&peer_id, &e.to_string());
                return Err(e);
            }
            if hdr.count > planned_max_blocks as u64 {
                let e = sys::Error::fault(format!(
                    "BLOCKS count {} exceeds request limit {}",
                    hdr.count, planned_max_blocks
                ));
                drop(g);
                self.stop_sync_session(&peer_id, &e.to_string());
                return Err(e);
            }
            sess.inflight.remove(&hdr.request_id);
            sess.remote_tip = sess.remote_tip.max(hdr.remote_tip);
            if let Ok(mut last_activity) = sess.last_activity.lock() {
                *last_activity = Instant::now();
            }
            sess.pending
                .insert(hdr.start_height, (hdr, blocks, offsets, decoded_blocks));

            let mut ready = Vec::new();
            while let Some((item_hdr, item_blocks, item_offsets, item_decoded_blocks)) =
                sess.pending.remove(&sess.next_apply_start)
            {
                let planned_end = sess
                    .next_apply_start
                    .saturating_add(DEFAULT_MAX_BLOCKS as u64)
                    .saturating_sub(1);
                let short = item_hdr.end_height < planned_end.min(sess.remote_tip);
                let next = item_hdr.end_height.saturating_add(1);
                if next <= sess.next_apply_start {
                    return sys::errf!("BLOCKS response does not advance at height {}", next);
                }
                sess.next_apply_start = next;
                ready.push((
                    item_blocks,
                    item_hdr.remote_tip,
                    item_hdr.count,
                    item_offsets,
                    item_decoded_blocks,
                    SyncBatchPrint {
                        peer_name: peer_name.clone(),
                        start_height: item_hdr.start_height,
                        end_height: item_hdr.end_height,
                        remote_tip: item_hdr.remote_tip,
                        inserting_printed: false,
                    },
                ));
                if short {
                    // Requests beyond a short response leave a gap: discard them and
                    // resume at the first height not included by the short response.
                    sess.inflight.retain(|_, (start, _)| *start < next);
                    sess.pending.retain(|start, _| *start < next);
                    sess.next_start = next;
                    break;
                }
            }

            let applied_through = sess.next_apply_start.saturating_sub(1);
            let caught_up = applied_through >= sess.remote_tip
                && sess.inflight.is_empty()
                && sess.pending.is_empty();
            let sender = sess.sender.clone();
            (sender, ready, caught_up, sess.pending_print_batches.clone())
        };

        for (blocks, remote_tip, count, offsets, decoded_blocks, print) in ready {
            if let Ok(mut batches) = pending_print_batches.lock() {
                batches.push_back(print);
            }
            if !blocks.is_empty() {
                if let Err(e) = push_validated_blocks(
                    sender.clone(),
                    blocks,
                    remote_tip,
                    count,
                    offsets,
                    decoded_blocks,
                )
                .await
                {
                    self.stop_sync_session(&peer_id, &format!("enqueue blocks failed: {}", e));
                    return Err(e);
                }
            }
        }
        if caught_up {
            sender.finish();
            return Ok(());
        }
        self.sync_fill_window(peer)
    }
}

async fn push_validated_blocks(
    sender: BlockSender,
    blocks: Vec<u8>,
    remote_tip: u64,
    count: u64,
    offsets: Arc<Vec<u32>>,
    decoded_blocks: Arc<Vec<base::BlockRef>>,
) -> Rerr {
    let batch = BlockBatch {
        bytes: Arc::new(blocks),
        remote_height: remote_tip,
        block_count: u32::try_from(count)
            .map_err(|_| sys::Error::fault("BLOCKS count exceeds u32".to_owned()))?,
        block_offsets: offsets,
        decoded_blocks,
    };
    match sender.try_push_block_batch(batch.clone())? {
        true => Ok(()),
        false => enqueue_block_batch(sender, batch)
            .await
            .map_err(|e| sys::Error::fault(format!("p2p block enqueue task failed: {}", e))),
    }
}

async fn enqueue_block_batch(sender: BlockSender, batch: BlockBatch) -> Rerr {
    let permit = blocking_enqueue_gate()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| sys::Error::fault("p2p block enqueue gate closed"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        sender.push_block_batch(batch)
    })
    .await
    .map_err(|e| sys::Error::fault(format!("p2p block enqueue task failed: {e}")))?
}

pub(crate) fn validate_blocks_payload(
    registry: &dyn base::BinaryCodecs,
    blocks: &[u8],
    count: u64,
    start_height: u64,
) -> sys::Ret<(Vec<u32>, Vec<base::BlockRef>)> {
    if blocks.is_empty() {
        return sys::errf!("BLOCKS payload is empty for {} declared blocks", count);
    }
    if count > blocks.len() as u64 {
        return sys::errf!(
            "BLOCKS declared count {} exceeds payload byte length {}",
            count,
            blocks.len()
        );
    }
    let mut off = 0usize;
    let initial_capacity = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .min(blocks.len());
    let mut offsets = Vec::with_capacity(initial_capacity);
    let mut decoded_blocks = Vec::with_capacity(initial_capacity);
    for idx in 0..count {
        if off >= blocks.len() {
            return sys::errf!(
                "BLOCKS payload ended before declared block {} of {}",
                idx + 1,
                count
            );
        }
        offsets.push(
            u32::try_from(off)
                .map_err(|_| sys::Error::fault("BLOCKS payload offset exceeds u32".to_owned()))?,
        );
        let (block, used) = registry.decode_block(&blocks[off..])?;
        if used == 0 || off.saturating_add(used) > blocks.len() {
            return sys::errf!("BLOCKS payload has incomplete block {}", idx + 1);
        }
        let expected_height = start_height.saturating_add(idx);
        if block.height() != expected_height {
            return sys::errf!(
                "BLOCKS payload block {} height mismatch: header expects {} but payload has {}",
                idx + 1,
                expected_height,
                block.height()
            );
        }
        decoded_blocks.push(block);
        off += used;
    }
    if off != blocks.len() {
        return sys::errf!(
            "BLOCKS payload has {} trailing bytes after {} blocks",
            blocks.len() - off,
            count
        );
    }
    Ok((offsets, decoded_blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_apply_queue_does_not_close_the_wire_window() {
        let (_stream, sender) = BlockStream::with_capacity(1);
        assert!(
            sender
                .try_push_block_batch(BlockBatch::raw(Arc::new(vec![1]), 10_000))
                .unwrap()
        );

        let session = SyncSession {
            generation: 1,
            peer_id: "peer".into(),
            sender,
            cancel: Arc::new(AtomicBool::new(false)),
            next_req_id: 3,
            next_start: 4_001,
            remote_tip: 10_000,
            inflight: BTreeMap::from([(2, (2_001, DEFAULT_MAX_BLOCKS))]),
            pending: BTreeMap::new(),
            next_apply_start: 1,
            last_activity: Arc::new(Mutex::new(Instant::now())),
            pending_print_batches: Arc::new(Mutex::new(VecDeque::new())),
        };

        assert!(session.sender.is_full());
        assert!(session.can_fill_wire_window());
    }

    #[test]
    fn stalled_session_reports_queue_cursors() {
        let (_stream, sender) = BlockStream::with_capacity(2);
        let session = SyncSession {
            generation: 1,
            peer_id: "peer".into(),
            sender,
            cancel: Arc::new(AtomicBool::new(false)),
            next_req_id: 1,
            next_start: 550_001,
            remote_tip: 769_089,
            inflight: BTreeMap::from([(1, (550_001, DEFAULT_MAX_BLOCKS))]),
            pending: BTreeMap::new(),
            next_apply_start: 550_001,
            last_activity: Arc::new(Mutex::new(
                Instant::now() - SYNC_STALL_TIMEOUT - Duration::from_secs(1),
            )),
            pending_print_batches: Arc::new(Mutex::new(VecDeque::new())),
        };

        let reason = session.stall_reason(SYNC_STALL_TIMEOUT).unwrap();
        assert!(reason.contains("next_request=550001"));
        assert!(reason.contains("remote_tip=769089"));
    }

    #[test]
    fn active_session_is_not_reported_as_stalled() {
        let (_stream, sender) = BlockStream::with_capacity(2);
        let session = SyncSession {
            generation: 1,
            peer_id: "peer".into(),
            sender,
            cancel: Arc::new(AtomicBool::new(false)),
            next_req_id: 1,
            next_start: 550_001,
            remote_tip: 769_089,
            inflight: BTreeMap::new(),
            pending: BTreeMap::new(),
            next_apply_start: 550_001,
            last_activity: Arc::new(Mutex::new(Instant::now())),
            pending_print_batches: Arc::new(Mutex::new(VecDeque::new())),
        };

        assert!(session.stall_reason(SYNC_STALL_TIMEOUT).is_none());
        assert!(session.can_fill_wire_window());
    }

    #[test]
    fn progress_does_not_start_the_next_batch_early() {
        let batches = Arc::new(Mutex::new(VecDeque::from([
            SyncBatchPrint {
                peer_name: "peer".into(),
                start_height: 100,
                end_height: 199,
                remote_tip: 299,
                inserting_printed: false,
            },
            SyncBatchPrint {
                peer_name: "peer".into(),
                start_height: 200,
                end_height: 299,
                remote_tip: 299,
                inserting_printed: false,
            },
        ])));
        let printer =
            SyncProgressPrinter::new(Arc::new(Mutex::new(Instant::now())), batches.clone());

        printer.on_pipeline_progress(&PipelineReport {
            final_height: 199,
            ..Default::default()
        });

        let batches = batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches.front().unwrap().start_height, 200);
        assert!(!batches.front().unwrap().inserting_printed);
    }
}
