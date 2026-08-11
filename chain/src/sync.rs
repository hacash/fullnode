//! Bulk sync: parallel decode, ordered execute/tree, ordered persistence.
//!
//! The execute stage may lead persistence by a bounded number of blocks. Tree
//! root planning therefore uses a scheduled cursor while the real root remains
//! the last durable one until the persistence stage commits it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;

use base::{
    ApplyMode, BlkPkg, BlockBatch, BlockRef, BlockSource, PipelineOptions, PipelineReport,
    PkgOrigin, PkgSource,
};
use sys::{Rerr, Ret};

use crate::engine::{ChainEngine, PersistJob, PreparedBlock};
use crate::ring::Ring;

const SYNC_CANCELLED: &str = "sync_cancelled";
/// Admission deferred the block; the stream stopped intentionally and the
/// held-block report is the result, never an error or a penalty.
const SYNC_DEFERRED: &str = "sync_deferred";

fn cancelled_error() -> sys::Error {
    sys::Error::fault("block sync cancelled").with_code(SYNC_CANCELLED)
}

fn deferred_error() -> sys::Error {
    sys::Error::fault("block sync deferred by admission").with_code(SYNC_DEFERRED)
}

fn report_progress(opts: &PipelineOptions, report: &PipelineReport) {
    if let Some(progress) = &opts.progress {
        *progress.lock().unwrap() = report.clone();
    }
    if let Some(sink) = &opts.progress_sink {
        sink.on_pipeline_progress(report);
    }
}

/// Reuse boundaries supplied by a producer that already decoded and validated
/// the whole batch. Calling `peek_block_size` here would decode every block a
/// second time when the registry has no dedicated block sizer.
fn validated_frames(batch: &BlockBatch) -> Ret<Option<Vec<(usize, usize)>>> {
    if batch.block_count == 0 || batch.block_offsets.is_empty() {
        return Ok(None);
    }
    let count = batch.block_count as usize;
    if batch.block_offsets.len() != count {
        return sys::errf!(
            "validated block batch declares {} blocks but has {} offsets",
            count,
            batch.block_offsets.len()
        );
    }
    if !batch.decoded_blocks.is_empty() && batch.decoded_blocks.len() != count {
        return sys::errf!(
            "validated block batch declares {} blocks but has {} decoded blocks",
            count,
            batch.decoded_blocks.len()
        );
    }

    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let off = batch.block_offsets[index] as usize;
        let end = batch
            .block_offsets
            .get(index + 1)
            .map_or(batch.bytes.len(), |next| *next as usize);
        if (index == 0 && off != 0) || end <= off || end > batch.bytes.len() {
            return sys::errf!(
                "validated block batch has invalid frame {} range {}..{} for {} bytes",
                index,
                off,
                end,
                batch.bytes.len()
            );
        }
        frames.push((off, end - off));
    }
    Ok(Some(frames))
}

#[derive(Clone, Copy)]
enum PipelinePurpose {
    Network,
    Replay { from: u64, to: u64 },
}

impl PipelinePurpose {
    fn is_network(self) -> bool {
        matches!(self, Self::Network)
    }

    fn is_replay(self) -> bool {
        matches!(self, Self::Replay { .. })
    }

    fn persist_body(self) -> bool {
        matches!(self, Self::Network)
    }
}

fn check_replay_height(expected: Option<u64>, height: u64) -> Rerr {
    if let Some(expected) = expected
        && height != expected
    {
        return sys::errf!(
            "replay expected block height {} but decoded {}",
            expected,
            height
        );
    }
    Ok(())
}

fn check_replay_complete(from: u64, to: u64, report: &PipelineReport) -> Rerr {
    let expected = to.saturating_sub(from).saturating_add(1);
    if report.accepted != expected || report.final_height != to {
        return sys::errf!(
            "replay incomplete: expected {} blocks through {}, accepted {} through {}",
            expected,
            to,
            report.accepted,
            report.final_height
        );
    }
    Ok(())
}

