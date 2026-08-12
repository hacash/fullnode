//! The single-block apply path: verify, execute and attach one block, shared
//! by discover, the sync pipeline and boot replay.

use base::{ApplyMode, BlkPkg, BlockRef, Env, ForkChoiceKey, StateChunkRef};
use field::Hash;
use sys::{Ret, errf};

use crate::engine::{ApplyAccepted, ApplyResult, ChainEngine};
use crate::history::BranchHistory;

fn tree_fatal(error: sys::Error) -> sys::Error {
    sys::Error::fault(format!("chain tree invariant failed: {}", error))
        .with_code(crate::engine::CoreFault::CoreFailed.code())
}

/// Execute a block into a detached Block chunk. Each transaction gets its own
/// child chunk and becomes visible to later transactions only after success.
pub fn execute_block(
    eng: &ChainEngine,
    blk: &dyn base::Block,
    mut block: StateChunkRef,
    fast_sync: bool,
) -> Ret<StateChunkRef> {
    let txs = blk.transactions();
    let Some(prelude) = txs.first() else {
        return errf!("block has no transactions; a prelude transaction is required");
    };
    if !prelude.is_block_prelude() {
        return errf!("block first transaction must be a prelude transaction");
    }

    let mut env = Env::default();
    env.chain.id = eng.consensus.chain_id();
    env.chain.fast_sync = fast_sync;
    env.chain.consensus_flags = eng.consensus.chain_flags(blk.height());
    env.block.height = blk.height();
    env.block.hash = blk.hash();
    let prelude = blk.prelude_transaction()?;
    if let Some(author) = prelude.author() {
        env.block.author = author;
    }
    let fee_receiver = prelude.fee_receiver();

    let mut total_fee = field::Amount::zero();
    for (idx, tx) in txs.iter().enumerate() {
        if idx > 0 && tx.is_block_prelude() {
            return errf!("prelude transaction cannot appear at index {}", idx);
        }
        let mut tx_env = env.clone();
        tx_env.tx = base::TxInfo {
            ty: tx.ty(),
            main: tx.main(),
            addrs: tx.addrs(),
            fee: tx.fee().clone(),
        };
        let tx_chunk = block.spawn_tx_child(tx.hash())?;
        let mut ctx = eng
            .registry
            .clone()
            .create_context(tx_env, tx_chunk, tx.clone())?;
        tx.execute(ctx.as_mut())?;
        let tx_chunk = ctx.release_chunk()?;
        let parent = tx_chunk.commit_to_parent()?;
        debug_assert!(parent.ptr_eq(&block));
        total_fee = total_fee.add_mode_u64(&tx.fee_got())?;
    }
    if let Some(receiver) = fee_receiver.filter(|_| total_fee.is_positive()) {
        base::hac_add_state(&mut block, &receiver, &total_fee)?;
    }
    Ok(block)
}

/// Resolve the candidate's parent and compute the fork-choice key along the
/// parent's branch. `Ok(None)` means the parent is not in the tree (orphan).
/// Shared by the live insert path and boot side replay, which must both see
/// the same deterministic fork choice over the reconstructed ancestry.
pub(crate) fn resolve_fork_choice<'a>(
    eng: &'a ChainEngine,
    pkg: &'a BlkPkg,
) -> Ret<Option<(BlockRef, BranchHistory<'a>, ForkChoiceKey)>> {
    let prev_hash = pkg.block().prev_hash();
    let Some((parent_block, parent_key)) = eng.tree.block_context(&prev_hash) else {
        return Ok(None);
    };
    let branch_blocks = eng.tree.branch_blocks(&prev_hash).ok_or_else(|| {
        tree_fatal(sys::Error::fault(
            "candidate parent disappeared from fork tree",
        ))
    })?;
    let branch_history = eng.block_history.for_branch(branch_blocks);
    let fork_choice = eng.consensus.fork_choice_key(pkg, &parent_key, &branch_history)?;
    Ok(Some((parent_block, branch_history, fork_choice)))
}

/// A live side branch failed to commit; drop it without classifying the block
/// (decision table #5). The canonical tree is untouched and the pipeline
/// continues. Not an error and not a classification.
fn side_discard(height: u64, hash: &Hash, phase: &str, error: sys::Error) -> Ret<ApplyResult> {
    eprintln!(
        "[Engine] side block <{}, {:?}> {}: {}; branch discarded",
        height, hash, phase, error
    );
    Ok(ApplyResult::Discarded)
}

