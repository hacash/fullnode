//! TEX settlement: `do_settlement` verifies the tex ledger residuals cancel and
//! delivers the DiaPay-committed diamonds straight from each payer to its
//! `diatrs` recipients; runs only at `ExecFrom::Top`.

use std::collections::HashMap;

use base::{Context, CoreState, ExecFrom, check_diamonds_ownership};
use field::{Address, Amount, DiamondName, DiamondNameListMax200, SatoshiAuto};
use sys::{Rerr, errf};

use crate::exec::apply::diamonds_transfer;

// Defined in `params` (non-exec module) because `TexCellExecute`'s codec reads it too;
// `tex` compiles unconditionally and is dead-code-eliminated in SDK/wasm builds.
pub use crate::params::SETTLEMENT_ADDR;

/// Close the tex ledger: settlement checks (residuals must be zero), then
/// deliver the recorded DiaPay diamonds from each payer to the `diatrs`
/// recipients. Nothing ever sits on the settlement address.
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
        // Deferred clearing: the pay-records never moved, so hand them out
        // now, FIFO, to the `diatrs` recipients. A recipient's count may span
        // several payer records; every segment keeps its original payer, and
        // the transfer re-verifies ownership so any mid-tx movement of a
        // recorded diamond fails the whole transaction.
        let mut pays = std::mem::take(&mut tex.diamonds);
        tex.dia_recorded.clear();
        for (addr, count) in &tex.diatrs {
            for (payer, dialist) in take_head_diamonds(&mut pays, *count)? {
                diamond_trs.push((payer, *addr, dialist));
            }
        }
        if !fast_sync && !pays.is_empty() {
            return errf!("diamonds settlement check failed");
        }
        tex.diamonds = pays;
    }
    for (payer, addr, dialist) in diamond_trs {
        if payer == addr {
            // Self-delivery: the diamonds already sit with the recipient, so
            // there is nothing to move — but the payer must still own them,
            // keeping every mid-tx movement on the revert path.
            let mut state = CoreState::wrap(ctx.layer());
            check_diamonds_ownership(&mut state, &payer, &dialist)?;
        } else {
            diamonds_transfer(ctx, &payer, &addr, &dialist)?;
        }
    }
    Ok(())
}

/// Take the next `count` FIFO pay-records for one `diatrs` entry and group
/// them into per-payer segments in pay order.
fn take_head_diamonds(
    pays: &mut Vec<(Address, DiamondName)>,
    count: usize,
) -> sys::Ret<Vec<(Address, DiamondNameListMax200)>> {
    if count == 0 {
        return errf!("diamond get count cannot be zero");
    }
    if pays.len() < count {
        return errf!("diamonds settlement check failed");
    }
    let mut segments: Vec<(Address, Vec<DiamondName>)> = Vec::new();
    for (payer, name) in pays.drain(..count) {
        match segments.last_mut() {
            Some((p, names)) if *p == payer => names.push(name),
            _ => segments.push((payer, vec![name])),
        }
    }
    segments
        .into_iter()
        .map(|(payer, names)| {
            let list = DiamondNameListMax200::from(names)?;
            list.check()?;
            Ok((payer, list))
        })
        .collect()
}

/// Clear leaked HAC/SAT/Asset on the settlement address after normal settlement;
/// diamonds no longer pass through the settlement address at all (deferred
/// clearing delivers them payer-to-recipient directly).
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