fn prepare_cancel(source: &mut dyn BlockSource, opts: &mut PipelineOptions) -> Arc<AtomicBool> {
    let cancel = opts
        .cancel
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    opts.cancel = Some(cancel.clone());
    source.set_cancel(Some(cancel.clone()));
    cancel
}

/// Apply a stream of blocks. Holds `inserting` for the whole run so discover
/// and other syncs stay out. Fast-sync root persistence may overlap its next
/// linear execution; strict execution and root movement remain exclusive.
pub fn run(
    eng: &ChainEngine,
    mut source: Box<dyn BlockSource>,
    mode: ApplyMode,
    mut opts: PipelineOptions,
) -> Ret<PipelineReport> {
    let cancel = prepare_cancel(source.as_mut(), &mut opts);
    let _cancel_registration = eng.register_sync_cancel(cancel.clone());

    let Some(_hold) = eng.waiter.try_hold() else {
        cancel.store(true, Ordering::Release);
        return Err(cancelled_error());
    };
    let _guard = eng.inserting.lock().unwrap();
    if eng.waiter.is_shutdown() || cancel.load(Ordering::Acquire) {
        cancel.store(true, Ordering::Release);
        return Err(cancelled_error());
    }
    eng.syncing.store(true, Ordering::Release);
    let _reset = SyncFlag(eng);

    match run_pipeline(eng, source, mode, opts, cancel, PipelinePurpose::Network) {
        Ok(report) => Ok(report),
        // A persistence failure after blocks were published to the tree is
        // engine-fatal: no recovery path exists, boot replay rebuilds from
        // the real disk state on the next start (§2.3).
        Err(e)
            if matches!(
                e.code(),
                Some(code)
                    if code == crate::engine::PERSIST_FAILED
                        || code == crate::engine::CORE_FAILED
                        || code == crate::engine::STORAGE_READ_FAILED
            ) =>
        {
            eprintln!("[Block Sync Fatal] operation=sync error={}", e);
            eng.mark_fatal();
            Err(e)
        }
        Err(e) if e.code() == Some(SYNC_CANCELLED) || eng.waiter.is_shutdown() => Err(e),
        // Source gaps, invalid blocks and admission stops return to the
        // caller. The failing block is never attached, and blocks accepted
        // earlier in the run stay valid: the tree only ever holds blocks
        // that were also persisted.
        Err(e) => {
            eprintln!("[Block Sync Warning] {}", e);
            Err(e)
        }
    }
}

/// Replay already-stored canonical blocks through the sync pipeline. The
/// caller owns `inserting`; errors are returned to that owner for recovery.
pub(crate) fn run_replay_locked(
    eng: &ChainEngine,
    mut source: Box<dyn BlockSource>,
    from: u64,
    to: u64,
    mut opts: PipelineOptions,
) -> Ret<PipelineReport> {
    let cancel = prepare_cancel(source.as_mut(), &mut opts);
    run_pipeline(
        eng,
        source,
        ApplyMode::FastSync,
        opts,
        cancel,
        PipelinePurpose::Replay { from, to },
    )
}

/// One block frame handed from the feeder to a decoder.
struct Job {
    seq: u64,
    blob: Arc<Vec<u8>>,
    offset: usize,
    len: usize,
    decoded: Option<BlockRef>,
}

/// Replay reads from the local store: the height index hash must match the
/// decoded body, or the node would replay a different branch than its own
/// index records. A read failure is a replay corruption, not a missing block.
fn check_replay_index(eng: &ChainEngine, pkg: &BlkPkg) -> Rerr {
    let indexed = eng
        .store
        .block_store()
        .hash_by_height(pkg.height())
        .map_err(|e| e.with_code(crate::engine::STORAGE_READ_FAILED))?;
    if indexed != Some(pkg.hash()) {
        return sys::errf!(
            "replay height index hash for {} does not match the decoded block",
            pkg.height()
        );
    }
    Ok(())
}

