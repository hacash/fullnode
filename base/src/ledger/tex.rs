use std::collections::{HashMap, HashSet};

use field::{Address, AssetAmt, DiamondName, DiamondNameListMax200, Fold64};
use sys::{Ret, errf};

use crate::Context;

use super::CoreState;
use super::operate::check_diamonds_ownership;

#[derive(Clone, Default, Debug)]
pub struct TexLedger {
    pub zhu: i64,
    pub sat: i64,
    pub dia: i32,
    /// Diamond names committed by `DiaPay` cells as `(payer, name)` records in
    /// pay order. Deferred clearing moves nothing at record time; settlement
    /// delivers each name straight from its original payer.
    pub diamonds: Vec<(Address, DiamondName)>,
    /// Names already present in `diamonds`, so a duplicate `DiaPay` is rejected
    /// at record time (the physical escrow move used to reject duplicates
    /// implicitly through the ownership check). Cleared when settlement takes
    /// the records; no `DiaPay` runs after settlement.
    pub dia_recorded: HashSet<DiamondName>,
    pub diatrs: Vec<(Address, usize)>,
    /// Per-address pending deferred-clearing totals: names this payer committed
    /// through `DiaPay` / counts this recipient is due through `DiaGet`. Kept
    /// alongside the ordered records so projected HACD-count conditions are
    /// O(1) instead of scanning every pending record. Per-tx scratch like the
    /// rest of the ledger.
    pub dia_pay_pending: HashMap<Address, usize>,
    pub dia_get_pending: HashMap<Address, usize>,
    pub entries: Vec<TexEntry>,
    pub asset_checked: HashSet<u64>,
}

/// DiaPay record-time booking (deferred clearing): the named diamonds are not
/// moved. The authorizer must currently own every one of them (check-only
/// ownership query), a name already recorded in this transaction is rejected,
/// and on success the names join the FIFO pay-records and the `dia` counter
/// advances. Delivery happens in `do_settlement`.
pub fn record_diamond_pay(
    ctx: &mut dyn Context,
    payer: &Address,
    names: &DiamondNameListMax200,
    max_diamonds: usize,
) -> Ret<()> {
    {
        let mut state = CoreState::wrap(ctx.layer());
        check_diamonds_ownership(&mut state, payer, names)?;
    }
    let tex = ctx.tex_ledger_mut_top()?;
    let n = names.length() as i32;
    let Some(newdia) = tex.dia.checked_add(n) else {
        return errf!("cell state diamond record overflow");
    };
    let Some(new_len) = tex.diamonds.len().checked_add(names.length()) else {
        return errf!("tex diamond pay record overflow");
    };
    if new_len > max_diamonds {
        return errf!("tex diamond pay exceeds {} diamonds", max_diamonds);
    }
    for name in names.as_list() {
        if !tex.dia_recorded.insert(*name) {
            return errf!(
                "diamond {} is already recorded for settlement",
                name.to_readable()
            );
        }
        tex.diamonds.push((*payer, *name));
    }
    tex.dia = newdia;
    // Successful record only: failed cells revert the whole transaction, but
    // the O(1) pending total must never run ahead of the ordered records.
    let pending = tex.dia_pay_pending.entry(*payer).or_insert(0);
    let Some(next) = pending.checked_add(names.length()) else {
        return errf!("tex diamond pay pending overflow");
    };
    *pending = next;
    Ok(())
}

impl TexLedger {
    pub fn record_diamond_get(
        &mut self,
        addr: Address,
        count: usize,
        max_diamonds: usize,
    ) -> Ret<()> {
        if count == 0 {
            return errf!("tex diamond_get count cannot be zero");
        }
        // The cap is per transaction: the SUM over every `DiaGet` record in
        // this tx, not each cell on its own — otherwise many just-under-cap
        // cells would jointly exceed the limit the parameter name promises.
        let total = self
            .diatrs
            .iter()
            .try_fold(0usize, |acc, (_, c)| acc.checked_add(*c))
            .and_then(|acc| acc.checked_add(count));
        match total {
            Some(total) if total <= max_diamonds => {}
            _ => return errf!("Tex state diamond trs num cannot exceed {}", max_diamonds),
        }
        let Some(diares) = self.dia.checked_sub(count as i32) else {
            return errf!("cell state diamond overflow");
        };
        self.dia = diares;
        self.diatrs.push((addr, count));
        let pending = self.dia_get_pending.entry(addr).or_insert(0);
        let Some(next) = pending.checked_add(count) else {
            return errf!("tex diamond get pending overflow");
        };
        *pending = next;
        Ok(())
    }

    pub fn record_asset_pay(&mut self, asset: &AssetAmt) -> Ret<()> {
        self.record_asset_delta(
            Address::default(),
            asset.serial.uint(),
            asset.amount.uint() as i128,
        )
    }

    pub fn record_asset_get(&mut self, addr: Address, asset: &AssetAmt) -> Ret<()> {
        self.record_asset_delta(addr, asset.serial.uint(), -(asset.amount.uint() as i128))
    }

    pub fn record_asset_delta(&mut self, addr: Address, asset_serial: u64, delta: i128) -> Ret<()> {
        if asset_serial == 0 {
            return errf!("tex asset serial cannot be zero");
        }
        self.entries.push(TexEntry {
            addr,
            asset_serial,
            delta,
        });
        Ok(())
    }

    pub fn mark_asset_checked(&mut self, serial: Fold64) {
        self.asset_checked.insert(serial.uint());
    }

    pub fn asset_is_checked(&self, serial: Fold64) -> bool {
        self.asset_checked.contains(&serial.uint())
    }
}

#[derive(Clone, Debug)]
pub struct TexEntry {
    pub addr: Address,
    pub asset_serial: u64,
    pub delta: i128,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diamond_get_cap_is_cumulative_across_cells() {
        let mut tex = TexLedger::default();
        tex.dia = 400;
        tex.record_diamond_get(Address::from([1u8; 21]), 200, 200).unwrap();
        // A second just-at-cap cell must not slip past the per-tx total.
        let err = tex
            .record_diamond_get(Address::from([2u8; 21]), 1, 200)
            .unwrap_err();
        assert!(err.to_string().contains("cannot exceed"), "{err}");
        // Room under the cap still records.
        tex.record_diamond_get(Address::from([2u8; 21]), 200, 400).unwrap();
    }
}
