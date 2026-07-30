//! Unified sync ingress: v1 serial / v2 window downloaders → one BlockStream → engine.
//!
//! Protocol differences stay in the downloader; engine only consumes `BlockSource`.

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
use crate::p2p::msg::v2::MSG_GET_BLOCKS;
use crate::p2p::msg::{MSG_REQ_BLOCK, MSG_REQ_STATUS};
use crate::p2p::syncwire::{DEFAULT_MAX_BLOCKS, GetBlocks, SYNC_WINDOW};

const MAX_BLOCKING_ENQUEUE_TASKS: usize = 4;

fn blocking_enqueue_gate() -> &'static Arc<tokio::sync::Semaphore> {
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_BLOCKING_ENQUEUE_TASKS)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyncWire {
    /// Serial REQ_BLOCK / MSG_BLOCK (window = 1).
    V1,
    /// Concurrent GET_BLOCKS / MSG_BLOCKS (window = SYNC_WINDOW).
    V2,
}

struct LegacySyncBatchPrint {
    peer_name: String,
    start_height: u64,
    end_height: u64,
    remote_tip: u64,
    inserting_printed: bool,
}

struct SyncProgressPrinter {
    last_activity: Arc<Mutex<Instant>>,
    pending_batches: Arc<Mutex<VecDeque<LegacySyncBatchPrint>>>,
}

impl SyncProgressPrinter {
    fn new(
        last_activity: Arc<Mutex<Instant>>,
        pending_batches: Arc<Mutex<VecDeque<LegacySyncBatchPrint>>>,
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
            // The downloader may request the next legacy batch while this
            // one is still being applied. Keep the historic output ordered
            // by completed application batches without changing that flow.
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
    pub wire: SyncWire,
    pub sender: BlockSender,
    pub cancel: Arc<AtomicBool>,
    pub next_req_id: u64,
    pub next_start: u64,
    pub remote_tip: u64,
    /// request_id -> planned start (v2). v1 uses a single synthetic id 0.
    pub inflight: BTreeMap<u64, (u64, u32)>,
    /// v2 responses can complete out of order because each response is
    /// dispatched independently. Keep decoded payloads here until the next
    /// contiguous height is available for the ordered chain pipeline.
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
    /// Updated on either a network response or applying another 1,000 blocks.
    /// The watchdog must treat both as liveness during queue backpressure.
    pub last_activity: Arc<Mutex<Instant>>,
    /// V1 legacy sync output is serialized after each batch is applied.
    pending_print_batches: Arc<Mutex<VecDeque<LegacySyncBatchPrint>>>,
    /// v1: true while waiting for the outstanding REQ_BLOCK response.
    pub v1_waiting: bool,
}

impl SyncSession {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.sender.finish();
    }

    fn window_limit(&self) -> usize {
        match self.wire {
            SyncWire::V1 => 1,
            SyncWire::V2 => SYNC_WINDOW,
        }
    }
}

pub type SyncSlot = Mutex<Option<SyncSession>>;

impl P2PNode {
    pub(crate) fn mark_sync_failure(&self, peer_id: &str, reason: &str) {
        if self.stopping.load(Ordering::Acquire) {
            self.sync_tracker.clear_peer(peer_id);
            self.doing_sync.store(0, Ordering::Release);
            return;
        }
        let terminal = self.engine.config().fast_sync;
        if terminal {
            self.fast_sync_terminal.store(true, Ordering::Release);
        }
        self.sync_tracker.halt_peer(peer_id);
        self.doing_sync.store(0, Ordering::Release);
        if terminal {
            eprintln!(
                "[Block Sync Warning] peer={} stopped FastSync permanently for this run: {}",
                peer_id, reason
            );
        } else {
            eprintln!("[Block Sync Warning] peer={} stopped: {}", peer_id, reason);
        }
    }

    /// Stop the active downloader without issuing another STATUS request.
    /// A malformed response or an apply failure is deterministic for the
    /// current peer/session, so restarting it immediately only hides the
    /// original error and can mix a late legacy response into the new stream.
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

