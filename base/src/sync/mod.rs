//! Sync pipeline: apply modes, pipeline options, sync handles, block source.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::BlockRef;
use crate::chain::PkgOrigin;
use field::Hash;
use sys::{Rerr, Ret};

/// Pipeline tuning options (see `Default`).
#[derive(Clone)]
pub struct PipelineOptions {
    pub decode_workers: usize,
    pub decode_queue: usize,
    /// Shared cancellation signal for the source and pipeline stages.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Optional progress cell shared with `SyncHandle`.
    pub progress: Option<Arc<Mutex<PipelineReport>>>,
    pub progress_sink: Option<Arc<dyn ProgressSink>>,
    /// Origin propagated to decoded blocks, root-roll events and persistence.
    pub origin: PkgOrigin,
}

impl std::fmt::Debug for PipelineOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineOptions")
            .field("decode_workers", &self.decode_workers)
            .field("decode_queue", &self.decode_queue)
            .field("cancel", &self.cancel.as_ref().map(|_| "Some(..)"))
            .field("progress", &self.progress.as_ref().map(|_| "Some(..)"))
            .field(
                "progress_sink",
                &self.progress_sink.as_ref().map(|_| "Some(..)"),
            )
            .finish()
    }
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            decode_workers: 2,
            decode_queue: 32,
            cancel: None,
            progress: None,
            progress_sink: None,
            origin: PkgOrigin::Sync,
        }
    }
}

pub trait ProgressSink: Send + Sync {
    fn on_pipeline_progress(&self, report: &PipelineReport);
}

/// Progress/result report of one pipeline run.
#[derive(Clone, Debug, Default)]
pub struct PipelineReport {
    pub accepted: u64,
    pub rolled: u64,
    pub events: u64,
    pub final_height: u64,
    pub confirmed_txs: Vec<(u64, Vec<Hash>)>,
    pub reverted_txs: Vec<(u64, Vec<Hash>)>,
    pub held_blocks: Vec<(u64, Hash)>,
    pub failure_height: Option<u64>,
    pub failure_message: Option<String>,
}

/// Handle to a running pipeline: `wait`, `progress`, `cancel`.
pub struct SyncHandle {
    cancel: Option<Arc<AtomicBool>>,
    progress: Arc<Mutex<PipelineReport>>,
    join: Option<JoinHandle<Ret<PipelineReport>>>,
}