/// Strict network mode: a block may only be admitted after its parent is
/// known to be in the tree, so an orphaned block never touches bidding/arrival
/// state (§6). Fast sync is linear: the parent is always present.
fn require_parent_in_tree(eng: &ChainEngine, pkg: &BlkPkg) -> Rerr {
    let prev = pkg.block().prev_hash();
    if !eng.tree.contains(&prev) {
        return sys::errf!(
            "network block {} is missing parent {:?}",
            pkg.height(),
            prev
        );
    }
    Ok(())
}

/// Validate, execute and route one block: replay checks, wire checks,
/// consensus checks, then prepare. Returns the persistence job, or None when
/// the block is skipped (duplicate or discarded side branch in network mode).
/// Failure metadata is recorded by the caller (`fail_with`).
fn process_block(
    ctx: &SyncCtx,
    report: &mut PipelineReport,
    pkg: &BlkPkg,
    replay_next: &mut Option<u64>,
) -> Ret<Option<PersistJob>> {
    let height = pkg.height();

    // Replay must decode exactly the expected next height.
    check_replay_height(*replay_next, height)?;
    if ctx.purpose.is_replay() {
        check_replay_index(ctx.eng, pkg)?;
    }

    // Wire-level checks (fast-sync blocks come from a trusted source).
    if !ctx.pipelined {
        crate::verify::check_intrinsic(ctx.eng, pkg)?;
    }

    // Consensus admission checks for network blocks; replay skips them.
    if ctx.purpose.is_network() {
        if !ctx.pipelined {
            require_parent_in_tree(ctx.eng, pkg)?;
        }
        // Same order as discover: arrive validation runs before admission, so
        // a block failing the arrive check never touches bidding state (§6).
        crate::engine::catch_storage_panic(|| {
            ctx.eng
                .consensus
                .check_block_arrive(pkg, ctx.eng, ctx.pipelined)
        })?;
        match crate::engine::catch_storage_panic(|| {
            ctx.eng
                .consensus
                .check_block_admission(pkg, ctx.eng, ctx.pipelined)
        })? {
            base::BlockAdmissionDecision::Continue => {}
            base::BlockAdmissionDecision::Defer(_) => {
                // Explicit deferred stop: the source pauses without judging
                // the block invalid; the report carries the held blocks.
                report.held_blocks.push((height, pkg.hash()));
                report_progress(ctx.opts, report);
                return Err(deferred_error());
            }
        }
    }

    // Execute and attach; the returned job is persisted in insertion order.
    match ctx
        .eng
        .prepare_one(pkg, ctx.mode, ctx.purpose.persist_body())?
    {
        PreparedBlock::Accepted(job) => Ok(Some(job)),
        // Network mode: a duplicate or discarded live side branch never stops
        // the stream.
        PreparedBlock::Duplicate(_) if ctx.purpose.is_network() => Ok(None),
        PreparedBlock::Discarded if ctx.purpose.is_network() => Ok(None),
        // Everything else is an error: replay is strictly linear and treats
        // any deviation as corruption; a network orphan ends the stream.
        PreparedBlock::Orphan(parent) if ctx.purpose.is_replay() => {
            sys::errf!("replay block {} is missing parent {:?}", height, parent)
        }
        PreparedBlock::Orphan(parent) => {
            sys::errf!("network block {} is missing parent {:?}", height, parent)
        }
        PreparedBlock::Duplicate(_) => sys::errf!(
            "replay block <{}, {:?}> is already present",
            height,
            pkg.hash()
        ),
        PreparedBlock::Discarded => {
            sys::errf!("replay block {} was discarded as a side branch", height)
        }
    }
}

/// Shared pipeline state: engine references, queues and the cancellation
/// flag, used by every stage instead of threading parameters around.
///
/// The job receiver deliberately lives outside this struct: it must be owned
/// only by the decoder workers, so that when they all exit on a hard pipeline
/// stop the jobs channel disconnects and releases a feeder blocked in
/// `send()` (see `run_pipeline`).
struct SyncCtx<'a> {
    eng: &'a ChainEngine,
    opts: &'a PipelineOptions,
    mode: ApplyMode,
    purpose: PipelinePurpose,
    cancel: Arc<AtomicBool>,
    origin: PkgOrigin,
    ring: Arc<Ring>,
    pipelined: bool,
}

