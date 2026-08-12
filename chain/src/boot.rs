//! Boot: bring the in-memory tree up to the tip of the block store.
//!
//! The state DB records the root; blocks above it are replayed from the block
//! store on every start. A missing or stale state DB triggers a full rebuild
//! from genesis. Any probe/validate/replay/rebuild failure rejects startup
//! with a structured boot error; the process exit decision belongs to the
//! outermost caller.
//!
//! After the canonical replay the side hash list is read and the volatile
//! side tree is rebuilt from it. That step is peripheral: failures only clear
//! the list and skip the side replay, never the canonical boot.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base::{BlkPkg, PipelineOptions, PipelineReport, PkgOrigin, PkgSource, ProgressSink, StateStatus};
use field::Hash;
use sys::errf;

use crate::engine::ChainEngine;

/// Build a boot failure. `kind` only distinguishes the ops action:
/// "repair/rebuild the store" vs "switch binary / migrate offline". The outer
/// layer prints the message and refuses to enter Running.
pub(crate) fn boot_fault(
    phase: &'static str,
    kind: &'static str,
    message: impl Into<String>,
) -> sys::Error {
    sys::Error::fault(format!("boot {} failed [{}]: {}", phase, kind, message.into()))
}

/// Probe-phase storage failure (the state status read).
pub(crate) fn probe_fault(message: impl Into<String>) -> sys::Error {
    boot_fault("probe", "storage", message)
}

/// Validate-phase storage failure (state root vs block store checks).
pub(crate) fn validate_fault(message: impl Into<String>) -> sys::Error {
    boot_fault("validate", "storage", message)
}

/// Replay-phase storage failure (the canonical re-apply).
pub(crate) fn replay_fault(message: impl Into<String>) -> sys::Error {
    boot_fault("replay", "storage", message)
}

/// Rebuild-phase storage failure (genesis re-initialization).
pub(crate) fn rebuild_fault(message: impl Into<String>) -> sys::Error {
    boot_fault("rebuild", "storage", message)
}

pub fn open_state(eng: &ChainEngine, status: StateStatus) -> sys::Rerr {
    // Storage read failures surface as StorageReadPanic through the
    // Option-based state API. Convert them to boot storage failures instead
    // of letting the panic escape boot (§3.1/§2.4 of the engine error
    // contract).
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_state_inner(eng, status)
    })) {
        Ok(result) => result,
        Err(payload) => match payload.downcast::<base::StorageReadPanic>() {
            Ok(fault) => Err(probe_fault(format!("storage read failed: {}", fault.error))),
            Err(payload) => std::panic::resume_unwind(payload),
        },
    }
}

fn open_state_inner(eng: &ChainEngine, status: StateStatus) -> sys::Rerr {
    let _guard = eng.inserting.lock().unwrap();
    let state = eng.store.stable_state();
    let root_height = match &status {
        StateStatus::Uninitialized => 0,
        StateStatus::Ready(status) => status.latest_height,
    };
    eng.consensus
        .validate_genesis_state(state.as_ref(), root_height)
        .map_err(|e| {
            boot_fault(
                "probe",
                "compatibility",
                format!("genesis state validation at height {}: {}", root_height, e),
            )
        })?;
    let rebuild = matches!(status, StateStatus::Uninitialized)
        || eng
            .consensus
            .genesis_state_needs_rebuild(state.as_ref(), root_height);
    let tip = resolve_available_cursor(eng)?;
    if let StateStatus::Ready(status) = &status {
        if status.latest_height > tip {
            return Err(validate_fault(format!(
                "state root height {} is above the block available cursor {}",
                status.latest_height, tip
            )));
        }
    }
    if rebuild {
        init_genesis_state(eng).map_err(|e| {
            rebuild_fault(format!("genesis initialization at height {}: {}", root_height, e))
        })?;
        if tip > 0 {
            replay(eng, 1, tip, PkgOrigin::Rebuild)?;
        }
    } else {
        replay(eng, root_height + 1, tip, PkgOrigin::Replay)?;
    }
    // Peripheral: failures clear the side hash list and skip the side replay.
    replay_side_branches(eng)?;
    Ok(())
}

/// Resolve the block available cursor. A missing cursor is a storage boot
/// failure unless the block store is provably empty (fresh install); it must
/// never silently fall back to zero, which would treat a non-empty store as
/// empty and restart from genesis.
fn resolve_available_cursor(eng: &ChainEngine) -> sys::Ret<u64> {
    let tip = eng
        .store
        .block_store()
        .available_cursor()
        .map_err(|e| probe_fault(format!("block available cursor read failed: {}", e)))?;
    let Some(tip) = tip else {
        let has_records = eng
            .store
            .block_store()
            .has_records()
            .map_err(|e| probe_fault(format!("block store scan failed: {}", e)))?;
        if !has_records {
            return Ok(0);
        }
        return Err(probe_fault(
            "block available cursor is missing or corrupt (has_records=true)",
        ));
    };
    Ok(tip)
}

