//! Block execution and insertion — the one path every block goes through.

use base::{ApplyMode, BlkPkg, Env, StateChunkRef};
use field::Hash;
use sys::{Ret, errf};

use crate::engine::ChainEngine;
use crate::tree::Inserted;

/// How much validation a block gets. The caller selects this trusted execution
/// mode explicitly; package source metadata never changes validation depth.
pub fn is_fast_sync(mode: ApplyMode) -> bool {
    mode.is_fast_sync()
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

/// Result of `insert_block`.
pub enum Insert {
    Accepted {
        height: u64,
        confirmed_txs: Vec<Hash>,
        reverted_txs: Vec<Hash>,
        is_head: bool,
        reorg: bool,
        roll: Option<crate::tree::RollJob>,
    },
    Duplicate,
    /// The parent is not in the tree; the caller should request it.
    Orphan(Hash),
}

/// Validate, execute and attach one block. Callers serialize this with
/// `eng.inserting`. Strict execution also holds the root-move read lock;
/// fast-sync relies on its enforced linear-head invariant instead.
pub fn insert_block(eng: &ChainEngine, pkg: &BlkPkg, mode: ApplyMode) -> Ret<Insert> {
    let fast_sync = is_fast_sync(mode);
    // Critical validation reads directly through the live chunk chain. Root
    // persistence must not change its disk fallback until attach completes.
    let _root_move = (!fast_sync).then(|| eng.root_move.read().unwrap());
    if !fast_sync && !eng.is_root_available() {
        return errf!("chain state is unavailable pending root recovery");
    }
    if fast_sync && !eng.is_root_readable_for_fast_sync() {
        return errf!("chain state is unavailable pending root recovery");
    }
    let height = pkg.height();
    let prev_hash = pkg.block().prev_hash();

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
        return Ok(Insert::Duplicate);
    }
    if fast_sync {
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
    let Some((parent_block, parent_key)) = eng.tree.block_context(&prev_hash) else {
        return Ok(Insert::Orphan(prev_hash));
    };
    let branch_blocks = eng
        .tree
        .branch_blocks(&prev_hash)
        .ok_or_else(|| sys::Error::fault("candidate parent disappeared from fork tree"))?;
    let branch_history = eng.block_history.for_branch(branch_blocks);

    if !fast_sync {
        crate::verify::verify_block(eng, pkg, parent_block.as_ref())?;
        eng.consensus
            .check_block_before_execute(pkg, parent_block.as_ref(), &branch_history)?;
    }

    let fork_choice = eng
        .consensus
        .fork_choice_key(pkg, &parent_key, &branch_history)?;
    let Some((chunk, parent_state)) =
        eng.tree
            .begin_block_execution(&prev_hash, pkg.block_ref(), fork_choice)?
    else {
        return Ok(Insert::Orphan(prev_hash));
    };
    let chunk = execute_block(eng, pkg.block(), chunk, fast_sync)?;

    if !fast_sync {
        eng.consensus
            .check_block_after_execute(pkg, &chunk, parent_state.as_ref(), eng)?;
    }

    let inserted = if fast_sync {
        eng.tree
            .attach_linear(&prev_hash, chunk, eng.config.unstable_block)?
    } else {
        eng.tree
            .attach(&prev_hash, chunk, eng.config.unstable_block)?
    };
    let Inserted {
        is_head,
        reorg,
        roll,
        confirmed_txs,
        reverted_txs,
    } = inserted;
    Ok(Insert::Accepted {
        height,
        confirmed_txs,
        reverted_txs,
        is_head,
        reorg,
        roll,
    })
}

#[cfg(test)]
mod tests {
    use field::Amount;

    #[test]
    fn block_fee_sum_keeps_the_legacy_u64_consensus_boundary() {
        let high = Amount::from("1:248").unwrap();
        let low = Amount::from("1:228").unwrap();
        assert!(high.add_mode_u64(&low).is_err());
        assert!(high.add_mode_u128(&low).is_ok());
    }
}