impl SyncHandle {
    pub fn done(report: PipelineReport) -> Self {
        Self {
            cancel: None,
            progress: Arc::new(Mutex::new(report)),
            join: None,
        }
    }
    pub fn background(
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<PipelineReport>>,
        join: JoinHandle<Ret<PipelineReport>>,
    ) -> Self {
        Self {
            cancel: Some(cancel),
            progress,
            join: Some(join),
        }
    }
    pub fn wait(mut self) -> Ret<PipelineReport> {
        if let Some(join) = self.join.take() {
            let report = join
                .join()
                .map_err(|_| sys::Error::fault("sync task panicked"))??;
            if let Ok(mut g) = self.progress.lock() {
                *g = report.clone();
            }
            Ok(report)
        } else {
            Ok(self.progress())
        }
    }
    pub fn progress(&self) -> PipelineReport {
        self.progress.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn cancel(&self) {
        if let Some(flag) = &self.cancel {
            flag.store(true, Ordering::Release);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }
    pub fn is_background(&self) -> bool {
        self.join.is_some()
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        if self.join.is_some() {
            self.cancel();
        }
    }
}

pub struct BackgroundSyncHandle {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<PipelineReport>>,
}

impl BackgroundSyncHandle {
    pub fn new(cancel: Arc<AtomicBool>, progress: Arc<Mutex<PipelineReport>>) -> Self {
        Self { cancel, progress }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    pub fn progress(&self) -> PipelineReport {
        self.progress.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// Block source feeding a pipeline: P2P stream, replay, or oneshot.
pub trait BlockSource: Send + 'static {
    fn set_cancel(&mut self, _cancel: Option<Arc<AtomicBool>>) {}

    /// Yield the next batch; `Ok(None)` ends the stream. With a registry block sizer present,
    /// `block_count`/`block_offsets` are filled by the pipeline feeder (`Registry::peek_block_size`).
    fn next(&mut self) -> Ret<Option<BlockBatch>>;
}

#[derive(Clone)]
pub struct BlockBatch {
    pub bytes: Arc<Vec<u8>>,
    pub remote_height: u64,
    /// Number of complete blocks in `bytes`, when the producer has already
    /// validated the batch. Zero means the feeder must discover boundaries.
    pub block_count: u32,
    /// Start offsets for validated blocks. An empty vector has the same
    /// compatibility meaning as `block_count == 0`.
    pub block_offsets: Arc<Vec<u32>>,
    /// Blocks decoded while validating the network payload; pipeline workers reuse these
    /// objects instead of decoding each frame a second time.
    pub decoded_blocks: Arc<Vec<BlockRef>>,
}

impl BlockBatch {
    pub fn raw(bytes: Arc<Vec<u8>>, remote_height: u64) -> Self {
        Self {
            bytes,
            remote_height,
            block_count: 0,
            block_offsets: Arc::new(Vec::new()),
            decoded_blocks: Arc::new(Vec::new()),
        }
    }
}

// =============================================================
// BlockStream / BlockSender — async download feeds sync pipeline
// =============================================================

/// Blocking queue consumed by `run_sync` / pipeline feeder.
pub struct BlockStream {
    queue: Arc<Mutex<VecDeque<BlockBatch>>>,
    cv: Arc<Condvar>,
    closed: Arc<Mutex<bool>>,
    cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl BlockStream {
    pub fn new() -> (Self, BlockSender) {
        Self::with_capacity(8)
    }

    pub fn with_capacity(cap: usize) -> (Self, BlockSender) {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let cv = Arc::new(Condvar::new());
        let closed = Arc::new(Mutex::new(false));
        let cancel = Arc::new(Mutex::new(None));
        let sender = BlockSender {
            queue: queue.clone(),
            cv: cv.clone(),
            closed: closed.clone(),
            cancel: cancel.clone(),
            cap: cap.max(1),
        };
        (
            Self {
                queue,
                cv,
                closed,
                cancel,
            },
            sender,
        )
    }
}

impl BlockSource for BlockStream {
    fn set_cancel(&mut self, cancel: Option<Arc<AtomicBool>>) {
        *self.cancel.lock().unwrap() = cancel;
        self.cv.notify_all();
    }

    fn next(&mut self) -> Ret<Option<BlockBatch>> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if self
                .cancel
                .lock()
                .ok()
                .and_then(|f| f.clone())
                .is_some_and(|f| f.load(Ordering::Acquire))
            {
                return Ok(None);
            }
            if let Some(item) = q.pop_front() {
                self.cv.notify_all();
                return Ok(Some(item));
            }
            if *self.closed.lock().unwrap() {
                return Ok(None);
            }
            let (next_q, _) = self.cv.wait_timeout(q, Duration::from_millis(100)).unwrap();
            q = next_q;
        }
    }
}

/// Producer side: P2P read loop pushes downloaded batches here.
#[derive(Clone)]
pub struct BlockSender {
    queue: Arc<Mutex<VecDeque<BlockBatch>>>,
    cv: Arc<Condvar>,
    closed: Arc<Mutex<bool>>,
    cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    cap: usize,
}

impl BlockSender {
    pub fn push(&self, bytes: Vec<u8>) -> Rerr {
        self.push_batch(bytes, 0)
    }

    /// Blocks while the queue is full (backpressure to download window).
    pub fn push_batch(&self, bytes: Vec<u8>, remote_height: u64) -> Rerr {
        self.push_block_batch(BlockBatch::raw(Arc::new(bytes), remote_height))
    }

    /// Push a validated network batch without discarding its block boundaries.
    pub fn push_block_batch(&self, batch: BlockBatch) -> Rerr {
        let mut q = self.queue.lock().unwrap();
        while q.len() >= self.cap && !*self.closed.lock().unwrap() && !self.is_cancelled() {
            let (next, _) = self.cv.wait_timeout(q, Duration::from_millis(100)).unwrap();
            q = next;
        }
        if *self.closed.lock().unwrap() || self.is_cancelled() {
            return sys::errf!("p2p block stream is closed");
        }
        q.push_back(batch);
        self.cv.notify_all();
        Ok(())
    }

    /// Non-blocking push; returns `Ok(false)` if the queue is full.
    pub fn try_push_batch(&self, bytes: Vec<u8>, remote_height: u64) -> Ret<bool> {
        self.try_push_block_batch(BlockBatch::raw(Arc::new(bytes), remote_height))
    }

    /// Non-blocking variant of [`Self::push_block_batch`].
    pub fn try_push_block_batch(&self, batch: BlockBatch) -> Ret<bool> {
        let mut q = self.queue.lock().unwrap();
        if *self.closed.lock().unwrap() || self.is_cancelled() {
            return sys::errf!("p2p block stream is closed");
        }
        if q.len() >= self.cap {
            return Ok(false);
        }
        q.push_back(batch);
        self.cv.notify_all();
        Ok(true)
    }

    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn is_full(&self) -> bool {
        self.len() >= self.cap
    }

    pub fn finish(&self) {
        *self.closed.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .lock()
            .ok()
            .and_then(|f| f.clone())
            .is_some_and(|f| f.load(Ordering::Acquire))
    }
}

/// Compatibility aliases for callers that still use the old transport-named
/// queue types. The queue itself is protocol-agnostic.
pub type P2pStream = BlockStream;
pub type P2pSender = BlockSender;

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn cancellation_discards_already_queued_batches() {
        let (mut stream, sender) = BlockStream::with_capacity(1);
        let cancel = Arc::new(AtomicBool::new(false));
        stream.set_cancel(Some(cancel.clone()));
        sender.push(vec![1, 2, 3]).unwrap();

        cancel.store(true, Ordering::Release);

        assert!(stream.next().unwrap().is_none());
    }

    #[test]
    fn normal_finish_drains_queued_batches() {
        let (mut stream, sender) = BlockStream::with_capacity(1);
        sender.push(vec![1, 2, 3]).unwrap();
        sender.finish();

        assert_eq!(stream.next().unwrap().unwrap().bytes.as_slice(), &[1, 2, 3]);
        assert!(stream.next().unwrap().is_none());
    }

    #[test]
    fn cancellation_releases_a_blocked_sender() {
        let (mut stream, sender) = BlockStream::with_capacity(1);
        let cancel = Arc::new(AtomicBool::new(false));
        stream.set_cancel(Some(cancel.clone()));
        sender.push(vec![1]).unwrap();
        let blocked = sender.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            done_tx.send(blocked.push(vec![2])).unwrap();
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());

        cancel.store(true, Ordering::Release);
        sender.finish();

        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        assert!(stream.next().unwrap().is_none());
        thread.join().unwrap();
    }
}
