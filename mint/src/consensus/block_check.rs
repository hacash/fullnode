//! Consensus check helpers used by `HacashConsensus` (ported from OLD check/*).

use base::{BlkPkg, Block, BlockHistory, ChainView, PowBlockExt, Transaction, TxPkg};
use field::Amount;
use protocol::block_std::StdBlock;
use sys::{Rerr, errf};

use crate::MintConf;
use crate::action::diamond::{
    DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST, DiamondMint,
};
use crate::action::util::pickout_diamond_mint_action;
use crate::bidding::DiamondBidding;
use crate::coinbase::{verify_coinbase, verify_coinbase_privakey};
use crate::difficulty::{DifficultyGnr, hash_bigger_than, u32_to_hash};
use crate::minter::block_reward_number;
use crate::tx_coinbase::CoinbaseTx;

/// StdBlock intro byte length (fixed header fields, no txs).
const BLOCK_INTRO_SIZE: usize = 1 + 5 + 5 + 32 + 32 + 4 + 4 + 4 + 2;

pub fn check_tx(bidding: &DiamondBidding, view: &dyn ChainView, txp: &TxPkg) -> Rerr {
    let txr = txp.tx();
    let curr_hei = view.latest_height();
    let next_hei = curr_hei + 1;
    let Some(diamintact) = pickout_diamond_mint_action(txr) else {
        return Ok(());
    };
    if next_hei % 5 == 0 {
        return errf!("diamond mint transaction cannot be submitted after height ending in 4 or 9");
    }
    check_diamond_mint_minimum_bidding_fee(next_hei, txr, &diamintact)?;
    bidding.record(curr_hei, txp, &diamintact);
    Ok(())
}

pub fn check_diamond_mint_minimum_bidding_fee(
    next_hei: u64,
    tx: &dyn Transaction,
    dmact: &DiamondMint,
) -> Rerr {
    const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
    let bidmin = Amount::mei(block_reward_number(next_hei) as u64);
    let bidfee = tx.fee().clone();
    let dianum = dmact.d.number.uint();
    if bidfee < bidmin && dianum > CKN {
        return errf!(
            "diamond bidding fee {} cannot be less than {} after number {}",
            bidfee,
            bidmin,
            CKN
        );
    }
    Ok(())
}

/// Earliest bomb gate: size + cheap intro PoW against claimed difficulty.
pub fn check_block_data(data: &[u8], view: &dyn ChainView) -> Rerr {
    let max = view.consensus().mint_params().max_block_size;
    if max > 0 && data.len() > max.saturating_add(100) {
        return errf!(
            "block wire size {} exceeds max payload {} plus header allowance",
            data.len(),
            max
        );
    }
    if data.len() < BLOCK_INTRO_SIZE {
        return Ok(());
    }
    let Ok(intro) =
        StdBlock::decode_intro(view.services().block_hasher_fn(), &data[..BLOCK_INTRO_SIZE])
    else {
        return Ok(());
    };
    let hei = intro.height();
    let pow = intro.hash().into_array();
    let target = u32_to_hash(intro.pow_difficulty());
    if hash_bigger_than(&pow, &target) {
        return errf!(
            "block data PoW check failed at height {}: hash exceeds claimed difficulty target",
            hei
        );
    }
    Ok(())
}

pub fn check_block_arrive(
    difficulty: &DifficultyGnr,
    _mint_conf: &MintConf,
    pkg: &BlkPkg,
    view: &dyn ChainView,
) -> Rerr {
    check_block_arrive_block(difficulty, pkg.block(), view)
}

pub fn check_block_arrive_data(
    difficulty: &DifficultyGnr,
    data: &[u8],
    view: &dyn ChainView,
) -> Rerr {
    if data.len() < BLOCK_INTRO_SIZE {
        return Ok(());
    }
    let Ok(intro) =
        StdBlock::decode_intro(view.services().block_hasher_fn(), &data[..BLOCK_INTRO_SIZE])
    else {
        return Ok(());
    };
    check_block_arrive_block(difficulty, &intro, view)
}