/// Write the consensus genesis state and point the root at the genesis block.
fn init_genesis_state(eng: &ChainEngine) -> sys::Rerr {
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
fn replay(eng: &ChainEngine, from: u64, to: u64, origin: PkgOrigin) -> sys::Rerr {
    if from > to {
        return eng
            .rebuild_runtime_caches()
            .map_err(|e| replay_fault(format!("runtime cache rebuild: {}", e)));
    }
    let is_rebuild = PkgOrigin::Rebuild == origin;
    if is_rebuild {
        println!(
            "[Database] scan all {} blocks to upgrade state db version, plase waiting...",
            to
        );
    } else {
        print!("[Engine] data: {}, replay ({})", eng.config.data_dir, from);
    }

    let source = Box::new(crate::source::LocalReplay::new(
        eng.registry.clone(),
        eng.store.block_store(),
        from,
        to,
    ));
    let mut opts = PipelineOptions::default();
    opts.origin = origin;
    opts.progress_sink = Some(Arc::new(ReplayProgress::new(origin, from, to)));
    crate::sync::run_replay_locked(eng, source, from, to, opts).map_err(|e| {
        replay_fault(format!("canonical replay [{}..{}]: {}", from, to, e))
    })?;

    if is_rebuild {
        println!("finish.");
    } else {
        println!(" ok.");
    }
    Ok(())
}

/// Rebuild the volatile side tree from the side hash list after the canonical
/// replay. Peripheral: any root-above record that is missing, corrupt, or
/// unexecutable clears the whole list and skips this replay; canonical boot
/// is unaffected. No arrival/admission records and no listener notifications.
fn replay_side_branches(eng: &ChainEngine) -> sys::Rerr {
    let Some(path) = &eng.side_list_path else {
        return Ok(());
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!(
                "[Engine] side hash list unreadable ({}); skip side replay",
                e
            );
            return Ok(());
        }
    };
    if bytes.len() % 32 != 0 {
        eprintln!(
            "[Engine] side hash list has partial records; clearing it and skipping side replay"
        );
        clear_side_list(path);
        return Ok(());
    }
    let mut hashes: Vec<Hash> = bytes
        .chunks_exact(32)
        .map(|chunk| {
            let mut buf = [0u8; 32];
            buf.copy_from_slice(chunk);
            Hash::from(buf)
        })
        .collect();
    hashes.sort_unstable();
    hashes.dedup();

    // A failed replay is either a bad record (the whole list is cleared and
    // the side replay skipped) or a storage failure (boot aborts: the store
    // cannot be trusted). Only catch_storage_panic tags errors with
    // StorageReadFailed; everything else is a bad record.
    if let Err(e) = replay_side(eng, &hashes) {
        if crate::engine::CoreFault::StorageReadFailed.is(&e) {
            return Err(rebuild_fault(format!("side replay storage failure: {}", e)));
        }
        side_replay_fail(path, &e);
        return Ok(());
    }
    Ok(())
}

/// Decode every recoverable side record and re-attach it to the recovered
/// tree. Blocks are replayed in topological order (height, hash): earlier
/// blocks are already attached, so the tree itself holds the side parents.
/// The body was fully validated before live persistence, so replay skips the
/// live consensus checks and body writes. Every error names the offending
/// block hash.
fn replay_side(eng: &ChainEngine, hashes: &[Hash]) -> sys::Ret<()> {
    let root_height = eng.tree.root_height();
    let head_hash = eng.tree.head_tip().0;
    let canonical: HashSet<Hash> = eng
        .tree
        .branch_blocks(&head_hash)
        .into_iter()
        .flatten()
        .map(|blk| blk.hash())
        .collect();

    // Decode and order every recoverable record by height (topological).
    let mut pending: Vec<(u64, Hash, base::BlockRef)> = Vec::new();
    for hash in hashes {
        if canonical.contains(hash) {
            continue;
        }
        let data = eng.store.block_data(hash).map_err(|e| {
            sys::Error::fault(format!("block {:?}: side body read failed: {}", hash, e))
        })?;
        let Some(data) = data else {
            return errf!("block {:?}: side body is missing from the block db", hash);
        };
        let Ok((blk, _)) = eng.registry.decode_block(&data) else {
            return errf!("block {:?}: side body cannot be decoded", hash);
        };
        if blk.hash() != *hash {
            return errf!("block {:?}: stored hash does not match the decoded block", hash);
        }
        if blk.height() <= root_height {
            continue; // below the durable root: pruned history, discard
        }
        pending.push((blk.height(), *hash, blk));
    }
    pending.sort_by_key(|(height, hash, _)| (*height, *hash));

    // Re-execute the deterministic state and attach without moving the head.
    // Branches over `side_tree_capacity` are pruned weakest-first by the
    // shared capacity bound below.
    for (_height, hash, blk) in pending {
        let prev_hash = blk.prev_hash();
        let pkg = BlkPkg::from_block(blk.clone(), PkgSource::new(PkgOrigin::Replay));
        let Some((_, _, fork_choice)) = crate::engine::catch_storage_panic(|| {
            crate::insert::resolve_fork_choice(eng, &pkg)
        })?
        else {
            return errf!("block {:?}: parent is not in the recovered tree", hash);
        };
        let Some((chunk, _)) = eng
            .tree
            .begin_block_execution(&prev_hash, pkg.block_ref(), fork_choice)?
        else {
            return errf!("block {:?}: parent is not in the recovered tree", hash);
        };
        let chunk = crate::engine::catch_storage_panic(|| {
            crate::insert::execute_block(eng, pkg.block(), chunk, false)
        })?;
        eng.tree.attach_side(&prev_hash, chunk)?;
    }
    eng.tree
        .enforce_side_capacity(eng.config.side_tree_capacity);
    Ok(())
}

fn side_replay_fail(path: &Path, error: &sys::Error) {
    eprintln!(
        "[Engine] side replay failed: {}; clearing the side hash list and skipping side replay",
        error
    );
    clear_side_list(path);
}

fn clear_side_list(path: &Path) {
    if let Err(e) = std::fs::write(path, &[] as &[u8]) {
        eprintln!("[Engine] cannot clear side hash list: {}", e);
    }
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
