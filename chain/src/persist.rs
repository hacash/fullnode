//! Root advance: write the newly stable blocks to disk, then move the root.
//!
//! State writes and root markers go into one batch, so a crash either loses
//! the whole advance or keeps all of it.
//! That is what lets boot trust `root_height` and replay from it safely.

use base::{
    Engine, MemDB, PERSIST_KEY_ROOT_HASH, PERSIST_KEY_ROOT_HEIGHT, PkgOrigin,
    STATE_DECODE_FAILED_CODE, STATE_READ_FAILED_CODE,
};
use field::Hash;
use sys::{Rerr, Ret};

use crate::engine::{ChainEngine, persist_fatal};
use crate::tree::{RollJob, hash_of, height_of};

/// Persist a root advance and then move the tree root. The root-move writer
/// excludes critical execution and marks the disk/tree transition unstable to
/// optimistic validators.
pub fn roll_root(
    eng: &ChainEngine,
    job: RollJob,
    origin: PkgOrigin,
    stored_replay: bool,
) -> Ret<Vec<(u64, Hash)>> {
    {
        let mut root_move = eng.begin_root_move();
        // Reject a broken ordering before it can publish an incomplete root
        // batch. Commit validates again after the write. Only the disk/root
        // writes are tagged `persist_failed`; callback errors keep their own
        // classification (§8.3).
        eng.tree.validate_roll(&job).map_err(persist_fatal)?;
        eng.store
            .disk()
            .try_write(&RootBatch { job: &job })
            .map_err(persist_fatal)?;
        eng.tree.commit_roll(&job).map_err(persist_fatal)?;
        root_move.commit();
    }

    let mut stable = Vec::with_capacity(job.chain.len());
    for chunk in &job.chain {
        let height = height_of(chunk);
        let hash = hash_of(chunk);
        if eng.config.is_open_vmlog(height) {
            let logs = chunk.block_logs();
            if !logs.is_empty()
                && let Err(e) = eng.store.log_backend().append_block_logs(height, &logs)
            {
                eprintln!("[Engine] block {} log write failed: {}", height, e);
            }
        }
        stable.push((height, hash));
    }
    notify_stable(eng, &stable, origin, stored_replay)?;
    Ok(stable)
}

fn notify_stable(
    eng: &ChainEngine,
    stable: &[(u64, Hash)],
    origin: PkgOrigin,
    stored_replay: bool,
) -> Rerr {
    // Consensus tracks stable blocks to age out its bidding state. Replaying
    // stored blocks re-derives that from the state it already loaded, and the
    // callback would cost a disk read plus a full decode per block. Replay
    // also skips every external listener: restart must not re-publish
    // non-durable events (§8 of the error contract). The pending-cache forget
    // still runs so replayed blocks do not accumulate in memory.
    for (height, hash) in stable {
        if !stored_replay {
            // Stable-block notification is consensus-critical (§8.3): a body
            // read failure is `Abort + storage_read_failed`, a decode failure
            // `Abort + state_decode_failed`. A missing body is never skipped.
            let block = match eng.block_history.cached(*height, hash) {
                Some(block) => block,
                None => {
                    let data = eng
                        .store
                        .block_data(hash)
                        .map_err(|e| {
                            sys::Error::abort(format!(
                                "stable block {} body read failed: {}",
                                hash, e
                            ))
                            .with_code(STATE_READ_FAILED_CODE)
                        })?
                        .ok_or_else(|| {
                            sys::Error::abort(format!(
                                "stable block {} body is missing from the block db",
                                hash
                            ))
                            .with_code(STATE_READ_FAILED_CODE)
                        })?;
                    eng.registry
                        .decode_block(&data)
                        .map_err(|e| {
                            sys::Error::abort(format!(
                                "stable block {} body cannot be decoded: {}",
                                hash, e
                            ))
                            .with_code(STATE_DECODE_FAILED_CODE)
                        })?
                        .0
                }
            };
            // Listeners that query BlockHistory during this callback reuse
            // the same decoded object.
            eng.block_history.remember(block.clone());
            // A consensus callback error is engine-fatal: preserve an already
            // `Abort` error as-is, otherwise escalate to `Abort + core_failed`
            // (§8.3). The accepted root is never rolled back.
            if let Err(e) = eng.consensus.on_stable_block(block.as_ref(), eng) {
                if e.is_abort() {
                    return Err(e);
                }
                return Err(
                    sys::Error::abort(format!("consensus.on_stable_block failed: {}", e))
                        .with_code("core_failed"),
                );
            }
            // Listener registry mutex is released before any external callback
            // (§8.4): ordinary `Err` only warns; `Abort` escalates to engine
            // fatal after every listener has been notified.
            let listeners = eng.listeners.lock().unwrap().clone();
            let mut abort = None;
            for listener in listeners {
                if let Err(error) = listener.on_stable_block(*height, *hash, origin) {
                    eprintln!("[Engine] listener.on_stable_block failed: {}", error);
                    if error.is_abort() {
                        abort = Some(error);
                    }
                }
            }
            if let Some(error) = abort {
                eng.report_core_error(&error);
            }
        }
        eng.block_history.forget(*height, hash);
    }
    Ok(())
}

/// The batch for one root advance, streamed straight from the frozen layers so
/// the writes are never copied into a second owned map.
struct RootBatch<'a> {
    job: &'a RollJob,
}

impl MemDB for RootBatch<'_> {
    fn len(&self) -> usize {
        let writes: usize = self
            .job
            .chain
            .iter()
            .map(|chunk| chunk.frozen_state().map_or(0, |w| w.len()))
            .sum();
        writes + 2
    }

    fn for_each(&self, each: &mut dyn FnMut(&[u8], Option<&[u8]>)) {
        // Oldest first, so a key written by several blocks keeps last-write-wins.
        for chunk in &self.job.chain {
            if let Some(writes) = chunk.frozen_state() {
                writes.for_each(each);
            }
        }
        each(PERSIST_KEY_ROOT_HASH, Some(&hash_of(&self.job.new_root).0));
        let height = height_of(&self.job.new_root).to_be_bytes();
        each(PERSIST_KEY_ROOT_HEIGHT, Some(&height));
    }
}
