//! Helpers to pick diamond-mint actions out of txs / blocks.

use base::{Block, Transaction};

use crate::action::coinbase_tx::CoinbaseTx;
use crate::action::diamond::DiamondMint;

pub fn pickout_diamond_mint_action(tx: &dyn Transaction) -> Option<DiamondMint> {
    if tx.ty() == CoinbaseTx::TYPE {
        return None;
    }
    for act in tx.actions() {
        if let Some(dm) = act.as_any().downcast_ref::<DiamondMint>() {
            return Some(dm.clone());
        }
    }
    None
}

pub fn pickout_diamond_mint_action_from_block(
    blk: &dyn Block,
) -> Option<(usize, base::TxRef, DiamondMint)> {
    let mut txposi: usize = 0;
    for tx in blk.transactions() {
        if let Some(act) = pickout_diamond_mint_action(tx.as_ref()) {
            return Some((txposi, tx.clone(), act));
        }
        txposi += 1;
    }
    None
}

pub fn get_diamond_mint_number(tx: &dyn Transaction) -> u32 {
    for act in tx.actions() {
        if let Some(dm) = act.as_any().downcast_ref::<DiamondMint>() {
            return dm.d.number.uint();
        }
    }
    0
}