impl SyncCtx<'_> {
    /// Failure broadcast: set the cancel flag and stop the ring so every
    /// thread waiting on it wakes up. The two must always happen together.
    fn abort(&self) {
        self.cancel.store(true, Ordering::Release);
        self.ring.stop();
    }
}

/// Feeder stage: split source batches into per-block jobs.
///
/// `jobs_tx` is dropped when this returns, which releases any decoder blocked
/// in `recv`. Every exit path must `close(seq)` first, or the apply stage
/// would wait forever on the missing sequence.
fn feed_batches(ctx: &SyncCtx, mut source: Box<dyn BlockSource>, jobs_tx: SyncSender<Job>) -> Rerr {
    let mut seq = 0u64;
    'outer: loop {
        // Cancellation is graceful at the batch boundary. Once a batch has
        // been taken from the source, publish all of its blocks so execute
        // and persistence can finish it.
        if ctx.cancel.load(Ordering::Acquire) {
            break;
        }
        let batch = match source.next() {
            Ok(Some(batch)) => batch,
            Ok(None) => break,
            Err(e) => {
                ctx.ring.close(seq);
                return Err(e);
            }
        };
        let blob = batch.bytes.clone();
        let frames = match validated_frames(&batch) {
            Ok(Some(frames)) => frames,
            Ok(None) => match peek_frames(ctx.eng.registry.as_ref(), &blob) {
                Ok(frames) => frames,
                Err(e) => {
                    ctx.ring.close(seq);
                    return Err(e);
                }
            },
            Err(e) => {
                ctx.ring.close(seq);
                return Err(e);
            }
        };
        for (index, (off, len)) in frames.into_iter().enumerate() {
            let job = Job {
                seq,
                blob: blob.clone(),
                offset: off,
                len,
                decoded: batch.decoded_blocks.get(index).cloned(),
            };
            if jobs_tx.send(job).is_err() {
                break 'outer;
            }
            seq += 1;
        }
    }
    ctx.ring.close(seq);
    Ok(())
}

/// Split a blob into block frames when the producer did not pre-split it,
/// using the registry's block sizer instead.
fn peek_frames(registry: &dyn base::BinaryCodecs, blob: &[u8]) -> Ret<Vec<(usize, usize)>> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off < blob.len() {
        let used = match registry.peek_block_size(&blob[off..]) {
            Ok(used) => used,
            Err(e) => return Err(e),
        };
        if used == 0 || off + used > blob.len() {
            return sys::errf!("incomplete block frame at offset {}", off);
        }
        frames.push((off, used));
        off += used;
    }
    Ok(frames)
}

/// Decoder stage: reserve a ring slot before taking a job. If sequence N is a
/// slow decode, later workers can fill the ring but can never occupy the slot
/// N still needs.
fn decode_loop(ctx: &SyncCtx, jobs_rx: Arc<Mutex<Receiver<Job>>>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loop {
            if !ctx.ring.reserve() {
                break;
            }
            let job = jobs_rx.lock().unwrap().recv();
            let Ok(job) = job else {
                ctx.ring.release();
                break;
            };
            let slot = match job.decoded {
                Some(blk) => BlkPkg::from_shared_decoded(
                    job.blob,
                    job.offset,
                    job.len,
                    blk,
                    PkgSource::new(ctx.origin),
                ),
                None => BlkPkg::from_shared(
                    ctx.eng.registry.as_ref(),
                    job.blob,
                    job.offset,
                    job.len,
                    PkgSource::new(ctx.origin),
                ),
            };
            if !ctx.ring.publish_reserved(job.seq, slot) {
                break;
            }
        }
    }));
    if result.is_err() {
        // The panicked thread's slot would never be published: wake every
        // stage first, then re-raise so the join side reports the failure
        // instead of the pipeline hanging on the missing sequence.
        ctx.abort();
        panic!("sync decoder panicked");
    }
}