/// Validate, execute and attach one block. Callers serialize this with
/// `eng.inserting`, which is also what excludes concurrent root movement:
/// every root-move writer holds the same mutex, so no second lock is needed.
/// Fast-sync relies on its enforced linear-head invariant instead.
/// `persist_body` is false only for the internal replay pipeline, whose
/// bodies already exist.
pub fn insert_block(
    eng: &ChainEngine,
    pkg: &BlkPkg,
    mode: ApplyMode,
    persist_body: bool,
) -> Ret<ApplyResult> {
    let fast_sync = mode.is_fast_sync();
    if !fast_sync && !eng.is_root_available() {
        return errf!("chain state is unavailable pending root recovery");
    }
    let height = pkg.height();
    let prev_hash = pkg.block().prev_hash();
    // The pre-attach canonical head; used only when the insert reorgs.
    let prev_head = eng.tree.head_tip().0;

    let root_height = eng.tree.root_height();
    let head_height = eng.tree.head_height();
    if height <= root_height || height > head_height + 1 {
        return errf!(
            "block height {} outside insertable range [{}, {}]",
            height,
            root_height + 1,
            head_height + 1
        );
    }
    if eng.tree.contains(&pkg.hash()) {
        return Ok(ApplyResult::Duplicate(pkg.hash()));
    }
    if fast_sync {
        // Cheap fail-fast: reject a non-extension before executing the block.
        // attach_linear re-checks the same invariant at attach time.
        let (head_hash, current_height) = eng.tree.head_tip();
        if prev_hash != head_hash || height != current_height.saturating_add(1) {
            return errf!(
                "fast-sync block <{}, {:?}> does not extend head <{}, {:?}>",
                height,
                pkg.hash(),
                current_height,
                head_hash
            );
        }
    }
    let Some((parent_block, branch_history, fork_choice)) = resolve_fork_choice(eng, pkg)? else {
        return Ok(ApplyResult::Orphan(prev_hash));
    };

    if !fast_sync {
        crate::verify::verify_block(eng, pkg, parent_block.as_ref())?;
    }
    // The fast-sync flag is handed to the mint: validation-only
    // implementations skip their checks there, while side-effectful ones
    // still see every block and decide for themselves.
    eng.consensus
        .check_block_before_execute(pkg, parent_block.as_ref(), &branch_history, fast_sync)?;
    // The commit plan (side vs canonical) must be fixed before the side body
    // write can be ordered ahead of the attach. `inserting` is held, so the
    // head cannot change and this comparison agrees with the attach below.
    let plan_head = eng.tree.head_fork_choice() < fork_choice;
    let Some((chunk, parent_state)) = eng
        .tree
        .begin_block_execution(&prev_hash, pkg.block_ref(), fork_choice)
        .map_err(tree_fatal)?
    else {
        return Ok(ApplyResult::Orphan(prev_hash));
    };
    // Execution errors: a live strict side branch is discarded (decision
    // table #5); everything else — including any fast-sync failure, which can
    // never be a legitimate side branch — is a real error.
    let chunk = match execute_block(eng, pkg.block(), chunk, fast_sync) {
        Ok(chunk) => chunk,
        Err(e) if !fast_sync && !plan_head => {
            return side_discard(height, &pkg.hash(), "execution failed", e);
        }
        Err(e) => return Err(e),
    };

    match eng
        .consensus
        .check_block_after_execute(pkg, &chunk, parent_state.as_ref(), eng, fast_sync)
    {
        Ok(()) => {}
        // A live side branch failing the post-execute state check is
        // discarded like any other side failure (decision table #5): the
        // canonical tree is untouched and the stream continues. Only a
        // canonical candidate returns the error (Rejected-class).
        Err(e) if !fast_sync && !plan_head => {
            return side_discard(height, &pkg.hash(), "post-execute check failed", e);
        }
        Err(e) => return Err(e),
    }

    // Commit: three disjoint arms. Fast sync is strictly linear — attach_linear
    // rejects a non-head plan, never a side branch. Strict mode follows the
    // plan fixed above (canonical attach vs side branch).
    if fast_sync {
        let inserted = eng
            .tree
            .attach_linear(&prev_hash, chunk, eng.config.unstable_block)
            .map_err(tree_fatal)?;
        return Ok(ApplyResult::Accepted(ApplyAccepted {
            pkg: pkg.clone(),
            inserted,
            persist_body,
            prev_head,
        }));
    }
    if plan_head {
        let inserted = eng
            .tree
            .attach(&prev_hash, chunk, eng.config.unstable_block)
            .map_err(tree_fatal)?;
        return Ok(ApplyResult::Accepted(ApplyAccepted {
            pkg: pkg.clone(),
            inserted,
            persist_body,
            prev_head,
        }));
    }

    // Side branch: the immutable body must be durable before the in-memory
    // attach, so a body write failure can drop the detached chunk without
    // touching the tree (no recover, no detach; §3.2).
    if persist_body {
        if let Err(e) = eng
            .store
            .block_store()
            .put_block(height, &pkg.hash(), pkg.data().clone())
        {
            return side_discard(height, &pkg.hash(), "body write failed", e);
        }
    }
    let inserted = match eng
        .tree
        .attach(&prev_hash, chunk, eng.config.unstable_block)
    {
        Ok(inserted) => inserted,
        // An attach failure drops the branch without a recovery hint: the
        // body stays on disk but is never replayed, so boot's "any invalid
        // record clears the list" rule cannot nuke the other side branches.
        Err(e) => return side_discard(height, &pkg.hash(), "attach failed", e),
    };
    // Best-effort recovery hint for a branch that is now live; a dropped hint
    // only costs side rebuild.
    eng.side_list.append(pkg.hash());
    // The plan was fixed under the same stable head; a head mismatch here is
    // a tree invariant bug and the canonical index write would fail loudly.
    debug_assert!(!inserted.is_head, "side plan mismatch: block became head");
    eng.tree
        .enforce_side_capacity(eng.config.side_tree_capacity);
    Ok(ApplyResult::Accepted(ApplyAccepted {
        pkg: pkg.clone(),
        inserted,
        persist_body,
        prev_head,
    }))
}
