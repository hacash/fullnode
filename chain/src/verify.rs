//! Protocol-level block checks that are independent of consensus. These run on
//! every block except fast-sync ones.

use base::{BlkPkg, Block};
use field::Hash;
use sys::{Rerr, errf};

use crate::engine::ChainEngine;

/// Cheap checks that need no parent block: wire limits.
/// Run on arrival, before the block is queued for insertion.
pub fn check_intrinsic(eng: &ChainEngine, pkg: &BlkPkg) -> Rerr {
    let params = eng.consensus.mint_params();
    if params.max_block_txs > 0 && pkg.block().transaction_count() as usize > params.max_block_txs {
        return errf!(
            "block tx count {} exceeds max {}",
            pkg.block().transaction_count(),
            params.max_block_txs
        );
    }
    if params.max_block_size > 0 && pkg.size() > params.max_block_size + 100 {
        return errf!(
            "block wire size {} exceeds max payload {} plus header allowance",
            pkg.size(),
            params.max_block_size
        );
    }
    Ok(())
}

/// Full structural validation against the parent block: linkage, timestamps,
/// transaction layout and the merkle root.
pub fn verify_block(eng: &ChainEngine, pkg: &BlkPkg, prev: &dyn Block) -> Rerr {
    let params = eng.consensus.mint_params();
    let blk = pkg.block();
    let txs = blk.transactions();
    if txs.is_empty() {
        return errf!("block has no transactions; a prelude transaction is required");
    }
    if blk.transaction_count() as usize != txs.len() {
        return errf!(
            "block tx count says {} but {} transactions were decoded",
            blk.transaction_count(),
            txs.len()
        );
    }
    if !txs[0].is_block_prelude() {
        return errf!("block first transaction must be a prelude transaction");
    }
    if blk.prev_hash() != prev.hash() {
        return errf!(
            "block prev hash {:?} does not match parent {:?}",
            blk.prev_hash(),
            prev.hash()
        );
    }

    let now = sys::curtimes();
    if blk.timestamp() > now {
        return errf!(
            "block timestamp {} is in the future (now {})",
            blk.timestamp(),
            now
        );
    }
    if blk.timestamp() <= prev.timestamp() {
        return errf!(
            "block timestamp {} must be later than parent timestamp {}",
            blk.timestamp(),
            prev.timestamp()
        );
    }

    let prelude_ty = txs[0].ty();
    let mut hashes = Vec::with_capacity(txs.len());
    let mut total_size = 0usize;
    for (idx, tx) in txs.iter().enumerate() {
        if idx > 0 {
            if tx.is_block_prelude() {
                return errf!("prelude transaction cannot appear at index {}", idx);
            }
            if tx.ty() == prelude_ty {
                return errf!(
                    "tx({}) must not repeat the prelude type {}",
                    idx,
                    prelude_ty
                );
            }
            if tx.action_count() != tx.actions().len() {
                return errf!(
                    "tx({}) action count does not match its decoded actions",
                    idx
                );
            }
            if tx.timestamp().value() > now {
                return errf!(
                    "tx({}) timestamp {} is in the future (now {})",
                    idx,
                    tx.timestamp().value(),
                    now
                );
            }
        }
        let size = tx.size();
        total_size += size;
        if base::tx_exceeds_max_size(size, params.max_tx_size) {
            return errf!(
                "tx({}) size {} exceeds max {}",
                idx,
                size,
                params.max_tx_size
            );
        }
        hashes.push(tx.hash_with_fee());
    }
    if params.max_block_size > 0 && total_size > params.max_block_size {
        return errf!(
            "block transaction payload {} exceeds max {}",
            total_size,
            params.max_block_size
        );
    }
    if merkle_root(&hashes) != blk.mrklroot() {
        return errf!("block merkle root does not match its transactions");
    }
    Ok(())
}

fn merkle_root(list: &[Hash]) -> Hash {
    if list.is_empty() {
        return Hash::default();
    }
    let mut layer = list.to_vec();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2 + 1);
        for pair in layer.chunks(2) {
            let right = pair.get(1).unwrap_or(&pair[0]);
            let mut buf = Vec::with_capacity(Hash::SIZE * 2);
            buf.extend_from_slice(pair[0].as_ref());
            buf.extend_from_slice(right.as_ref());
            next.push(Hash::from(sys::calculate_hash(buf)));
        }
        layer = next;
    }
    layer[0]
}
