//! Root advance: write the newly stable blocks to disk, then move the root.
//!
//! State writes and root markers go into one batch, so a crash either loses
//! the whole advance or keeps all of it.
//! That is what lets boot trust `root_height` and replay from it safely.

use base::{MemDB, PERSIST_KEY_ROOT_HASH, PERSIST_KEY_ROOT_HEIGHT, PkgOrigin};
use field::Hash;
use sys::Ret;

use crate::engine::ChainEngine;
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
        // batch. Commit validates again after the write.
        eng.tree.validate_roll(&job)?;
        eng.store.disk().try_write(&RootBatch { job: &job })?;
        eng.tree.commit_roll(&job)?;
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
    notify_stable(eng, &stable, origin, stored_replay);
    Ok(stable)
}

fn notify_stable(
    eng: &ChainEngine,
    stable: &[(u64, Hash)],
    origin: PkgOrigin,
    stored_replay: bool,
) {
    // Consensus tracks stable blocks to age out its bidding state. Replaying
    // stored blocks re-derives that from the state it already loaded, and the
    // callback would cost a disk read plus a full decode per block.
    let notify_consensus = !stored_replay;
    for (height, hash) in stable {
        if notify_consensus {
            let block = eng.block_history.cached(*height, hash).or_else(|| {
                let data = eng.store.block_data(hash)?;
                eng.registry.decode_block(&data).ok().map(|(blk, _)| blk)
            });
            if let Some(block) = block {
                // Listeners that query BlockHistory during this callback reuse
                // the same decoded object.
                eng.block_history.remember(block.clone());
                eng.consensus.on_stable_block(block.as_ref(), eng);
            }
        }
        for listener in eng.listeners.lock().unwrap().iter() {
            listener.on_stable_block(*height, *hash, origin);
        }
        eng.block_history.forget(*height, hash);
    }
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
