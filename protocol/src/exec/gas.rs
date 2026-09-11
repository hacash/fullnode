//! Protocol-side Hacash transaction billing (`TxGasMeter`): final burn/refund from
//! `used_net()`; returned-gas charges only the extra9 delta. Modes: Soft (Type1/2 budget, no escrow), Running (Type3 escrow + `gas_refund` settle).
//! Pricing (`purity_fee` / `purity_size`) stays in u232; every HAC amount written
//! to a balance is ceiled to the gas settlement unit (u238).

use base::{
    Context, CoreState, FEE_PRICING_UNIT, SETTLEMENT_SCALE, hac_add, hac_sub, total_add_u12,
    with_base_total,
};
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
    /// Gas unit price in the chain pricing unit (`FEE_PRICING_UNIT` = u232 per byte):
    /// `purity_fee = max(raw_fee, floor × purity_size)`, all figures already in u232.
    /// Pure core of [`GasPrice::from_context`], factored out for testability.
    fn from_parts(raw_fee: u128, purity_size: u128, floor: u128) -> Ret<Self> {
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

    fn from_context(ctx: &dyn Context) -> Ret<Self> {
        let tx = ctx.tx();
        // The declared fee is read directly in the pricing unit: a fee mantissa
        // below 238 (e.g. `123:236`) is preserved, not truncated to u238.
        let raw_fee = tx
            .fee_got()
            .to_unit_u128(FEE_PRICING_UNIT)
            .map_err(|e| sys::Error::fault(format!("tx gas price invalid: {}", e)))?;
        let purity_size = tx.billing_size()? as u128;
        let floor = ctx
            .services()
            .vm_params()?
            .fee_purity_floor_at(ctx.env().block.height) as u128;
        Self::from_parts(raw_fee, purity_size, floor)
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

    /// `ceil(cost × purity_fee / (purity_size × SETTLEMENT_SCALE))` in the gas
    /// settlement unit (u238). Pricing (`purity_fee` / `purity_size`) stays in
    /// u232; a single division is exact because `ceil(ceil(x)/m) == ceil(x/m)`.
    /// The u238 count has the same range as the pre-232 settle, so it fits in `u64`.
    fn calc_burn_amount(cost: i64, price: &GasPrice) -> Ret<Amount> {
        if cost <= 0 {
            return errf!("gas cost invalid");
        }
        let num = (cost as i128)
            .checked_mul(price.purity_fee)
            .ok_or_else(|| sys::Error::fault("gas burn overflow"))?;
        if price.purity_size <= 0 {
            return errf!("gas settle denominator invalid");
        }
        let den = price
            .purity_size
            .checked_mul(SETTLEMENT_SCALE as i128)
            .ok_or_else(|| sys::Error::fault("gas burn overflow"))?;
        let burn_238 = num
            .checked_add(den - 1)
            .ok_or_else(|| sys::Error::fault("gas burn overflow"))?
            / den;
        if burn_238 <= 0 {
            return errf!("gas burn underflow");
        }
        if burn_238 > u64::MAX as i128 {
            return errf!("gas burn overflow");
        }
        Ok(Amount::unit238(burn_238 as u64))
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
    // Settlement is already in u238, so a positive `used_charge` converts
    // exactly (`used_238 >= 1`). The `== 0` skip is unreachable but harmless.
    let used_238 = used_charge.to_238_u64()?;
    if used_238 != 0 {
        let mut state = CoreState::wrap(ctx.layer());
        with_base_total(&mut state, |ttcount| {
            total_add_u12(
                &mut ttcount.ast_vm_gas_burn_238,
                used_238 as u128,
                "ast_vm_gas_burn_238",
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::UNIT_238;

    fn burn(cost: i64, raw_fee: u128, size: u128, floor: u128) -> Ret<Amount> {
        let price = GasPrice::from_parts(raw_fee, size, floor)?;
        TxGasMeter::calc_burn_amount(cost, &price)
    }

    /// A sub-u238 fee mantissa that the old 238 path truncated to 0 still
    /// prices at 232 and settles by ceiling to 1 u238.
    #[test]
    fn sub_u238_fee_ceils_to_one_u238() {
        // fee `7:236` = 70,000 u232; billing size 3; used 1 gas.
        let fee = Amount::from("7:236").unwrap();
        let raw = fee.to_unit_u128(FEE_PRICING_UNIT).unwrap();
        assert_eq!(raw, 70_000);
        // the same fee under the old accumulator unit would truncate to 0 u238…
        assert_eq!(fee.to_238_u128().unwrap(), 0);
        // …yet ceil(70_000 / (3 × 10⁶)) = 1 u238.
        let out = burn(1, raw, 3, 0).unwrap();
        assert_eq!(out.to_unit_u128(UNIT_238).unwrap(), 1);
        assert!(out.unit() >= UNIT_238);
    }

    /// Dust-free fees (exact multiples of 1 u238) replay the pre-232 formula
    /// `ceil(cost × fee238 / size)` in u238.
    #[test]
    fn burn_matches_legacy_u238_formula_for_dust_free_fees() {
        let costs: [i64; 3] = [1, 7, 111_911];
        let fees_238: [u128; 4] = [1, 3, 50_000 * 300, 10u128.pow(17) / 10u128.pow(6)];
        let sizes: [u128; 5] = [1, 2, 3, 300, 16_384];
        for cost in costs {
            for fee238 in fees_238 {
                for size in sizes {
                    let expected = (cost as u128)
                        .checked_mul(fee238)
                        .map(|n| n.div_ceil(size))
                        .expect("legacy numerator fits u128");
                    let out = burn(cost, fee238 * SETTLEMENT_SCALE, size, 0).unwrap();
                    assert_eq!(
                        out.to_unit_u128(UNIT_238).unwrap(),
                        expected,
                        "cost={cost} fee238={fee238} size={size}"
                    );
                    assert!(out.unit() >= UNIT_238);
                }
            }
        }
    }

    /// Degenerate regime: floor 0 and `cost × fee_u232 < 10⁶ × size` still
    /// burns exactly 1 u238 (never zero, never a sub-238 dust amount).
    #[test]
    fn degenerate_sub_scale_fee_burns_one_u238() {
        let raw = Amount::unit232(10)
            .to_unit_u128(FEE_PRICING_UNIT)
            .unwrap();
        assert_eq!(raw, 10);
        assert!(1u128.checked_mul(raw).unwrap() < SETTLEMENT_SCALE * 300);
        let out = burn(1, raw, 300, 0).unwrap();
        assert_eq!(out.to_unit_u128(UNIT_238).unwrap(), 1);
        assert!(out.unit() >= UNIT_238);
    }

    /// `from_parts` keeps the floor-dominance rule `max(fee, floor × size)`.
    #[test]
    fn from_parts_applies_floor_over_declared_fee() {
        // mainnet floor 5×10¹⁰ u232/byte × 3 bytes dominates a 70,000 u232 fee.
        let price = GasPrice::from_parts(70_000, 3, 50_000_000_000).unwrap();
        assert_eq!(price.purity_fee, 150_000_000_000);
        assert_eq!(price.purity_size, 3);
        // a higher declared fee wins instead.
        let price = GasPrice::from_parts(10 * 150_000_000_000, 3, 50_000_000_000).unwrap();
        assert_eq!(price.purity_fee, 1_500_000_000_000);
        // zero fee + zero floor is invalid (no free gas).
        assert!(GasPrice::from_parts(0, 3, 0).is_err());
    }

    /// 10 HAC fee, size 300, cost 111,911 → ceil(111911 × 10¹⁷ / (300 × 10⁶))
    /// = 37_303_666_666_667 u238, which fits in `u64` (same range as the pre-232 settle).
    #[test]
    fn ten_hac_max_budget_burns_in_u64_u238() {
        let fee_10_hac_u232 = 10u128 * 10u128.pow(16);
        let out = burn(111_911, fee_10_hac_u232, 300, 0).unwrap();
        let burn_238 = out.to_unit_u128(UNIT_238).unwrap();
        assert!(burn_238 <= u64::MAX as u128, "burn {burn_238} must fit u64");
        assert_eq!(burn_238, 37_303_666_666_667);
        assert!(out.unit() >= UNIT_238);
    }

    /// A sub-238 pricing result rounds UP to 1 u238; the accumulator conversion
    /// is then exact (`to_238_u64() == 1`), not truncated to 0.
    #[test]
    fn sub_u238_pricing_ceils_to_one_u238() {
        let fee = Amount::from("7:236").unwrap();
        let raw = fee.to_unit_u128(FEE_PRICING_UNIT).unwrap();
        let out = burn(1, raw, 70_000, 0).unwrap(); // ceil(70_000 / (70_000 × 10⁶)) = 1 u238
        assert_eq!(out.to_unit_u128(FEE_PRICING_UNIT).unwrap(), SETTLEMENT_SCALE);
        assert_eq!(out.to_238_u128().unwrap(), 1);
        assert_eq!(out.to_238_u64().unwrap(), 1u64);
        assert_eq!(out.to_unit_u128(UNIT_238).unwrap(), 1);
        assert!(out.unit() >= UNIT_238);
    }

    // ================================ end-to-end settle pipeline ================================
    //
    // Drives the real meter through a `Context`: `tx_gas_initialize` escrow debit
    // → `gas_charge` → `gas_refund` (refund credit + `ast_vm_gas_burn_238` write),
    // with a wire-real type-3 `StdTransaction`.

    use base::{
        ActOut, BinaryCodecs, BlockHasherFn, Context, Env, ExecFrom, ExecutionServices,
        JsonCodecs, LogEntry, P2sh, StateLayer, StateRead, TexLedger, Transaction, TxRef, Vm,
        VmExecutionParams, VmHostActionDef, VmHostCallKind,
    };
    use crate::codec::action::TransferHacTo;
    use crate::codec::tx::StdTransaction;
    use field::{Address, Uint1};
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemLayer(HashMap<Vec<u8>, Vec<u8>>);

    impl StateRead for MemLayer {
        fn get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
            Ok(self.0.get(key).cloned())
        }
    }

    impl StateLayer for MemLayer {
        fn set(&mut self, key: &[u8], val: Vec<u8>) {
            self.0.insert(key.to_vec(), val);
        }
        fn del(&mut self, key: &[u8]) {
            self.0.remove(key);
        }
    }

    /// Codec stubs; only `vm_params` / `execution_profile` are live.
    #[derive(Clone)]
    struct GasServices {
        params: VmExecutionParams,
    }

    fn stub_hasher(_height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }

    impl BinaryCodecs for GasServices {
        fn decode_action(&self, _buf: &[u8]) -> Ret<(base::ActionRef, usize)> {
            errf!("stub: decode_action")
        }
        fn decode_transaction(&self, _buf: &[u8]) -> Ret<(TxRef, usize)> {
            errf!("stub: decode_transaction")
        }
        fn decode_block(&self, _buf: &[u8]) -> Ret<(base::BlockRef, usize)> {
            errf!("stub: decode_block")
        }
        fn peek_block_size(&self, _buf: &[u8]) -> Ret<usize> {
            errf!("stub: peek_block_size")
        }
        fn block_hash(&self, _height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
            [0u8; base::HASH_SIZE]
        }
        fn block_hasher_fn(&self) -> BlockHasherFn {
            stub_hasher
        }
    }

    impl JsonCodecs for GasServices {
        fn decode_action_json(&self, _json: &str) -> Ret<base::ActionRef> {
            errf!("stub: decode_action_json")
        }
    }

    impl ExecutionServices for GasServices {
        fn assign_vm(&self, _height: u64) -> Option<Box<dyn Vm>> {
            None
        }
        fn vm_host_def(
            &self,
            _kind: VmHostCallKind,
            _id: u8,
        ) -> Option<&VmHostActionDef> {
            None
        }
        fn vm_params(&self) -> Ret<&VmExecutionParams> {
            Ok(&self.params)
        }
        fn execution_profile(&self) -> Ret<&'static dyn base::ExecutionProfile> {
            Ok(&hacash_params::MAINNET_PARAMS)
        }
        fn create_context(
            self: std::sync::Arc<Self>,
            _env: Env,
            _chunk: base::StateChunkRef,
            _tx: TxRef,
        ) -> Ret<Box<dyn Context>> {
            errf!("stub: create_context")
        }
    }

    struct SettleCtx {
        env: Env,
        tx: TxRef,
        services: GasServices,
        layer: MemLayer,
        gas: TxGasMeter,
        exec_from: ExecFrom,
        tex: TexLedger,
    }

    impl SettleCtx {
        fn new(main: Address, tx: TxRef, params: VmExecutionParams) -> Self {
            let mut env = Env::default();
            env.tx.main = main;
            Self {
                env,
                tx,
                services: GasServices { params },
                layer: MemLayer::default(),
                gas: TxGasMeter::new(),
                exec_from: ExecFrom::Top,
                tex: TexLedger::default(),
            }
        }
    }

    impl Context for SettleCtx {
        fn services(&self) -> std::sync::Arc<dyn ExecutionServices> {
            std::sync::Arc::new(self.services.clone())
        }
        fn env(&self) -> &Env {
            &self.env
        }
        fn tx(&self) -> &dyn Transaction {
            self.tx.as_ref()
        }
        fn exec_from(&self) -> ExecFrom {
            self.exec_from
        }
        fn exec_from_set(&mut self, from: ExecFrom) {
            self.exec_from = from;
        }
        fn check_sign(&mut self, _adr: &Address) -> Rerr {
            Ok(())
        }
        fn layer(&mut self) -> &mut dyn StateLayer {
            &mut self.layer
        }
        fn emit_log(&mut self, _entry: LogEntry) {}
        fn gas_remaining(&self) -> i64 {
            self.gas.remaining()
        }
        fn gas_charge(&mut self, gas: i64) -> Rerr {
            self.gas.charge(gas)
        }
        fn gas_rebate(&mut self, gas: i64) -> Rerr {
            self.gas.rebate(gas)
        }
        fn gas_initialize(&mut self, budget: i64) -> Rerr {
            let mut gas = std::mem::take(&mut self.gas);
            let res = gas_initialize_on(&mut gas, self, budget);
            self.gas = gas;
            res
        }
        fn gas_refund(&mut self) -> Rerr {
            let mut gas = std::mem::take(&mut self.gas);
            let res = gas_refund_on(&mut gas, self);
            self.gas = gas;
            res
        }
        fn snapshot_volatile(&self) -> Box<dyn std::any::Any> {
            Box::new(self.gas.rebated_checkpoint())
        }
        fn restore_volatile(&mut self, snap: Box<dyn std::any::Any>) {
            let rebated = *snap.downcast::<i64>().expect("gas snapshot type mismatch");
            self.gas.restore_rebated(rebated);
        }
        fn action_call(&mut self, _kind: u16, _body: Vec<u8>) -> Ret<ActOut> {
            errf!("stub: action_call")
        }
        fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
            None
        }
        fn vm_put(&mut self, _vm: Box<dyn Vm>) {}
        fn as_context_mut(&mut self) -> &mut dyn Context {
            self
        }
        fn tex_ledger(&self) -> &TexLedger {
            &self.tex
        }
        fn p2sh_set(&mut self, _addr: Address, _p2sh: Box<dyn P2sh>) -> Rerr {
            errf!("stub: p2sh_set")
        }
    }

    fn balance(ctx: &mut SettleCtx, addr: &Address) -> Amount {
        CoreState::wrap(&mut ctx.layer)
            .balance(addr)
            .unwrap()
            .map(|b| b.hacash)
            .unwrap_or_default()
    }

    fn gas_burn_total(ctx: &mut SettleCtx) -> u128 {
        CoreState::wrap(&mut ctx.layer)
            .get_base_total()
            .unwrap()
            .ast_vm_gas_burn_238
            .uint()
    }

    fn ceil_div(n: u128, d: u128) -> u128 {
        n.div_ceil(d)
    }

    /// A wire-real type-3 tx (one transfer, one signer) at the maximal budget byte.
    fn type3_tx(fee: &Amount) -> (Address, TxRef, usize) {
        let acc = sys::Account::create_by_password("gas-e2e").unwrap();
        let addr = Address::from(*acc.address());
        let mut tx =
            StdTransaction::new_by(hacash_params::TX_TYPE_3, addr, fee.clone(), 1_700_000_000);
        tx.push_action_in(std::sync::Arc::new(TransferHacTo::new(addr, Amount::mei(1))));
        tx.gas_max = Uint1::from(hacash_params::TX_GAS_BUDGET_CAP_BYTE);
        tx.fill_sign_account(&acc).unwrap();
        let size = tx.billing_size().unwrap();
        (addr, std::sync::Arc::new(tx), size)
    }

    /// Full pipeline, mainnet-shaped params: a 10 HAC fee on a ~120-byte type-3 tx
    /// escrows `ceil(budget × fee232 / (size × 10⁶))` u238, refunds the unused
    /// part, and books the used burn into the 238 accumulator exactly.
    #[test]
    fn type3_full_settle_escrows_refunds_and_books_exact_u238() {
        let fee = Amount::from("10:248").unwrap(); // 10 HAC = 10^17 u232
        let (addr, tx, size) = type3_tx(&fee);
        assert!(size > 0 && size < 400, "type3 billing size {size}");
        let mut ctx = SettleCtx::new(addr, tx, hacash_params::MAINNET_PARAMS.protocol.vm);
        // The maximal-budget escrow is budget/size ≈ 700× the declared fee, so the
        // funding must cover ~7,100 HAC to hold a 10 HAC fee at byte 99.
        let initial = Amount::from("20000:248").unwrap();
        base::hac_add(&mut ctx, &addr, &initial).unwrap();

        assert!(tx_gas_initialize(&mut ctx).unwrap(), "max budget initializes gas");
        let fee232 = fee.to_unit_u128(FEE_PRICING_UNIT).unwrap();
        let budget = hacash_params::MAINNET_PARAMS
            .protocol
            .decode_gas_budget(hacash_params::TX_GAS_BUDGET_CAP_BYTE) as u128;
        let escrow = ceil_div(budget * fee232, size as u128 * SETTLEMENT_SCALE);
        assert!(
            escrow <= u64::MAX as u128,
            "escrow {escrow} must fit u64 (u238 range matches the pre-232 settle)"
        );
        assert_eq!(
            balance(&mut ctx, &addr),
            initial
                .sub_mode_u128(&Amount::coin_u128(escrow, UNIT_238))
                .unwrap(),
            "escrow debits the ceiled u238 max-charge"
        );

        ctx.gas_charge(50_000).unwrap();
        ctx.gas_refund().unwrap();
        let used = ceil_div(50_000 * fee232, size as u128 * SETTLEMENT_SCALE);
        let after = balance(&mut ctx, &addr);
        assert_eq!(
            after,
            initial
                .sub_mode_u128(&Amount::coin_u128(used, UNIT_238))
                .unwrap(),
            "refund returns escrow minus the used burn"
        );
        assert!(
            after.unit() >= UNIT_238,
            "balance must not carry sub-238 dust (unit {})",
            after.unit()
        );
        assert_eq!(
            gas_burn_total(&mut ctx),
            used,
            "accumulator books the exact u238 burn"
        );
    }

    /// A tiny fee still burns exactly 1 u238 over the real pipeline, and the
    /// 238 accumulator records that 1 (no truncation to 0).
    #[test]
    fn sub_u238_fee_refunds_over_real_pipeline_and_books_one_u238() {
        let params = VmExecutionParams {
            contract_store_perm_periods: 10_000,
            contract_storage_fee: base::ContractStorageFeeParams::disabled(),
            initial_fee_purity_floor: 0,
            fee_purity_reductions: &[],
            gas_budget_lookup: &hacash_params::GAS_BUDGET_LOOKUP_1P07_FROM_138,
            tx_gas_budget_cap_byte: 99,
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
        };
        let fee = Amount::unit232(10);
        let (addr, tx, size) = type3_tx(&fee);
        let mut ctx = SettleCtx::new(addr, tx, params);
        let initial = Amount::from("1:248").unwrap();
        base::hac_add(&mut ctx, &addr, &initial).unwrap();

        assert!(tx_gas_initialize(&mut ctx).unwrap());
        ctx.gas_charge(1).unwrap();
        ctx.gas_refund().unwrap();
        // ceil(1 × 10 / (size × 10⁶)) = 1 u238
        let used = ceil_div(10, size as u128 * SETTLEMENT_SCALE);
        assert_eq!(used, 1);
        let after = balance(&mut ctx, &addr);
        assert_eq!(
            after,
            initial
                .sub_mode_u128(&Amount::unit238(1))
                .unwrap(),
            "refund applied; exactly 1 u238 stays debited"
        );
        assert!(
            after.unit() >= UNIT_238,
            "balance must not carry sub-238 dust (unit {})",
            after.unit()
        );
        assert_eq!(gas_burn_total(&mut ctx), 1, "tiny fee books exactly 1 u238");
    }
}
