//! Protocol-side Hacash transaction billing (`TxGasMeter`): final burn/refund from
//! `used_net()`; returned-gas charges only the extra9 delta. Modes: Soft (Type1/2 budget, no escrow), Running (Type3 escrow + `gas_refund` settle).

use base::{Context, CoreState, hac_add, hac_sub, total_add_u12, with_base_total};
use field::Amount;
use sys::{Rerr, Ret, errf};

/// Returned-gas extra9 delta only (plain actions add no returned-gas charge).
#[allow(dead_code)] // reserved for future gas accounting extensions
#[inline(always)]
pub fn extra9_surcharge(extra9: bool, gas: u32) -> u32 {
    if extra9 { gas.saturating_mul(9) } else { 0 }
}

#[derive(Clone, Copy)]
struct GasPrice {
    purity_fee: i128,
    purity_size: i128,
}

impl GasPrice {
    fn from_context(ctx: &dyn Context) -> Ret<Self> {
        let tx = ctx.tx();
        let raw_fee = tx
            .fee_got()
            .to_238_u128()
            .map_err(|e| sys::Error::fault(format!("tx gas price invalid: {}", e)))?;
        let purity_size = tx.billing_size()? as u128;
        let floor = ctx
            .services()
            .vm_params()?
            .fee_purity_floor_at(ctx.env().block.height) as u128;
        let floor_fee = floor
            .checked_mul(purity_size)
            .ok_or_else(|| sys::Error::fault("tx gas price invalid"))?;
        let purity_fee = raw_fee.max(floor_fee);
        if purity_fee > i128::MAX as u128 || purity_size > i128::MAX as u128 {
            return errf!("tx gas price invalid");
        }
        let purity_fee = purity_fee as i128;
        let purity_size = purity_size as i128;
        if purity_fee <= 0 || purity_size <= 0 {
            return errf!("tx gas price invalid");
        }
        Ok(Self {
            purity_fee,
            purity_size,
        })
    }
}

/// Source of truth for protocol-side gas billing.
#[derive(Clone)]
pub(crate) struct TxGasMeter {
    running: bool,
    remaining: i64,
    used: i64,
    rebated: i64,
    max_charge: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasDiag {
    pub running: bool,
    pub remaining: i64,
    pub used: i64,
    pub rebated: i64,
    pub used_net: i64,
    pub max_charge: Amount,
}

impl Default for TxGasMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl TxGasMeter {
    pub fn new() -> Self {
        Self {
            running: false,
            remaining: 0,
            used: 0,
            rebated: 0,
            max_charge: Amount::zero(),
        }
    }

    fn calc_burn_amount(cost: i64, price: &GasPrice) -> Ret<Amount> {
        if cost <= 0 {
            return errf!("gas cost invalid");
        }
        let num = (cost as i128)
            .checked_mul(price.purity_fee)
            .ok_or_else(|| sys::Error::fault("gas burn overflow"))?;
        let den = price.purity_size;
        if den <= 0 {
            return errf!("gas settle denominator invalid");
        }
        let burn = num
            .checked_add(den - 1)
            .ok_or_else(|| sys::Error::fault("gas burn overflow"))?
            / den;
        if burn <= 0 {
            return errf!("gas burn underflow");
        }
        if burn > u64::MAX as i128 {
            return errf!("gas burn overflow");
        }
        Ok(Amount::unit238(burn as u64))
    }

    pub fn remaining(&self) -> i64 {
        self.remaining
    }

    pub fn diag(&self) -> GasDiag {
        GasDiag {
            running: self.running,
            remaining: self.remaining,
            used: self.used,
            rebated: self.rebated,
            used_net: self.used_net(),
            max_charge: self.max_charge.clone(),
        }
    }

    pub fn rebated_checkpoint(&self) -> i64 {
        self.rebated
    }

    pub fn restore_rebated(&mut self, rebated: i64) {
        self.rebated = rebated;
    }

    #[inline(always)]
    fn used_net(&self) -> i64 {
        let cut = self.rebated.min(self.used);
        self.used - cut
    }

    fn used_charge(&self, price: &GasPrice) -> Ret<Amount> {
        if !self.max_charge.is_positive() {
            return errf!("gas not initialized");
        }
        let used = self.used_net();
        if used <= 0 {
            return Ok(Amount::zero());
        }
        Self::calc_burn_amount(used, price)
    }

