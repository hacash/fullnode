//! Root advance: write the newly stable blocks to disk, then move the root.
//! State writes and root markers share one batch, so boot can trust `root_height`.

use base::{
    MemDB, PERSIST_KEY_ROOT_HASH, PERSIST_KEY_ROOT_HEIGHT, PkgOrigin, STATE_DECODE_FAILED_CODE,
    STATE_READ_FAILED_CODE,
};
use field::Hash;
use sys::{Rerr, Ret};

use crate::engine::{ChainEngine, Phase, PostCommitPolicy, persist_fatal};
use crate::tree::{RollJob, hash_of, height_of};

/// Persist a root advance and move the tree root, marking the disk/tree
/// transition unstable. Write failures are `PostAttach`, callbacks `PostCommit` (§4.1/§4.2).
pub fn roll_root(
    eng: &ChainEngine,
    job: RollJob,
    origin: PkgOrigin,
    stored_replay: bool,
    post_commit: PostCommitPolicy,
) -> Ret<Vec<(u64, Hash)>> {
    {
        let mut root_move = eng.begin_root_move();
        // Reject a broken ordering before it publishes an incomplete root
        // batch; only disk/root writes are tagged `persist_failed` (§8.3).
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
    notify_stable(eng, &stable, origin, stored_replay, post_commit)?;
    Ok(stable)
}

fn notify_stable(
    eng: &ChainEngine,
    stable: &[(u64, Hash)],
    origin: PkgOrigin,
    stored_replay: bool,
    post_commit: PostCommitPolicy,
) -> Rerr {
    // Replay skips stable-block callbacks and listeners: restart must not
    // re-publish non-durable events (§8). The pending-cache forget still runs.
    for (height, hash) in stable {
        if !stored_replay {
            // Stable-block notification is consensus-critical (§8.3): read/decode
            // failures are `Abort + STATE_*_FAILED_CODE`, PostCommit, never rolled back (§4.2).
            let block = match eng.block_history.cached(*height, hash) {
                Some(block) => block,
                None => {
                    let data = match eng.store.block_data(hash) {
                        Ok(data) => data,
                        Err(e) => {
                            let err = sys::Error::abort(format!(
                                "stable block {} body read failed: {}",
                                hash, e
                            ))
                            .with_code(STATE_READ_FAILED_CODE);
                            return post_commit_err(
                                eng,
                                "stable_block_body_read",
                                *height,
                                hash,
                                err,
                                post_commit,
                            );
                        }
                    };
                    let Some(data) = data else {
                        let err = sys::Error::abort(format!(
                            "stable block {} body is missing from the block db",
                            hash
                        ))
                        .with_code(STATE_READ_FAILED_CODE);
                        return post_commit_err(
                            eng,
                            "stable_block_body_read",
                            *height,
                            hash,
                            err,
                            post_commit,
                        );
                    };
                    match eng.registry.decode_block(&data) {
                        Ok((block, _)) => block,
                        Err(e) => {
                            let err = sys::Error::abort(format!(
                                "stable block {} body cannot be decoded: {}",
                                hash, e
                            ))
                            .with_code(STATE_DECODE_FAILED_CODE);
                            return post_commit_err(
                                eng,
                                "stable_block_body_decode",
                                *height,
                                hash,
                                err,
                                post_commit,
                            );
                        }
                    }
                }
            };
            // Listeners that query BlockHistory during this callback reuse
            // the same decoded object.
            eng.block_history.remember(block.clone());
            // A consensus callback error is engine-fatal: keep an `Abort`
            // as-is, otherwise escalate to `Abort + "core_failed"` (§8.1).
            if let Err(e) = eng.consensus.on_stable_block(block.as_ref(), eng) {
                let fatal = if e.is_abort() {
                    e
                } else {
                    sys::Error::abort(format!("consensus.on_stable_block failed: {}", e))
                        .with_code("core_failed")
                };
                let err = eng
                    .handle_error(
                        Phase::PostCommit,
                        "consensus.on_stable_block",
                        Some(*height),
                        Some(hash),
                        Err::<(), _>(fatal),
                    )
                    .unwrap_err();
                if matches!(post_commit, PostCommitPolicy::StopPipeline) {
                    return Err(err);
                }
            }
            // Listener registry mutex is released before any external callback
            // (§4.3); ordinary `Err` warns, `Abort` escalates after all notified.
            if let Err(error) = eng.notify_listeners(|l| l.on_stable_block(*height, *hash, origin))
            {
                let _ = eng.handle_error(
                    Phase::PostCommit,
                    "listener.on_stable_block",
                    Some(*height),
                    Some(hash),
                    Err::<(), _>(error),
                );
            }
        }
        eng.block_history.forget(*height, hash);
    }
    Ok(())
}

/// Record a PostCommit failure at the engine boundary and either stop the
/// pipeline or keep the committed fact according to `post_commit` (§4.2).
fn post_commit_err(
    eng: &ChainEngine,
    operation: &'static str,
    height: u64,
    hash: &Hash,
    err: sys::Error,
    post_commit: PostCommitPolicy,
) -> Rerr {
    let err = eng
        .handle_error(
            Phase::PostCommit,
            operation,
            Some(height),
            Some(hash),
            Err::<(), _>(err),
        )
        .unwrap_err();
    if matches!(post_commit, PostCommitPolicy::StopPipeline) {
        return Err(err);
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