/// Persistence stage: write executed jobs to disk in insertion order. On
/// failure the real disk/root error must reach the caller, so wake every
/// stage instead of letting execute wait on more network input.
fn persist_loop(ctx: &SyncCtx, persist_rx: Receiver<PersistJob>) -> Ret<(u64, u64)> {
    let result = (|| {
        let mut rolled = 0;
        let mut events = 0;
        while let Ok(job) = persist_rx.recv() {
            let outcome = ctx.eng.persist_one(job, false)?;
            rolled += outcome.rolled;
            events += outcome.events;
        }
        Ok((rolled, events))
    })();
    if result.is_err() {
        ctx.abort();
    }
    result
}

/// Record a failure and return the same error, for use in `?` chains. A
/// deferred stop (SYNC_DEFERRED) is not a failure: it records nothing, so
/// the report stays clean for the caller and any progress sink.
fn fail_with(report: &mut PipelineReport, height: u64, e: sys::Error) -> sys::Error {
    if e.code() != Some(SYNC_DEFERRED) {
        report.failure_height = Some(height);
        report.failure_message = Some(e.to_string());
    }
    e
}

/// Apply stage: consume decoded blocks in order. Fast-sync jobs are handed to
/// the bounded persister; strict mode persists inline so a reorg can never be
/// planned on top of outstanding root jobs.
fn apply_loop(
    ctx: &SyncCtx,
    persist_tx: &SyncSender<PersistJob>,
    report: &mut PipelineReport,
    replay_next: &mut Option<u64>,
) -> Rerr {
    let mut seq = 0u64;
    loop {
        let Some(slot) = ctx.ring.take(seq) else {
            break;
        };
        seq += 1;
        let pkg = match slot {
            Ok(pkg) => pkg,
            // A decode failure has no block height, so only the message is
            // recorded here; process failures use `fail_with` below.
            Err(e) => {
                report.failure_message = Some(e.to_string());
                return Err(e);
            }
        };
        let height = pkg.height();
        let Some(job) = process_block(ctx, report, &pkg, replay_next)
            .map_err(|e| fail_with(report, height, e))?
        else {
            continue; // duplicate or discarded side branch: keep streaming
        };
        debug_assert!(!ctx.pipelined || (job.inserted.is_head && !job.inserted.reorg));
        let confirmed_txs = job.inserted.confirmed_txs.clone();
        let reverted_txs = job.inserted.reverted_txs.clone();
        if ctx.pipelined {
            persist_tx.send(job).map_err(|_| {
                fail_with(
                    report,
                    height,
                    sys::Error::fault("ordered persistence stage stopped"),
                )
            })?;
        } else {
            let outcome = ctx
                .eng
                .persist_one(job, false)
                .map_err(|e| fail_with(report, height, e))?;
            report.rolled += outcome.rolled;
            report.events += outcome.events;
        }
        // Counted once attached to the tree; persistence may lag behind by up
        // to the persist queue depth, so `accepted` is not the durable height.
        report.accepted += 1;
        report.final_height = ctx.eng.tree.head_height();
        if replay_next.is_some() {
            *replay_next = height.checked_add(1);
        }
        if !confirmed_txs.is_empty() {
            report.confirmed_txs.push((height, confirmed_txs));
        }
        if !reverted_txs.is_empty() {
            report.reverted_txs.push((height, reverted_txs));
        }
        if ctx.purpose.is_replay() || report.accepted.is_multiple_of(200) {
            report_progress(ctx.opts, report);
        }
    }
    Ok(())
}