fn check_block_arrive_block(
    difficulty: &DifficultyGnr,
    curblk: &dyn Block,
    view: &dyn ChainView,
) -> Rerr {
    let curhei = curblk.height();
    let curdifnum = curblk.pow_difficulty();
    let cblkhx = curblk.hash().into_array();
    let history = view.block_history();

    // Pure validation: arrival records are published only after the block is
    // accepted (§6 of the engine error contract), so orphaned or unvalidated
    // blocks never pollute the bidding map.
    if difficulty.is_pre_asert_mainnet(curhei) {
        return Ok(());
    }

    if difficulty.is_asert_height(curhei) {
        let canonical_prev = history.block_at_height(curhei - 1)?.map(|b| b.hash());
        if canonical_prev.as_ref() == Some(&curblk.prev_hash()) {
            let prev = history
                .block_at_height(curhei - 1)?
                .ok_or_else(|| sys::Error::fault("prev block missing for asert arrive"))?;
            let target = difficulty.target_asert(
                prev.pow_difficulty(),
                curhei,
                curblk.timestamp(),
                history.as_ref(),
            );
            if target.num != curdifnum {
                return errf!(
                    "block found height {} PoW difficulty check failed: expected {} but got {}",
                    curhei,
                    target.num,
                    curdifnum
                );
            }
            if hash_bigger_than(&cblkhx, &target.hash) {
                return errf!("block found height {} PoW hashrates check failed", curhei);
            }
        }
    } else {
        // Non-mainnet pre-ASERT: soft check via retarget (bootstrap / weighted).
        let canonical_prev = history.block_at_height(curhei - 1)?.map(|b| b.hash());
        if canonical_prev.as_ref() == Some(&curblk.prev_hash()) {
            let prev = history
                .block_at_height(curhei - 1)?
                .ok_or_else(|| sys::Error::fault("prev block missing for difficulty arrive"))?;
            let target = difficulty.target(
                prev.pow_difficulty(),
                prev.timestamp(),
                curhei,
                curblk.timestamp(),
                history.as_ref(),
            );
            if target.num != curdifnum {
                return errf!(
                    "block found height {} PoW difficulty check failed: expected {} but got {}",
                    curhei,
                    target.num,
                    curdifnum
                );
            }
            if hash_bigger_than(&cblkhx, &target.hash) {
                return errf!("block found height {} PoW hashrates check failed", curhei);
            }
        }
    }
    Ok(())
}

pub fn check_block_before_execute(
    _mint_conf: &MintConf,
    difficulty: &DifficultyGnr,
    pkg: &BlkPkg,
    parent: &dyn Block,
    history: &dyn BlockHistory,
) -> Rerr {
    let curblk = pkg.block();
    let curhei = curblk.height();
    let ptx = curblk.prelude_transaction()?;
    if ptx.ty() != CoinbaseTx::TYPE {
        return errf!("mainnet prelude tx must be coinbase");
    }
    verify_coinbase(curhei, ptx)?;
    if difficulty.is_pre_asert_mainnet(curhei) {
        let blkcln = difficulty.adjust_blocks();
        if curhei >= blkcln.saturating_mul(200) {
            verify_coinbase_privakey(ptx)?;
        }
        return Ok(());
    }
    verify_coinbase_privakey(ptx)?;
    let curn = curblk.pow_difficulty();
    let target = difficulty.target(
        parent.pow_difficulty(),
        parent.timestamp(),
        curhei,
        curblk.timestamp(),
        history,
    );
    if target.num != curn {
        return errf!(
            "height {} PoW difficulty check failed: expected {} but got {}",
            curhei,
            target.num,
            curn
        );
    }
    let pow = curblk.hash().into_array();
    if hash_bigger_than(&pow, &target.hash) {
        return errf!(
            "height {} PoW hashrates check failed: must not exceed {} but got {}",
            curhei,
            hex::encode(target.hash),
            hex::encode(pow)
        );
    }
    Ok(())
}

pub fn check_highest_bid(
    bidding: &DiamondBidding,
    pkg: &BlkPkg,
    prev_state: &dyn base::StateRead,
) -> Rerr {
    bidding.check_highest_bid(pkg, prev_state)
}