    fn begin(&mut self, budget: i64, max_charge: Amount) -> Rerr {
        if budget <= 0 {
            return errf!("gas budget invalid");
        }
        if self.running {
            return errf!("gas already initialized");
        }
        if self.max_charge.is_positive() {
            return errf!("gas already settled");
        }
        self.running = true;
        self.remaining = budget;
        self.used = 0;
        self.rebated = 0;
        self.max_charge = max_charge;
        Ok(())
    }

    fn finalize(&mut self, price: &GasPrice) -> Ret<(Amount, Amount)> {
        if !self.running {
            if self.max_charge.is_positive() {
                return errf!("gas already settled");
            }
            return errf!("gas not initialized");
        }
        let used_charge = self.used_charge(price)?;
        let refund = self.max_charge.sub_mode_u128(&used_charge)?;
        self.running = false;
        Ok((refund, used_charge))
    }

    pub fn charge(&mut self, gas: i64) -> Rerr {
        if gas < 0 {
            return errf!("gas cost invalid");
        }
        if gas == 0 {
            return Ok(());
        }
        if !self.running {
            return if self.max_charge.is_positive() {
                errf!("gas already settled")
            } else {
                errf!("gas not initialized")
            };
        }
        let Some(next) = self.remaining.checked_sub(gas) else {
            return errf!("gas has run out");
        };
        if next < 0 {
            return errf!("gas has run out");
        }
        self.remaining = next;
        self.used = self
            .used
            .checked_add(gas)
            .ok_or_else(|| sys::Error::fault("gas has run out"))?;
        Ok(())
    }

    pub fn rebate(&mut self, gas: i64) -> Rerr {
        if gas < 0 {
            return errf!("gas refund invalid");
        }
        if !self.running {
            return if self.max_charge.is_positive() {
                errf!("gas already settled")
            } else {
                errf!("gas not initialized")
            };
        }
        if gas == 0 {
            return Ok(());
        }
        self.rebated = self
            .rebated
            .checked_add(gas)
            .ok_or_else(|| sys::Error::fault("gas refund overflow"))?;
        Ok(())
    }
}

/// Decode `gas_max` and initialize when budget > 0. Returns whether gas was started.
pub fn tx_gas_initialize(ctx: &mut dyn Context) -> Ret<bool> {
    let tx = ctx.tx();
    let txty = tx.ty();
    let Some(gas_max_byte) = tx.gas_max_byte() else {
        return errf!("tx type {} gas_max must exist", txty);
    };
    let params = crate::execution_params(ctx.services().as_ref())?;
    let budget = params.decode_gas_budget(gas_max_byte.min(params.tx_gas_budget_cap_byte));
    if budget <= 0 {
        return Ok(false);
    }
    ctx.gas_initialize(budget)?;
    Ok(true)
}

pub(crate) fn gas_initialize_on(gas: &mut TxGasMeter, ctx: &mut dyn Context, budget: i64) -> Rerr {
    if gas.running {
        return errf!("gas already initialized");
    }
    if gas.max_charge.is_positive() {
        return errf!("gas already settled");
    }
    if budget <= 0 {
        return errf!("gas budget invalid");
    }
    let price = GasPrice::from_context(ctx)?;
    let params = crate::execution_params(ctx.services().as_ref())?;
    let cap = params.decode_gas_budget(params.tx_gas_budget_cap_byte);
    let budget = budget.min(cap);
    let max_burn_amt = TxGasMeter::calc_burn_amount(budget, &price)?;
    let main = ctx.env().tx.main;
    hac_sub(ctx, &main, &max_burn_amt)?;
    gas.begin(budget, max_burn_amt)
}

pub(crate) fn gas_refund_on(gas: &mut TxGasMeter, ctx: &mut dyn Context) -> Rerr {
    let price = GasPrice::from_context(ctx)?;
    let (refund, used_charge) = gas.finalize(&price)?;
    if refund.is_positive() {
        let main = ctx.env().tx.main;
        hac_add(ctx, &main, &refund)?;
    }
    if !used_charge.is_positive() {
        return Ok(());
    }
    let used_238 = used_charge.to_238_u64()?;
    if used_238 == 0 {
        return Ok(());
    }
    let mut state = CoreState::wrap(ctx.layer());
    with_base_total(&mut state, |ttcount| {
        total_add_u12(
            &mut ttcount.ast_vm_gas_burn_238,
            used_238 as u128,
            "ast_vm_gas_burn_238",
        )
    })?;
    Ok(())
}