/// Finalize a pipeline run: cancellation detection, replay completeness,
/// cache coordination after any durable commit, and the deferred/error split.
///
/// `persist_result` is the persister stage's own result: it drains the whole
/// queue even when the apply stage stopped early, so its totals belong to the
/// report in every non-fatal outcome. `pipeline_result` is the apply/feeder/
/// decoder outcome.
fn finalize_run(
    eng: &ChainEngine,
    opts: &PipelineOptions,
    purpose: PipelinePurpose,
    cancel: &AtomicBool,
    mut report: PipelineReport,
    persist_result: Ret<(u64, u64)>,
    pipeline_result: Rerr,
) -> Ret<PipelineReport> {
    // A persistence failure is engine-fatal: the caller marks the engine dead
    // and boot replay rebuilds caches from the real disk state (§2.3).
    let (rolled, events) = match persist_result {
        Ok(summary) => summary,
        Err(e) => return Err(e),
    };
    report.rolled += rolled;
    report.events += events;

    // External cancellation: no deferred blocks were held, the run was cut
    // short by the caller.
    if pipeline_result.is_ok() && cancel.load(Ordering::Acquire) {
        if !eng.waiter.is_shutdown() {
            eng.rebuild_runtime_caches()?;
        }
        report_progress(opts, &report);
        return Err(cancelled_error());
    }

    // A successful run must cover the full requested replay range.
    if pipeline_result.is_ok()
        && let PipelinePurpose::Replay { from, to } = purpose
    {
        check_replay_complete(from, to, &report)?;
    }

    // Every durable commit needs a final cache coordination no matter how the
    // run stopped: recent-block and fee caches are only rebuilt here, so a
    // partial run (defer, decode or validation failure, source gap) would
    // otherwise leave them pointing at pre-sync state. A rebuild failure
    // after real commits leaves them stale, so it becomes the run's error
    // instead of being masked under the original one; only a zero-commit run
    // keeps the original error (the rebuild then re-reads unchanged data).
    if let Err(e) = eng.rebuild_runtime_caches() {
        if pipeline_result.is_ok() || report.accepted > 0 {
            return Err(e);
        }
        eprintln!("[Block Sync Warning] cache rebuild failed: {}", e);
    }
    report_progress(opts, &report);
    match pipeline_result {
        Ok(()) => Ok(report),
        // An admission deferral is a successful stop: the source is paused
        // and the held-block report tells the caller what to retry later.
        Err(e) if e.code() == Some(SYNC_DEFERRED) => Ok(report),
        Err(e) => {
            if report.failure_message.is_none() {
                report.failure_message = Some(e.to_string());
            }
            Err(e)
        }
    }
}