    /// Start bulk sync: shared apply thread + version-specific downloader.
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
        let wire = if peer.protocol_version() >= 2 {
            SyncWire::V2
        } else {
            SyncWire::V1
        };
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
            if self.stopping.load(Ordering::Acquire)
                || slot.is_some()
                || !self.try_begin_sync(&peer_id, start_height, remote_tip)
            {
                return Ok(());
            }
            self.mark_doing_sync();
            let generation = self.sync_generation.fetch_add(1, Ordering::AcqRel) + 1;
            *slot = Some(SyncSession {
                generation,
                peer_id: peer_id.clone(),
                wire,
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
                v1_waiting: false,
            });
            generation
        };
        let engine = self.engine.clone();
        let txpool = self.txpool.clone();
        let sync_session = self.sync_session.clone();
        let sync_tracker = self.sync_tracker.clone();
        let peertable = self.peertable.clone();
        let node_for_apply = self.clone();
        let cleanup_peer_id = peer_id.clone();
        let inserting = self.inserting.clone();
        let spawn_result = std::thread::Builder::new()
            .name("node-sync-apply".into())
            .spawn(move || {
                // Serialize block application itself.  Release the guard
                // before post-processing, because orphan retries and
                // deferred one-shot batches acquire the same lock.
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
                        // The apply pipeline may stop at a held external block
                        // while downloader responses are still in flight. Stop
                        // that session before replaying, otherwise stale
                        // responses can fill a queue with no consumer.
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
                            sync_tracker.finish_if_done(
                                &cleanup_peer_id,
                                report.final_height.saturating_add(1),
                                remote_tip,
                            );
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
                        let replay_drained = node_for_apply.drain_deferred_blocks();
                        // A held block is intentionally not retried until the
                        // consensus replay policy releases it. The periodic
                        // replay drain requests STATUS after it succeeds and
                        // starts the next sync window.
                        if report.held_blocks.is_empty() || replay_drained {
                            if let Some(peer) = peertable.get_snapshot(&cleanup_peer_id) {
                                let _ = peer.send_msg(MSG_REQ_STATUS, Vec::new());
                            }
                        }
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
                        node_for_apply.stop_sync_session(
                            &cleanup_peer_id,
                            "apply pipeline failure; automatic retry disabled",
                        );
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
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                if watch_node.stopping.load(Ordering::Acquire) {
                    return;
                }
                let (timed_out, refill_window) = {
                    let mut g = sync_session_watch.lock().unwrap();
                    let Some(session) = g.as_ref() else {
                        return;
                    };
                    if session.peer_id != watch_peer_id || session.generation != generation {
                        return;
                    }
                    if session.last_activity.lock().is_ok_and(|last_activity| {
                        last_activity.elapsed() < Duration::from_secs(30)
                    }) {
                        (false, !session.sender.is_full())
                    } else {
                        let session = g.take().unwrap();
                        session.cancel();
                        (true, false)
                    }
                };
                if timed_out {
                    if watch_node.stopping.load(Ordering::Acquire) {
                        return;
                    }
                    watch_node.mark_sync_failure(
                        &watch_peer_id,
                        &format!("range {}..={} timed out", start_height, remote_tip),
                    );
                    return;
                }
                if refill_window {
                    if let Err(e) = watch_node.sync_fill_window(watch_peer.clone()) {
                        eprintln!(
                            "[Block Sync Warning] peer={} refill request failed: {}",
                            watch_peer_id, e
                        );
                    }
                }
            }
        });

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
                let in_flight = match sess.wire {
                    SyncWire::V1 => {
                        if sess.v1_waiting {
                            1
                        } else {
                            0
                        }
                    }
                    SyncWire::V2 => sess.inflight.len(),
                };
                if in_flight >= sess.window_limit()
                    || sess.next_start > sess.remote_tip
                    || sess.sender.is_full()
                {
                    return Ok(());
                }
                match sess.wire {
                    SyncWire::V1 => {
                        let start = sess.next_start;
                        sess.v1_waiting = true;
                        sess.inflight.insert(0, (start, 1));
                        Some((SyncWire::V1, 0u64, start, start.to_be_bytes().to_vec()))
                    }
                    SyncWire::V2 => {
                        let req_id = sess.next_req_id;
                        sess.next_req_id = sess.next_req_id.saturating_add(1);
                        let start = sess.next_start;
                        sess.next_start = start.saturating_add(DEFAULT_MAX_BLOCKS as u64);
                        if sess.next_start <= start {
                            sess.next_start = start + 1;
                        }
                        sess.inflight.insert(req_id, (start, DEFAULT_MAX_BLOCKS));
                        Some((
                            SyncWire::V2,
                            req_id,
                            start,
                            GetBlocks::new(req_id, start).encode(),
                        ))
                    }
                }
            };
            let Some((wire, req_id, start, body)) = work else {
                return Ok(());
            };
            let ty = match wire {
                SyncWire::V1 => MSG_REQ_BLOCK,
                SyncWire::V2 => MSG_GET_BLOCKS as u16,
            };
            if let Err(e) = peer.send_msg(ty, body) {
                let cancelled = {
                    let mut g = self.sync_session.lock().unwrap();
                    if let Some(sess) = g.as_mut() {
                        sess.inflight.remove(&req_id);
                        if wire == SyncWire::V1 {
                            sess.v1_waiting = false;
                        } else if sess.next_start > start {
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
            if wire == SyncWire::V1 {
                // Serial: only one outstanding request. Its old-style log is
                // emitted when the corresponding batch reaches the pipeline.
                return Ok(());
            }
        }
    }

    /// Ingest a v1 MSG_BLOCK batch into the shared apply queue (active V1 session).
    /// Returns `true` if handled as sync; `false` if no matching session (caller may oneshot).
    pub(crate) async fn try_ingest_v1_sync_batch(
        self: &Arc<Self>,
        peer: Arc<dyn Peer>,
        start_height: u64,
        end_height: u64,
        remote_tip: u64,
        batch: BlockBatch,
    ) -> sys::Ret<bool> {
        let peer_id = peer.id();
        let peer_name = peer.name();
        let (sender, caught_up, pending_print_batches) = {
            let mut g = self.sync_session.lock().unwrap();
            let Some(sess) = g.as_mut() else {
                return Ok(false);
            };
            if sess.wire != SyncWire::V1 || sess.peer_id != peer_id {
                return Ok(false);
            }
            let Some((expected, _)) = sess.inflight.get(&0).copied() else {
                return Ok(false);
            };
            if expected != start_height {
                return Ok(false);
            }
            sess.inflight.remove(&0);
            sess.v1_waiting = false;
            sess.remote_tip = sess.remote_tip.max(remote_tip);
            sess.next_start = end_height.saturating_add(1);
            if let Ok(mut last_activity) = sess.last_activity.lock() {
                *last_activity = Instant::now();
            }

            let caught_up = end_height >= sess.remote_tip;
            let sender = sess.sender.clone();
            (sender, caught_up, sess.pending_print_batches.clone())
        };

        if let Ok(mut batches) = pending_print_batches.lock() {
            batches.push_back(LegacySyncBatchPrint {
                peer_name,
                start_height,
                end_height,
                remote_tip,
                inserting_printed: false,
            });
        }

        if caught_up {
            if !batch.bytes.is_empty() {
                if let Err(e) = push_block_batch(sender.clone(), batch, remote_tip).await {
                    self.stop_sync_session(&peer_id, &format!("enqueue v1 blocks failed: {}", e));
                    return Err(e);
                }
            }
            sender.finish();
            return Ok(true);
        }

        if !batch.bytes.is_empty() {
            if let Err(e) = push_block_batch(sender.clone(), batch, remote_tip).await {
                self.stop_sync_session(&peer_id, &format!("enqueue v1 blocks failed: {}", e));
                return Err(e);
            }
        }
        self.sync_tracker
            .finish_if_done(&peer_id, end_height + 1, remote_tip);
        self.mark_doing_sync();

        // Prefer same peer; fall back to another backbone if needed.
        let next_peer = self
            .peertable
            .try_switch_peer(&peer_id)
            .map(|p| p as Arc<dyn Peer>)
            .unwrap_or(peer);
        if next_peer.id() != peer_id {
            // Peer switched mid-sync: restart session on the new peer.
            let tip = self
                .sync_tracker
                .active_remote_height()
                .unwrap_or(remote_tip);
            return self
                .start_sync_pipe(next_peer, end_height + 1, tip)
                .map(|_| true);
        }
        self.sync_fill_window(next_peer).map(|_| true)
    }

    /// Handle v2 MSG_BLOCKS: push into apply queue and refill window.
    pub(crate) async fn handle_v2_blocks_message(
        &self,
        peer: Arc<dyn Peer>,
        body: Vec<u8>,
    ) -> Rerr {
        let peer_id = peer.id();
        let (hdr, blocks) = match crate::p2p::syncwire::BlocksHeader::decode(&body) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.stop_sync_session(&peer_id, &format!("invalid v2 response header: {}", e));
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
                self.stop_sync_session(&peer_id, &format!("invalid v2 block response: {}", e));
                return Err(e);
            }
        };

        let (sender, ready, caught_up, pending_print_batches) = {
            let mut g = self.sync_session.lock().unwrap();
            let Some(sess) = g.as_mut() else {
                return Ok(());
            };
            if sess.wire != SyncWire::V2 || sess.peer_id != peer_id {
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
                    LegacySyncBatchPrint {
                        peer_name: peer_name.clone(),
                        start_height: item_hdr.start_height,
                        end_height: item_hdr.end_height,
                        remote_tip: item_hdr.remote_tip,
                        inserting_printed: false,
                    },
                ));
                if short {
                    // Requests beyond a short response would leave a gap.
                    // Discard those responses/requests and resume at the
                    // first height not included by the short response.
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
                    self.stop_sync_session(&peer_id, &format!("enqueue v2 blocks failed: {}", e));
                    return Err(e);
                }
            }
        }
        if caught_up {
            sender.finish();
            return Ok(());
        }
        self.mark_doing_sync();
        self.sync_fill_window(peer)
    }
}

async fn push_block_batch(sender: BlockSender, batch: BlockBatch, remote_tip: u64) -> Rerr {
    match sender.try_push_block_batch(batch.clone())? {
        true => Ok(()),
        false => enqueue_block_batch(sender, batch)
            .await
            .map_err(|e| {
                sys::Error::fault(format!(
                    "p2p block enqueue at remote height {} failed: {}",
                    remote_tip, e
                ))
            }),
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
    fn progress_does_not_start_the_next_batch_early() {
        let batches = Arc::new(Mutex::new(VecDeque::from([
            LegacySyncBatchPrint {
                peer_name: "peer".into(),
                start_height: 100,
                end_height: 199,
                remote_tip: 299,
                inserting_printed: false,
            },
            LegacySyncBatchPrint {
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
