//! TEX settlement: `do_settlement` verifies the tex ledger residuals cancel and
//! moves owned diamonds to their `diatrs` addresses; runs only at `ExecFrom::Top`.

use std::collections::HashMap;

use base::{
    Context, CoreState, ExecFrom, diamond_owned_move, hacd_move_one_diamond, hacd_transfer,
};
use field::{Amount, DiamondName, DiamondNameListMax200, Hash, SatoshiAuto};
use sys::{Rerr, errf};

// Defined in `params` (non-exec module) because `TexCellAct`'s codec reads it too;
// `tex` compiles unconditionally and is dead-code-eliminated in SDK/wasm builds.
pub use crate::params::SETTLEMENT_ADDR;

/// Close the tex ledger: settlement checks (residuals must be zero) then move the
/// `diatrs`-listed diamonds out of the settlement address to their owners.
pub fn do_settlement(ctx: &mut dyn Context) -> Rerr {
    if ctx.exec_from() != ExecFrom::Top {
        return errf!("do_settlement only allowed in TOP context");
    }
    let mut diamond_trs = Vec::new();
    let fast_sync = ctx.env().chain.fast_sync;
    {
        let tex = ctx.tex_ledger_mut_top()?;
        if !fast_sync {
            if tex.zhu != 0 || tex.sat != 0 || tex.dia != 0 {
                return errf!("coin settlement check failed");
            }
            let mut assets = HashMap::<u64, i128>::new();
            for entry in &tex.entries {
                let Some(next) = assets
                    .get(&entry.asset_serial)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(entry.delta)
                else {
                    return errf!("asset <{}> settlement overflow", entry.asset_serial);
                };
                assets.insert(entry.asset_serial, next);
            }
            for (serial, delta) in assets {
                if delta != 0 {
                    return errf!("asset <{}> settlement check failed", serial);
                }
            }
        }
        let mut diamonds = std::mem::take(&mut tex.diamonds);
        for (addr, count) in &tex.diatrs {
            let list = take_head_diamonds(&mut diamonds, *count)?;
            diamond_trs.push((*addr, list));
        }
        if !fast_sync && !diamonds.is_empty() {
            return errf!("diamonds settlement check failed");
        }
        tex.diamonds = diamonds;
    }
    for (addr, dialist) in diamond_trs {
        let diamond_form_flag = crate::execution_params(ctx.services().as_ref())?.diamond_form_flag;
        let diamond_form = ctx.env().chain.consensus_flags & diamond_form_flag != 0;
        let mut state = CoreState::wrap(ctx.layer());
        for name in dialist.as_list() {
            hacd_move_one_diamond(&mut state, &SETTLEMENT_ADDR, &addr, name)?;
        }
        if diamond_form {
            diamond_owned_move(&mut state, &SETTLEMENT_ADDR, &addr, &dialist)?;
        }
        hacd_transfer(
            &mut state,
            &SETTLEMENT_ADDR,
            &addr,
            &field::DiamondNumber::from(dialist.length() as u32),
            &dialist,
        )?;
    }
    Ok(())
}

fn take_head_diamonds(diamonds: &mut Vec<Hash>, count: usize) -> sys::Ret<DiamondNameListMax200> {
    if count == 0 {
        return errf!("diamond get count cannot be zero");
    }
    if diamonds.len() < count {
        return errf!("diamonds settlement check failed");
    }
    let taken = diamonds.drain(..count).collect::<Vec<_>>();
    let names = taken
        .into_iter()
        .map(|h| DiamondName::from(first6(h)))
        .collect::<Vec<_>>();
    let list = DiamondNameListMax200::from(names)?;
    list.check()?;
    Ok(list)
}

fn first6(hash: Hash) -> [u8; DiamondName::SIZE] {
    let mut buf = [0u8; DiamondName::SIZE];
    buf.copy_from_slice(&hash.as_ref()[..DiamondName::SIZE]);
    buf
}

/// Clear leaked HAC/SAT/Asset on the settlement address after normal settlement;
/// diamond ownership is intentionally left to `do_settlement`.
pub fn settlement_addr_postsettle_cleanup(ctx: &mut dyn Context) -> Rerr {
    let mut state = CoreState::wrap(ctx.layer());
    if let Some(mut bls) = state.balance(&SETTLEMENT_ADDR)? {
        let mut dirty = false;
        if bls.hacash > Amount::zero() {
            bls.hacash = Amount::zero();
            dirty = true;
        }
        if bls.satoshi.uint() > 0 {
            bls.satoshi = SatoshiAuto::default();
            dirty = true;
        }
        if bls.assets.length() > 0 {
            bls.assets = field::AssetAmtW1::default();
            dirty = true;
        }
        if dirty {
            state.balance_set(&SETTLEMENT_ADDR, &bls);
        }
    }
    Ok(())
}