/// Run one sync pipeline to completion: feed, parallel decode, ordered apply
/// and ordered persistence. Errors from any stage are resolved with the
/// persister's disk/root error taking priority; see `finalize_run` for the
/// success/cancel/deferred outcome mapping.
fn run_pipeline(
    eng: &ChainEngine,
    source: Box<dyn BlockSource>,
    mode: ApplyMode,
    opts: PipelineOptions,
    cancel: Arc<AtomicBool>,
    purpose: PipelinePurpose,
) -> Ret<PipelineReport> {
    let workers = opts.decode_workers.max(1);
    let queue = opts.decode_queue.max(workers + 1);
    let persist_queue = (eng.config.unstable_block as usize * 2).clamp(8, 16);

    let mut report = PipelineReport {
        final_height: eng.tree.head_height(),
        ..Default::default()
    };
    let mut replay_next = match purpose {
        PipelinePurpose::Network => None,
        PipelinePurpose::Replay { from, .. } => Some(from),
    };
    report_progress(&opts, &report);

    let ring = Arc::new(Ring::new(queue));
    let (jobs_tx, jobs_rx) = sync_channel::<Job>(queue);
    let jobs_rx = Arc::new(Mutex::new(jobs_rx));
    let ctx = SyncCtx {
        eng,
        opts: &opts,
        mode,
        purpose,
        cancel,
        origin: opts.origin,
        ring,
        pipelined: mode.is_fast_sync(),
    };
    let ctx_ref = &ctx;

    // Feed -> decode -> apply -> persist. Every stage blocks on the ring or a
    // channel; any failure is broadcast through SyncCtx::abort. Returns the
    // persister outcome separately from the run outcome: the persister drains
    // the whole queue even when the apply stage stops early, so its totals
    // reach the report in every non-fatal outcome.
    let (persist_result, pipeline_result) = thread::scope(|s| -> (Ret<(u64, u64)>, Rerr) {
        let (persist_tx, persist_rx) = sync_channel(persist_queue);
        let feeder = s.spawn(move || feed_batches(ctx_ref, source, jobs_tx));
        let decoders: Vec<_> = (0..workers)
            .map(|_| {
                let rx = jobs_rx.clone();
                s.spawn(move || decode_loop(ctx_ref, rx))
            })
            .collect();
        // Only decoder workers may keep the receive side alive: when they all
        // exit on a hard pipeline stop, the channel disconnects and releases
        // a feeder blocked in send().
        drop(jobs_rx);
        let persister = s.spawn(move || persist_loop(ctx_ref, persist_rx));

        let applied = apply_loop(ctx_ref, &persist_tx, &mut report, &mut replay_next);
        if applied.is_err() {
            ctx.abort();
        }
        // Close the persistence channel; the persister drains and exits.
        drop(persist_tx);

        // A closed persistence channel is only a symptom; preserve the actual
        // disk/root error returned by the persister when one exists.
        let persist_result = match persister.join() {
            Ok(Ok(summary)) => Ok(summary),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(sys::Error::fault("sync persister panicked")
                .with_code(crate::engine::PERSIST_FAILED)),
        };
        // The run outcome: the apply error, or the first error among the
        // feeder and decoder joins. Every handle is joined explicitly so a
        // panicking worker surfaces as an error here instead of panicking
        // the scope and skipping finalize_run.
        let feeder_result = match feeder.join() {
            Ok(result) => result,
            Err(_) => Err(sys::Error::fault("sync feeder panicked")),
        };
        let mut decoder_result: Rerr = Ok(());
        for decoder in decoders {
            if decoder.join().is_err() {
                decoder_result = Err(sys::Error::fault("sync decoder panicked"));
            }
        }
        (
            persist_result,
            applied.and(feeder_result).and(decoder_result),
        )
    });

    finalize_run(
        eng,
        &opts,
        purpose,
        &ctx.cancel,
        report,
        persist_result,
        pipeline_result,
    )
}

/// Clears the syncing flag however the run ends.
struct SyncFlag<'a>(&'a ChainEngine);

impl Drop for SyncFlag<'_> {
    fn drop(&mut self) {
        self.0.syncing.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_batch(bytes: Vec<u8>, count: u32, offsets: Vec<u32>) -> BlockBatch {
        BlockBatch {
            bytes: Arc::new(bytes),
            remote_height: 0,
            block_count: count,
            block_offsets: Arc::new(offsets),
            decoded_blocks: Arc::new(Vec::new()),
        }
    }

    #[test]
    fn validated_batch_offsets_bypass_frame_discovery() {
        let batch = raw_batch(vec![0; 9], 3, vec![0, 2, 7]);
        assert_eq!(
            validated_frames(&batch).unwrap(),
            Some(vec![(0, 2), (2, 5), (7, 2)])
        );
    }

    #[test]
    fn validated_batch_offsets_reject_gaps_and_bad_counts() {
        assert!(validated_frames(&raw_batch(vec![0; 4], 2, vec![1, 2])).is_err());
        assert!(validated_frames(&raw_batch(vec![0; 4], 2, vec![0])).is_err());
        assert!(validated_frames(&raw_batch(vec![0; 4], 2, vec![0, 4])).is_err());
    }

    #[test]
    fn replay_rejects_a_height_gap() {
        assert!(check_replay_height(Some(8), 8).is_ok());
        assert!(check_replay_height(Some(8), 9).is_err());
    }

    #[test]
    fn replay_requires_the_complete_requested_range() {
        let complete = PipelineReport {
            accepted: 3,
            final_height: 12,
            ..Default::default()
        };
        assert!(check_replay_complete(10, 12, &complete).is_ok());

        let partial = PipelineReport {
            accepted: 2,
            final_height: 11,
            ..Default::default()
        };
        assert!(check_replay_complete(10, 12, &partial).is_err());
    }
}
