//! Boot: bring the in-memory tree up to the tip of the block store.
//!
//! The state DB records the root; blocks above it are replayed from the block
//! store on every start. A missing or stale state DB triggers a full rebuild
//! from genesis.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base::{PipelineOptions, PipelineReport, PkgOrigin, ProgressSink, StateStatus};
use sys::Rerr;

use crate::engine::ChainEngine;
use crate::source::LocalReplay;

pub fn open_state(eng: &ChainEngine, status: StateStatus) -> Rerr {
    let _guard = eng.inserting.lock().unwrap();
    let state = eng.store.stable_state();
    let root_height = match &status {
        StateStatus::Uninitialized => 0,
        StateStatus::Ready(status) => status.latest_height,
    };
    eng.consensus
        .validate_genesis_state(state.as_ref(), root_height)?;
    let rebuild = matches!(status, StateStatus::Uninitialized)
        || eng
            .consensus
            .genesis_state_needs_rebuild(state.as_ref(), root_height);

    let tip = eng.store.block_store().available_cursor().unwrap_or(0);
    if rebuild {
        init_genesis_state(eng)?;
        if tip > 0 {
            return replay(eng, 1, tip, PkgOrigin::Rebuild);
        }
        return Ok(());
    }
    replay(eng, root_height + 1, tip, PkgOrigin::Replay)
}

/// Write the consensus genesis state and point the root at the genesis block.
fn init_genesis_state(eng: &ChainEngine) -> Rerr {
    let mut chunk = base::StateChunkRef::block_draft_on_disk(eng.store.disk(), 0);
    eng.consensus.initialize(&mut chunk)?;
    let mut delta = chunk.take_draft_delta()?;
    let genesis_hash = eng.genesis.hash();
    delta.state.put(
        base::PERSIST_KEY_ROOT_HASH.to_vec(),
        genesis_hash.0.to_vec(),
    );
    delta.state.put(
        base::PERSIST_KEY_ROOT_HEIGHT.to_vec(),
        0u64.to_be_bytes().to_vec(),
    );
    {
        let mut root_move = eng.begin_root_move();
        eng.store.disk().try_write(&delta.state)?;
        eng.tree.reset_root(eng.store.disk(), eng.genesis.clone());
        root_move.commit();
    }
    Ok(())
}

/// Re-apply stored blocks in order. Bodies are already on disk, so only state
/// is rebuilt. The caller must hold `eng.inserting`.
fn replay(eng: &ChainEngine, from: u64, to: u64, origin: PkgOrigin) -> Rerr {
    if from > to {
        return eng.rebuild_runtime_caches();
    }
    match origin {
        PkgOrigin::Rebuild => println!(
            "[Database] scan all {} blocks to upgrade state db version, plase waiting...",
            to
        ),
        _ => sys::flush!("[Engine] data: {}, replay ({})", eng.config.data_dir, from),
    }

    let source = Box::new(LocalReplay::new(eng.store.block_store(), from, to));
    let mut opts = PipelineOptions::default();
    opts.origin = origin;
    opts.progress_sink = Some(Arc::new(ReplayProgress::new(origin, from, to)));
    crate::sync::run_replay_locked(eng, source, from, to, opts)?;

    match origin {
        PkgOrigin::Rebuild => {
            println!("finish.");
        }
        _ => println!(" ok."),
    }
    Ok(())
}

struct ReplayProgress {
    origin: PkgOrigin,
    from: u64,
    to: u64,
    shown: AtomicU64,
}

impl ReplayProgress {
    fn new(origin: PkgOrigin, from: u64, to: u64) -> Self {
        Self {
            origin,
            from,
            to,
            shown: AtomicU64::new(from.saturating_sub(1)),
        }
    }
}

impl ProgressSink for ReplayProgress {
    fn on_pipeline_progress(&self, report: &PipelineReport) {
        let height = report
            .final_height
            .clamp(self.from.saturating_sub(1), self.to);
        let shown = self.shown.load(Ordering::Relaxed);
        if height <= shown {
            return;
        }
        match self.origin {
            PkgOrigin::Rebuild if height < self.to && height / 10_000 == shown / 10_000 => return,
            PkgOrigin::Rebuild => {
                let percent = height as f64 / self.to.max(1) as f64 * 100.0;
                sys::flush!("\r{:10} ({:.2}%) ", height, percent);
            }
            _ => sys::flush!("\u{27a2}{}", height),
        }
        self.shown.store(height, Ordering::Relaxed);
    }
}

/// Drop the in-memory tree back to the persisted root and replay the unstable
/// tail. Used when a sync stream fails partway through; the caller holds
/// `eng.inserting`.
pub fn recover(eng: &ChainEngine) -> Rerr {
    let status = eng.store.status();
    eng.block_history.clear_pending();
    {
        let mut root_move = eng.begin_root_move();
        let root_block = crate::engine::load_persisted_root_block(
            eng.registry.as_ref(),
            eng.store.as_ref(),
            &eng.genesis,
            &status,
        )?;
        eng.tree.reset_root(eng.store.disk(), root_block);
        root_move.commit();
    }
    let tip = eng.store.block_store().available_cursor().unwrap_or(0);
    replay(eng, status.latest_height + 1, tip, PkgOrigin::Replay)
}
