//! Bulk sync: parallel decode, ordered execute/tree, ordered persistence.
//!
//! The execute stage may lead persistence by a bounded number of blocks. Tree
//! root planning therefore uses a scheduled cursor while the real root remains
//! the last durable one until the persistence stage commits it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;

use base::{
    ApplyMode, BlkPkg, BlockBatch, BlockSource, PipelineOptions, PipelineReport, PkgSource,
};
use sys::{Rerr, Ret};

use crate::engine::{ChainEngine, PreparedBlock};
use crate::ring::Ring;

const SYNC_CANCELLED: &str = "sync_cancelled";

fn cancelled_error() -> sys::Error {
    sys::Error::fault("block sync cancelled").with_code(SYNC_CANCELLED)
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
        Err(e) if e.code() == Some(SYNC_CANCELLED) || eng.waiter.is_shutdown() => Err(e),
        Err(e) => {
            eprintln!("[Block Sync Warning] {}", e);
            if let Err(re) = crate::boot::recover(eng) {
                return sys::errf!("sync failed: {}; recovery failed: {}", e, re);
            }
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

fn run_pipeline(
    eng: &ChainEngine,
    mut source: Box<dyn BlockSource>,
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
    // (seq, blob, offset, len, remote_height, pre-decoded block if any)
    type Job = (u64, Arc<Vec<u8>>, usize, usize, u64, Option<base::BlockRef>);
    let (jobs_tx, jobs_rx) = sync_channel::<Job>(queue);
    let jobs_rx = Arc::new(Mutex::new(jobs_rx));
    let origin = opts.origin;
    let pipelined = mode.is_fast_sync();

    let result = thread::scope(|s| -> Ret<(u64, u64)> {
        // Feeder: split incoming batches into per-block jobs.
        let feed_ring = ring.clone();
        let feed_registry = eng.registry.clone();
        let feed_cancel = cancel.clone();
        let feeder = s.spawn(move || -> Rerr {
            let mut seq = 0u64;
            'outer: loop {
                // Cancellation is graceful at the batch boundary. Once a
                // batch has been taken from the source, publish all of its
                // blocks so execute and persistence can finish it.
                if feed_cancel.load(Ordering::Acquire) {
                    break;
                }
                let batch = match source.next() {
                    Ok(Some(batch)) => batch,
                    Ok(None) => break,
                    Err(e) => {
                        feed_ring.close(seq);
                        drop(jobs_tx);
                        return Err(e);
                    }
                };
                let blob = batch.bytes.clone();
                let frames = match validated_frames(&batch) {
                    Ok(frames) => frames,
                    Err(e) => {
                        feed_ring.close(seq);
                        return Err(e);
                    }
                };
                if let Some(frames) = frames {
                    for (index, (off, len)) in frames.into_iter().enumerate() {
                        let decoded = batch.decoded_blocks.get(index).cloned();
                        if jobs_tx
                            .send((seq, blob.clone(), off, len, batch.remote_height, decoded))
                            .is_err()
                        {
                            break 'outer;
                        }
                        seq += 1;
                    }
                    continue;
                }
                let mut off = 0usize;
                let mut index = 0usize;
                while off < blob.len() {
                    let used = match feed_registry.peek_block_size(&blob[off..]) {
                        Ok(used) => used,
                        Err(e) => {
                            feed_ring.close(seq);
                            drop(jobs_tx);
                            return Err(e);
                        }
                    };
                    if used == 0 || off + used > blob.len() {
                        feed_ring.close(seq);
                        drop(jobs_tx);
                        return sys::errf!("incomplete block frame at offset {}", off);
                    }
                    let decoded = batch.decoded_blocks.get(index).cloned();
                    if jobs_tx
                        .send((seq, blob.clone(), off, used, batch.remote_height, decoded))
                        .is_err()
                    {
                        break 'outer;
                    }
                    off += used;
                    index += 1;
                    seq += 1;
                }
            }
            feed_ring.close(seq);
            drop(jobs_tx);
            Ok(())
        });

        // Decoders.
        for _ in 0..workers {
            let rx = jobs_rx.clone();
            let ring = ring.clone();
            let registry = eng.registry.clone();
            s.spawn(move || {
                loop {
                    // Reserve capacity before taking a job. If sequence N is a
                    // slow decode, later workers can fill the ring but can
                    // never occupy the slot N still needs.
                    if !ring.reserve() {
                        break;
                    }
                    let job = { rx.lock().unwrap().recv() };
                    let Ok((seq, blob, off, len, _remote, decoded)) = job else {
                        ring.release();
                        break;
                    };
                    let slot = match decoded {
                        Some(blk) => {
                            BlkPkg::from_shared_decoded(blob, off, len, blk, PkgSource::new(origin))
                        }
                        None => BlkPkg::from_shared(
                            registry.as_ref(),
                            blob,
                            off,
                            len,
                            PkgSource::new(origin),
                        ),
                    };
                    if !ring.publish_reserved(seq, slot) {
                        break;
                    }
                }
            });
        }
        // Only decoder workers may keep the receive side alive. On a hard
        // pipeline stop they all exit, disconnecting the channel and releasing
        // a feeder blocked in send().
        drop(jobs_rx);

        let (persist_tx, persist_rx) = sync_channel(persist_queue);
        let persist_ring = ring.clone();
        let persist_cancel = cancel.clone();
        let persister = s.spawn(move || -> Ret<(u64, u64)> {
            let result = (|| {
                let mut rolled = 0;
                let mut events = 0;
                while let Ok(job) = persist_rx.recv() {
                    let outcome = eng.persist_one(job, false)?;
                    rolled += outcome.rolled;
                    events += outcome.events;
                }
                Ok((rolled, events))
            })();
            if result.is_err() {
                // Persistence can fail while execute is waiting for more
                // network input. Wake every stage so the real disk/root error
                // can be returned and recovery can reset pending tree state.
                persist_cancel.store(true, Ordering::Release);
                persist_ring.stop();
            }
            result
        });

        // Execute/tree: consume decoded blocks in order. Fast-sync jobs are
        // handed to the bounded persister; strict mode persists inline so a
        // reorg can never be planned on top of outstanding root jobs.
        let apply_result = (|| -> Rerr {
            let mut seq = 0u64;
            loop {
                let Some(slot) = ring.take(seq) else { break };
                seq += 1;
                let pkg = match slot {
                    Ok(pkg) => pkg,
                    Err(e) => {
                        report.failure_message = Some(e.to_string());
                        return Err(e);
                    }
                };
                let height = pkg.height();
                if let Err(e) = check_replay_height(replay_next, height) {
                    report.failure_height = Some(height);
                    report.failure_message = Some(e.to_string());
                    return Err(e);
                }
                if !pipelined {
                    crate::verify::check_intrinsic(eng, &pkg).inspect_err(|e| {
                        report.failure_height = Some(height);
                        report.failure_message = Some(e.to_string());
                    })?;
                }
                if matches!(purpose, PipelinePurpose::Network) {
                    let admission =
                        eng.consensus
                            .check_block_admission(&pkg, eng)
                            .inspect_err(|e| {
                                report.failure_height = Some(height);
                                report.failure_message = Some(e.to_string());
                            })?;
                    if matches!(admission, base::BlockAdmissionDecision::Defer(_)) {
                        report.held_blocks.push((height, pkg.hash()));
                        report_progress(&opts, &report);
                        // Wake a network source that may already be waiting for
                        // the next batch. This is an intentional successful
                        // stop, distinguished from external cancellation by
                        // the non-empty held report below.
                        cancel.store(true, Ordering::Release);
                        ring.stop();
                        break;
                    }
                    eng.consensus
                        .check_block_arrive(&pkg, eng)
                        .inspect_err(|e| {
                            report.failure_height = Some(height);
                            report.failure_message = Some(e.to_string());
                        })?;
                }
                let prepared = eng
                    .prepare_one(&pkg, mode, purpose.persist_body())
                    .map_err(|e| {
                        report.failure_height = Some(height);
                        report.failure_message = Some(e.to_string());
                        e
                    })?;
                let job = match prepared {
                    PreparedBlock::Accepted(job) => job,
                    PreparedBlock::Duplicate(hash) => match purpose {
                        PipelinePurpose::Network => {
                            report.final_height = eng.tree.head_height();
                            continue;
                        }
                        PipelinePurpose::Replay { .. } => {
                            let e = sys::Error::fault(format!(
                                "replay block <{}, {:?}> is already present",
                                height, hash
                            ));
                            report.failure_height = Some(height);
                            report.failure_message = Some(e.to_string());
                            return Err(e);
                        }
                    },
                    PreparedBlock::Orphan(parent) => match purpose {
                        PipelinePurpose::Network => {
                            let e = sys::Error::fault(format!(
                                "network block {} is missing parent {:?}",
                                height, parent
                            ));
                            report.failure_height = Some(height);
                            report.failure_message = Some(e.to_string());
                            return Err(e);
                        }
                        PipelinePurpose::Replay { .. } => {
                            let e = sys::Error::fault(format!(
                                "replay block {} is missing parent {:?}",
                                height, parent
                            ));
                            report.failure_height = Some(height);
                            report.failure_message = Some(e.to_string());
                            return Err(e);
                        }
                    },
                };
                debug_assert!(!pipelined || (job.is_head && !job.reorg));
                let confirmed_txs = job.confirmed_txs.clone();
                let reverted_txs = job.reverted_txs.clone();
                if pipelined {
                    persist_tx.send(job).map_err(|_| {
                        let e = sys::Error::fault("ordered persistence stage stopped");
                        report.failure_height = Some(height);
                        report.failure_message = Some(e.to_string());
                        e
                    })?;
                } else {
                    let outcome = eng.persist_one(job, false).map_err(|e| {
                        report.failure_height = Some(height);
                        report.failure_message = Some(e.to_string());
                        e
                    })?;
                    report.rolled += outcome.rolled;
                    report.events += outcome.events;
                }
                report.accepted += 1;
                report.final_height = eng.tree.head_height();
                if replay_next.is_some() {
                    replay_next = height.checked_add(1);
                }
                if !confirmed_txs.is_empty() {
                    report.confirmed_txs.push((height, confirmed_txs));
                }
                if !reverted_txs.is_empty() {
                    report.reverted_txs.push((height, reverted_txs));
                }
                if matches!(purpose, PipelinePurpose::Replay { .. })
                    || report.accepted.is_multiple_of(200)
                {
                    report_progress(&opts, &report);
                }
            }
            Ok(())
        })();

        if apply_result.is_err() {
            cancel.store(true, Ordering::Release);
            ring.stop();
        }

        drop(persist_tx);
        let feeder_result = feeder
            .join()
            .map_err(|_| sys::Error::fault("sync feeder panicked"))?;
        let persist_result = persister
            .join()
            .map_err(|_| sys::Error::fault("sync persister panicked"))?;

        // A closed persistence channel is only a symptom; preserve the actual
        // disk/root error returned by the persister when one exists.
        let persisted = match persist_result {
            Ok(summary) => summary,
            Err(e) => return Err(e),
        };
        apply_result?;
        feeder_result?;
        Ok(persisted)
    });

    let result = match result {
        Ok(_) if cancel.load(Ordering::Acquire) && report.held_blocks.is_empty() => {
            if !eng.waiter.is_shutdown() {
                eng.rebuild_runtime_caches()?;
            }
            Err(cancelled_error())
        }
        result => result,
    };
    let result = result.and_then(|(rolled, events)| {
        report.rolled += rolled;
        report.events += events;
        if let PipelinePurpose::Replay { from, to } = purpose {
            check_replay_complete(from, to, &report)?;
        }
        eng.rebuild_runtime_caches()
    });
    report_progress(&opts, &report);
    match result {
        Ok(()) => Ok(report),
        Err(e) => {
            if report.failure_message.is_none() {
                report.failure_message = Some(e.to_string());
            }
            Err(e)
        }
    }
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
