//! TexCellExecute execute body and TEX cell state changes.

use base::{
    Context, CoreState, ExecFrom, asset_add, asset_sub, hac_add, hac_sub, record_diamond_pay,
    sat_add, sat_sub,
};
use field::{AddrOrPtr, Address, Amount, Fold64, Hash, Satoshi, Sign, ToJSON};
use sys::{Account, Rerr, Ret, errf};

use crate::codec::action::tex::{TexCell, TexCellExecute};
use crate::exec::apply::diamonds_transfer;
use crate::params::SETTLEMENT_ADDR;

fn tex_check_asset_serial(ctx: &mut dyn Context, serial: Fold64) -> Rerr {
    if serial.is_zero() {
        return errf!("tex asset serial cannot be zero");
    }
    {
        let tex = ctx.tex_ledger();
        if tex.asset_is_checked(serial) {
            return Ok(());
        }
    }
    let exist = {
        let state = CoreState::wrap(ctx.layer());
        state.asset(&serial)?.is_some()
    };
    if !exist {
        return errf!("tex asset <{}> does not exist", serial.uint());
    }
    ctx.tex_ledger_mut_top()?.mark_asset_checked(serial);
    Ok(())
}

/// HACD count the bundle authorizer is *projected* to hold after the pending
/// deferred clearing in this transaction settles: the settled `Balance.diamond`
/// count minus the names this authorizer committed with `DiaPay`, plus the
/// counts it is due through `DiaGet`. Computed on demand from the ledger's
/// per-address pending counters, before `do_settlement`, and never written to
/// any ledger state; an invalid bundle still fails and reverts at settlement.
fn projected_diamond_count(ctx: &mut dyn Context, taradr: &Address) -> Ret<i128> {
    let actual = CoreState::wrap(ctx.layer())
        .balance(taradr)?
        .unwrap_or_default()
        .diamond
        .uint();
    let tex = ctx.tex_ledger();
    let pay_pending = tex.dia_pay_pending.get(taradr).copied().unwrap_or(0);
    let get_pending = tex.dia_get_pending.get(taradr).copied().unwrap_or(0);
    let Some(projected) = (actual as i128)
        .checked_sub(pay_pending as i128)
        .and_then(|value| value.checked_add(get_pending as i128))
    else {
        return errf!("tex projected diamond count overflow");
    };
    Ok(projected)
}

/// Directed cells move value outside the TEX escrow, so the endpoint they name
/// must be an ordinary user address: the settlement address, every other
/// system address (privkey unknown, including the blackhole), and every
/// non-privkey address (multisig/scriptmh/contract) are rejected. The
/// authorizer itself is not a valid endpoint either: a directed self-transfer
/// is always rejected instead of silently no-oping. The type restriction is
/// deliberate: directed cells move balances directly through
/// `hac_sub`/`hac_add` and never route through the transfer hook machinery
/// (`resolve_transfer_routing`), so crediting or debiting a contract or
/// script address here would silently skip its authorize/receive hooks.
fn check_direct_endpoint(taradr: &Address, addr: &Address) -> Rerr {
    if *addr == *taradr {
        return errf!(
            "directed cell endpoint cannot be the authorizer itself: {}",
            addr.to_readable()
        );
    }
    if let Err(e) = addr.must_privkey() {
        return errf!("direct cell endpoint must be a privkey user address: {}", e);
    }
    if *addr == SETTLEMENT_ADDR {
        return errf!(
            "direct cell cannot involve the tex settlement address {}",
            addr.to_readable()
        );
    }
    if addr.is_privkey_unknown() {
        return errf!(
            "direct cell cannot involve system address {} (privkey unknown)",
            addr.to_json()
        );
    }
    Ok(())
}

/// A directed PAY recipient must be pinned by the authorizer's signature:
/// the transaction main (`Ptr(0)`) or an explicit static address. Any other
/// pointer resolves against the transaction address list, which the
/// replayable bundle signature does not bind — the executor could place any
/// address at that slot and redirect the authorizer's funds. Directed GET
/// sources are exempt: they are declared in the action's `req_sign`, so the
/// signature consent binds them to the transaction.
fn check_direct_pay_form(to: &AddrOrPtr) -> Rerr {
    match to {
        AddrOrPtr::Addr(_) | AddrOrPtr::Ptr(0) => Ok(()),
        AddrOrPtr::Ptr(_) => errf!(
            "direct pay target must be the transaction main (ptr 0) or an explicit static address, not a free pointer"
        ),
    }
}

/// Resolve a DirectedGet source. Consent is no longer checked here: every
/// DirectedGet `from` is declared in the action's `req_sign`, so the dispatcher
/// runs the real signature check (`ctx.check_sign`) on each source before any
/// cell executes — co-signing the transaction is the payment consent. This
/// keeps the single maintenance point on the dispatch level, like
/// `TransferHacFrom`/`TransferHacFromTo`, and legalizes static sources that
/// co-sign. The exec-time system-address guard (`check_direct_endpoint`) stays.
fn resolve_directed_source(ctx: &dyn Context, from: &AddrOrPtr) -> Ret<Address> {
    from.real(&ctx.env().tx.addrs)
}

impl TexCell {
    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        match self {
            Self::ZhuPay { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_sub(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_add(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::ZhuGet { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_add(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_sub(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::SatPay { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_add(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::SatGet { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_add(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_sub(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::DiaPay { diamonds } => {
                // Deferred clearing: nothing moves here. The ledger booking
                // re-checks that the authorizer owns every named diamond and
                // rejects duplicates; do_settlement delivers the diamonds.
                let max = crate::execution_params(ctx.services().as_ref())?.tex_diamond_pay_max;
                record_diamond_pay(ctx, taradr, diamonds, max)
            }
            Self::DiaGet { dianum } => {
                if dianum.uint() == 0 {
                    return errf!("cell diamond get: number cannot be zero");
                }
                let max =
                    crate::execution_params(ctx.services().as_ref())?.tex_diamond_get_max_per_tx;
                ctx.tex_ledger_mut_top()?
                    .record_diamond_get(*taradr, dianum.uint() as usize, max)
            }
            Self::AssetPay { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_sub(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_pay(asset)?;
                Ok(())
            }
            Self::AssetGet { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_add(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_get(*taradr, asset)?;
                Ok(())
            }
            Self::DirectedZhuPay { to, haczhu } => {
                check_direct_pay_form(to)?;
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(taradr, &target)?;
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_sub(ctx, taradr, &amt)?;
                hac_add(ctx, &target, &amt)?;
                Ok(())
            }
            Self::DirectedZhuGet { from, haczhu } => {
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(taradr, &source)?;
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_sub(ctx, &source, &amt)?;
                hac_add(ctx, taradr, &amt)?;
                Ok(())
            }
            Self::DirectedSatPay { to, satnum } => {
                check_direct_pay_form(to)?;
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(taradr, &target)?;
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, taradr, &sat)?;
                sat_add(ctx, &target, &sat)?;
                Ok(())
            }
            Self::DirectedSatGet { from, satnum } => {
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(taradr, &source)?;
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, &source, &sat)?;
                sat_add(ctx, taradr, &sat)?;
                Ok(())
            }
            Self::DirectedDiaPay { to, diamonds } => {
                check_direct_pay_form(to)?;
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(taradr, &target)?;
                diamonds_transfer(ctx, taradr, &target, diamonds)?;
                Ok(())
            }
            Self::DirectedDiaGet { from, diamonds } => {
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(taradr, &source)?;
                diamonds_transfer(ctx, &source, taradr, diamonds)?;
                Ok(())
            }
            Self::DirectedAssetPay { to, asset } => {
                check_direct_pay_form(to)?;
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(taradr, &target)?;
                tex_check_asset_serial(ctx, asset.serial)?;
                let mut state = CoreState::wrap(ctx.layer());
                asset_sub(&mut state, taradr, asset)?;
                asset_add(&mut state, &target, asset)?;
                Ok(())
            }
            Self::DirectedAssetGet { from, asset } => {
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(taradr, &source)?;
                tex_check_asset_serial(ctx, asset.serial)?;
                let mut state = CoreState::wrap(ctx.layer());
                asset_sub(&mut state, &source, asset)?;
                asset_add(&mut state, taradr, asset)?;
                Ok(())
            }
            Self::CondZhuAtMost { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu >= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuAtLeast { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu <= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuEq { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu == bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondSatAtMost { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() >= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatAtLeast { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() <= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatEq { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() == sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondDiaActualAtMost { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() >= dia {
                    Ok(())
                } else {
                    errf!("cell condition actual dia check failed")
                }
            }
            Self::CondDiaActualAtLeast { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() <= dia {
                    Ok(())
                } else {
                    errf!("cell condition actual dia check failed")
                }
            }
            Self::CondDiaActualEq { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() == dia {
                    Ok(())
                } else {
                    errf!("cell condition actual dia check failed")
                }
            }
            Self::CondDiaProjectedAtMost { diamond } => {
                let dia = projected_diamond_count(ctx, taradr)?;
                if (diamond.uint() as i128) >= dia {
                    Ok(())
                } else {
                    errf!("cell condition projected dia check failed")
                }
            }
            Self::CondDiaProjectedAtLeast { diamond } => {
                let dia = projected_diamond_count(ctx, taradr)?;
                if (diamond.uint() as i128) <= dia {
                    Ok(())
                } else {
                    errf!("cell condition projected dia check failed")
                }
            }
            Self::CondDiaProjectedEq { diamond } => {
                let dia = projected_diamond_count(ctx, taradr)?;
                if (diamond.uint() as i128) == dia {
                    Ok(())
                } else {
                    errf!("cell condition projected dia check failed")
                }
            }
            Self::CondAssetAtMost { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() >= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetAtLeast { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() <= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetEq { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() == ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondHeightAtMost { height } => {
                if height.uint() >= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondHeightAtLeast { height } => {
                if height.uint() <= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondChainIdEq { chainid } => {
                if ctx.env().chain.id.get() == chainid.uint() {
                    Ok(())
                } else {
                    errf!("cell condition chain id check failed")
                }
            }
        }
    }
}

base::impl_action_execute! {
    TexCellExecute {
        (self, ctx) {
            if ctx.exec_from() != ExecFrom::Top {
                return errf!(
                    "TexCellExecute can only run in TOP context, got {}",
                    ctx.exec_from()
                );
            }
            self.addr.must_privkey()?;
            // Cached per transaction (same-tx replays must not re-run ECDSA)
            // and skipped under fast sync, like every other signature check.
            let thx = self.get_sign_stuff();
            ctx.check_bundle_sign(&thx, &self.addr, &self.sign)?;
            for cell in &self.cells {
                cell.execute(ctx, &self.addr)?;
            }
            Ok(vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{
        ActionDispatcher, ActionExecute, ActionRef, BinaryCodecs, BlockHasherFn, BlockRef,
        DIAMOND_STATUS_NORMAL, Env, ExecutionServices, JsonCodecs, StateLayer, StateRead,
        TexLedger, Transaction, TxInfo, TxRef, Vm, VmExecutionParams, VmHostActionDef,
        VmHostCallKind,
    };
    use field::{
        AssetAmt, AssetAmtW1, AssetSmelt, Balance, BlockHeight, BytesW1, DiamondName,
        DiamondNameListMax200, DiamondNumber, DiamondNumberAuto, DiamondOwnedForm, DiamondSto,
        Inscripts, SatoshiAuto, Uint1, Uint8, UNIT_ZHU,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::action_std::TransferHacdTo;
    use crate::codec::tx::StdTransaction;

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

    /// Codec stubs; only `execution_profile` is live so `execution_params`
    /// (e.g. the diamond-form flag) resolves like mainnet.
    #[derive(Clone)]
    struct DirectedServices;

    fn stub_hasher(_height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }

    impl BinaryCodecs for DirectedServices {
        fn decode_action(&self, _buf: &[u8]) -> Ret<(base::ActionRef, usize)> {
            errf!("stub: decode_action")
        }
        fn decode_transaction(&self, _buf: &[u8]) -> Ret<(TxRef, usize)> {
            errf!("stub: decode_transaction")
        }
        fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
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

    impl JsonCodecs for DirectedServices {
        fn decode_action_json(&self, _json: &str) -> Ret<base::ActionRef> {
            errf!("stub: decode_action_json")
        }
    }

    impl ExecutionServices for DirectedServices {
        fn assign_vm(&self, _height: u64) -> Option<Box<dyn Vm>> {
            None
        }
        fn vm_host_def(&self, _kind: VmHostCallKind, _id: u8) -> Option<&VmHostActionDef> {
            None
        }
        fn vm_params(&self) -> Ret<&VmExecutionParams> {
            errf!("stub: vm_params")
        }
        fn execution_profile(&self) -> Ret<&'static dyn base::ExecutionProfile> {
            Ok(&hacash_params::MAINNET_PARAMS)
        }
        fn create_context(
            self: Arc<Self>,
            _env: Env,
            _chunk: base::StateChunkRef,
            _tx: TxRef,
        ) -> Ret<Box<dyn Context>> {
            errf!("stub: create_context")
        }
    }

    struct DirectedCtx {
        env: Env,
        tx: TxRef,
        layer: MemLayer,
        exec_from: ExecFrom,
        tex: TexLedger,
    }

    impl DirectedCtx {
        /// Mainnet-shaped context for a wire-real type-3 tx: `env.tx` is
        /// populated exactly like the block apply loop does, so `AddrOrPtr`
        /// resolution and the dispatch-level signature consent (`req_sign` +
        /// `check_sign`) run against the real transaction address list.
        fn new(tx: StdTransaction) -> Self {
            let mut env = Env::default();
            env.chain.consensus_flags =
                hacash_params::MAINNET_PARAMS.protocol.diamond_form_flag;
            env.block.height = 1;
            env.tx = TxInfo {
                ty: tx.ty(),
                main: tx.main(),
                addrs: tx.addrs(),
                fee: tx.fee().clone(),
            };
            Self {
                env,
                tx: Arc::new(tx),
                layer: MemLayer::default(),
                exec_from: ExecFrom::Top,
                tex: TexLedger::default(),
            }
        }
    }

    impl Context for DirectedCtx {
        fn services(&self) -> Arc<dyn ExecutionServices> {
            Arc::new(DirectedServices)
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
        /// Real per-signer verification against this tx's signatures, mirroring
        /// the production context in strict mode (no fast-sync shortcut), so the
        /// dispatcher's req_sign consent check behaves like consensus.
        fn check_sign(&mut self, adr: &Address) -> Rerr {
            crate::tx_std::verify_target_signature(adr, self.tx.as_ref()).map(|_| ())
        }
        fn layer(&mut self) -> &mut dyn StateLayer {
            &mut self.layer
        }
        fn emit_log(&mut self, _entry: base::LogEntry) {}
        fn gas_remaining(&self) -> i64 {
            i64::MAX
        }
        fn gas_charge(&mut self, _gas: i64) -> Rerr {
            Ok(())
        }
        fn gas_rebate(&mut self, _gas: i64) -> Rerr {
            Ok(())
        }
        fn gas_initialize(&mut self, _budget: i64) -> Rerr {
            Ok(())
        }
        fn gas_refund(&mut self) -> Rerr {
            Ok(())
        }
        fn snapshot_volatile(&self) -> Box<dyn std::any::Any> {
            Box::new(())
        }
        fn restore_volatile(&mut self, _snap: Box<dyn std::any::Any>) {}
        fn action_call(&mut self, _kind: u16, _body: Vec<u8>) -> Ret<base::ActOut> {
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
        fn tex_ledger_mut_top(&mut self) -> Ret<&mut TexLedger> {
            if self.exec_from != ExecFrom::Top {
                return errf!(
                    "tex ledger write only allowed in TOP context, got {}",
                    self.exec_from
                );
            }
            Ok(&mut self.tex)
        }
        fn p2sh_set(&mut self, _addr: Address, _p2sh: Box<dyn base::P2sh>) -> Rerr {
            errf!("stub: p2sh_set")
        }
    }

    fn hacd_name(i: usize) -> DiamondName {
        const ALPHA: &[u8; 16] = b"WTYUIAHXVMEKBSZN";
        let mut raw = [b'W'; 6];
        let mut n = i;
        for slot in (0..6).rev() {
            raw[slot] = ALPHA[n % 16];
            n /= 16;
        }
        DiamondName::from(raw)
    }

    /// Seed `owner` with `names` (diamond record + owned list) and the
    /// matching balance count.
    fn seed_diamonds(ctx: &mut DirectedCtx, owner: &Address, names: &[DiamondName]) {
        for name in names {
            CoreState::wrap(ctx.layer()).diamond_set(
                name,
                &DiamondSto {
                    status: DIAMOND_STATUS_NORMAL,
                    address: *owner,
                    prev_engraved_height: BlockHeight::from(0),
                    inscripts: Inscripts::default(),
                },
            );
        }
        let mut owned = DiamondOwnedForm::default();
        for name in names {
            owned.push_one(name).unwrap();
        }
        CoreState::wrap(ctx.layer()).diamond_owned_set(owner, &owned);
        let balance = Balance {
            diamond: DiamondNumberAuto::from_diamond(&DiamondNumber::from(names.len() as u32)),
            ..Default::default()
        };
        CoreState::wrap(ctx.layer()).balance_set(owner, &balance);
    }

    fn hac_balance(ctx: &mut DirectedCtx, addr: &Address) -> Amount {
        CoreState::wrap(ctx.layer())
            .balance(addr)
            .unwrap()
            .map(|b| b.hacash)
            .unwrap_or_default()
    }

    /// Type-3 tx: `main` signs the envelope (its address list is the signer
    /// set), the TEX bundle is separately signed by `authorizer`.
    fn directed_tx(
        main: &sys::Account,
        addrlist: Vec<Address>,
        cells: Vec<TexCell>,
        authorizer: &sys::Account,
    ) -> StdTransaction {
        let mut tx = StdTransaction::new_by(
            hacash_params::TX_TYPE_3,
            addrlist[0],
            Amount::from("1:244").unwrap(),
            1_700_000_000,
        );
        tx.addrlist = field::AddrOrList::from_list(addrlist).unwrap();
        let action =
            TexCellExecute::create_signed(Address::from(*authorizer.address()), cells, authorizer)
                .unwrap();
        tx.push_action_in(Arc::new(action));
        tx.fill_sign_account(main).unwrap();
        tx
    }

    fn run_directed_cells(ctx: &mut DirectedCtx) -> Ret<()> {
        // Clone the Arc out of the tx first so the mutable context borrow does
        // not overlap the action-list borrow.
        let act = ctx.tx.actions()[0].clone();
        let act = act.as_any().downcast_ref::<TexCellExecute>().unwrap();
        ActionExecute::execute(act, ctx.as_context_mut()).map(|_| ())
    }

    fn assert_ledger_residual_zero(ctx: &DirectedCtx) {
        assert_eq!(ctx.tex.zhu, 0);
        assert_eq!(ctx.tex.sat, 0);
        assert_eq!(ctx.tex.dia, 0);
        assert!(ctx.tex.entries.is_empty());
        assert!(ctx.tex.diamonds.is_empty());
        assert!(ctx.tex.dia_recorded.is_empty());
        assert!(ctx.tex.diatrs.is_empty());
    }

    /// Post-settlement state: the pay-records are fully handed out. The
    /// `diatrs` entries and the dia counter stay as recorded — the ledger is
    /// per-tx scratch and is dropped with the context.
    fn assert_ledger_diamonds_drained(ctx: &DirectedCtx) {
        assert!(ctx.tex.diamonds.is_empty());
        assert!(ctx.tex.dia_recorded.is_empty());
    }

    #[test]
    fn directed_two_party_exchange_moves_hac_and_leaves_no_ledger_residual() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![
                TexCell::DirectedZhuPay {
                    to: AddrOrPtr::Addr(taker_addr),
                    haczhu: Fold64::from(1000).unwrap(),
                },
                TexCell::DirectedZhuGet {
                    from: AddrOrPtr::Ptr(0),
                    haczhu: Fold64::from(2000).unwrap(),
                },
            ],
            &maker,
        ));
        CoreState::wrap(ctx.layer()).balance_set(
            &maker_addr,
            &Balance::hac(Amount::coin_u128(3000, UNIT_ZHU)),
        );
        CoreState::wrap(ctx.layer()).balance_set(
            &taker_addr,
            &Balance::hac(Amount::coin_u128(5000, UNIT_ZHU)),
        );

        run_directed_cells(&mut ctx).unwrap();
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(4000, UNIT_ZHU)
        );
        assert_eq!(
            hac_balance(&mut ctx, &taker_addr),
            Amount::coin_u128(4000, UNIT_ZHU)
        );
        assert_ledger_residual_zero(&ctx);
        // Directed cells must not touch the ledger: do_settlement passes with
        // every residual zero.
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
    }

    #[test]
    fn directed_sat_and_asset_cells_move_value_between_the_parties() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let sat = |n: u64| Fold64::from(n).unwrap();
        let asset = |n: u64| AssetAmt {
            serial: sat(11),
            amount: sat(n),
        };
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![
                TexCell::DirectedSatPay {
                    to: AddrOrPtr::Addr(taker_addr),
                    satnum: sat(100),
                },
                TexCell::DirectedSatGet {
                    from: AddrOrPtr::Ptr(0),
                    satnum: sat(50),
                },
                TexCell::DirectedAssetPay {
                    to: AddrOrPtr::Addr(taker_addr),
                    asset: asset(5),
                },
                TexCell::DirectedAssetGet {
                    from: AddrOrPtr::Ptr(0),
                    asset: asset(2),
                },
            ],
            &maker,
        ));
        let maker_balance = Balance {
            satoshi: SatoshiAuto::from_satoshi(&Uint8::from(1000)).unwrap(),
            assets: AssetAmtW1::from(vec![asset(10)]).unwrap(),
            ..Default::default()
        };
        let taker_balance = Balance {
            satoshi: SatoshiAuto::from_satoshi(&Uint8::from(500)).unwrap(),
            assets: AssetAmtW1::from(vec![asset(1)]).unwrap(),
            ..Default::default()
        };
        CoreState::wrap(ctx.layer()).balance_set(&maker_addr, &maker_balance);
        CoreState::wrap(ctx.layer()).balance_set(&taker_addr, &taker_balance);
        // Directed asset cells verify the serial like the escrow cells, so the
        // #11 record must exist in state.
        CoreState::wrap(ctx.layer()).asset_set(
            &sat(11),
            &AssetSmelt {
                serial: sat(11),
                supply: sat(1_000_000),
                decimal: Uint1::from(8),
                issuer: maker_addr,
                ticket: BytesW1::default(),
                name: BytesW1::default(),
            },
        );

        run_directed_cells(&mut ctx).unwrap();
        let maker_after = CoreState::wrap(ctx.layer())
            .balance(&maker_addr)
            .unwrap()
            .unwrap();
        let taker_after = CoreState::wrap(ctx.layer())
            .balance(&taker_addr)
            .unwrap()
            .unwrap();
        // SAT: pay 100 then get 50; Asset #11: pay 5 then get 2.
        assert_eq!(maker_after.satoshi.uint(), 950);
        assert_eq!(taker_after.satoshi.uint(), 550);
        assert_eq!(
            maker_after
                .assets
                .as_list()
                .iter()
                .find(|a| a.serial.uint() == 11)
                .unwrap()
                .amount
                .uint(),
            7
        );
        assert_eq!(
            taker_after
                .assets
                .as_list()
                .iter()
                .find(|a| a.serial.uint() == 11)
                .unwrap()
                .amount
                .uint(),
            4
        );
        assert_ledger_residual_zero(&ctx);
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
    }

    /// Directed asset cells check the serial like the escrow cells: a zero or
    /// unknown serial is rejected before any balance moves, and the ledger is
    /// left untouched.
    #[test]
    fn directed_asset_cells_check_asset_serial_like_escrow() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let taker_addr = Address::from(*taker.address());
        let asset = |serial: u64| AssetAmt {
            serial: Fold64::from(serial).unwrap(),
            amount: Fold64::from(1).unwrap(),
        };

        // Serial 999 has no asset record in state, on both directions. No
        // balances are seeded: the serial check fires before the balance move,
        // so the message itself proves the check ran.
        for cells in [
            vec![TexCell::DirectedAssetPay {
                to: AddrOrPtr::Addr(taker_addr),
                asset: asset(999),
            }],
            vec![TexCell::DirectedAssetGet {
                from: AddrOrPtr::Addr(taker_addr),
                asset: asset(999),
            }],
        ] {
            let mut ctx = DirectedCtx::new(directed_tx(&taker, vec![taker_addr], cells, &maker));
            let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
            assert!(err.contains("does not exist"), "{err}");
            assert!(ctx.tex.entries.is_empty());
        }

        // Zero serial.
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![TexCell::DirectedAssetPay {
                to: AddrOrPtr::Addr(taker_addr),
                asset: asset(0),
            }],
            &maker,
        ));
        let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("cannot be zero"), "{err}");
        assert!(ctx.tex.entries.is_empty());
    }

    #[test]
    fn directed_diamond_cells_move_ownership_between_the_parties() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let pay_names = vec![hacd_name(1), hacd_name(2)];
        let get_names = vec![hacd_name(3)];
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![
                TexCell::DirectedDiaPay {
                    to: AddrOrPtr::Addr(taker_addr),
                    diamonds: DiamondNameListMax200::from(pay_names.clone()).unwrap(),
                },
                TexCell::DirectedDiaGet {
                    from: AddrOrPtr::Ptr(0),
                    diamonds: DiamondNameListMax200::from(get_names.clone()).unwrap(),
                },
            ],
            &maker,
        ));
        seed_diamonds(&mut ctx, &maker_addr, &pay_names);
        seed_diamonds(&mut ctx, &taker_addr, &get_names);

        run_directed_cells(&mut ctx).unwrap();
        let state = CoreState::wrap(ctx.layer());
        // Pay moved maker's named diamonds to the taker...
        for name in &pay_names {
            assert_eq!(state.diamond(name).unwrap().unwrap().address, taker_addr);
        }
        // ...and the get moved the main signer's diamonds to the maker.
        for name in &get_names {
            assert_eq!(state.diamond(name).unwrap().unwrap().address, maker_addr);
        }
        assert_eq!(state.balance(&maker_addr).unwrap().unwrap().diamond.uint(), 1);
        // Taker seeded 1 (name 3), gained 2 in the pay, sent 1 back in the get.
        assert_eq!(state.balance(&taker_addr).unwrap().unwrap().diamond.uint(), 2);
        drop(state);
        assert_ledger_residual_zero(&ctx);
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
    }

    #[test]
    fn directed_zhu_pay_to_ptr_zero_pays_the_transaction_main() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![TexCell::DirectedZhuPay {
                to: AddrOrPtr::Ptr(0),
                haczhu: Fold64::from(700).unwrap(),
            }],
            &maker,
        ));
        CoreState::wrap(ctx.layer()).balance_set(
            &maker_addr,
            &Balance::hac(Amount::coin_u128(1000, UNIT_ZHU)),
        );

        run_directed_cells(&mut ctx).unwrap();
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(300, UNIT_ZHU)
        );
        assert_eq!(
            hac_balance(&mut ctx, &taker_addr),
            Amount::coin_u128(700, UNIT_ZHU)
        );
    }

    /// Consent for DirectedGet lives at dispatch: every directed get `from` is
    /// declared in the action's `req_sign`, so a static source that did not
    /// co-sign the transaction fails the dispatcher's `ctx.check_sign` before
    /// any cell executes. There is no exec-time membership re-check.
    #[test]
    fn directed_get_rejects_a_source_that_did_not_sign_the_transaction() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let bystander = Address::from(
            *sys::Account::create_by_secret_key_value([11u8; 32])
                .unwrap()
                .address(),
        );
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(bystander),
                haczhu: Fold64::from(1000).unwrap(),
            }],
            &maker,
        ));
        CoreState::wrap(ctx.layer()).balance_set(
            &bystander,
            &Balance::hac(Amount::coin_u128(9000, UNIT_ZHU)),
        );

        let act = ctx.tx.actions()[0].clone();
        let err = ActionDispatcher::dispatch_top(ctx.as_context_mut(), &act)
            .unwrap_err()
            .to_string();
        assert!(err.contains("signature verification failed"), "{err}");
        assert_eq!(
            hac_balance(&mut ctx, &bystander),
            Amount::coin_u128(9000, UNIT_ZHU)
        );
        assert_ledger_residual_zero(&ctx);
    }

    /// A directed pay target must be pinned by the authorizer's signature:
    /// the bundle signature covers the cells, not the transaction address
    /// list, so `Ptr(k)` with k >= 1 would let the executing main redirect
    /// the authorizer's payment to any address placed at that slot. Only
    /// `Ptr(0)` (the main the signer's intent names) and explicit static
    /// addresses are accepted.
    #[test]
    fn directed_pay_rejects_free_pointer_targets() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let taker_addr = Address::from(*taker.address());
        let cases = vec![
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Ptr(1),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedSatPay {
                to: AddrOrPtr::Ptr(2),
                satnum: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedDiaPay {
                to: AddrOrPtr::Ptr(1),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedAssetPay {
                to: AddrOrPtr::Ptr(3),
                asset: AssetAmt {
                    serial: Fold64::from(11).unwrap(),
                    amount: Fold64::from(5).unwrap(),
                },
            },
        ];
        for cells in cases.into_iter().map(|c| vec![c]) {
            let mut ctx = DirectedCtx::new(directed_tx(&taker, vec![taker_addr], cells, &maker));
            let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
            assert!(err.contains("not a free pointer"), "{err}");
        }
    }

    #[test]
    fn directed_pay_rejects_the_settlement_and_system_addresses_and_bad_ptrs() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let taker_addr = Address::from(*taker.address());
        let settlement = crate::params::SETTLEMENT_ADDR;
        let cases = vec![
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Addr(settlement),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedSatPay {
                to: AddrOrPtr::Addr(settlement),
                satnum: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedDiaPay {
                to: AddrOrPtr::Addr(settlement),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedAssetPay {
                to: AddrOrPtr::Addr(settlement),
                asset: AssetAmt {
                    serial: Fold64::from(11).unwrap(),
                    amount: Fold64::from(5).unwrap(),
                },
            },
            // The blackhole and every other unknown-privkey address are system
            // addresses for directed moves, mirroring the diamond recipient guard.
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Addr(Address::from([0u8; 21])),
                haczhu: Fold64::from(1000).unwrap(),
            },
            // A ptr beyond the transaction address list cannot resolve.
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Ptr(5),
                haczhu: Fold64::from(1000).unwrap(),
            },
        ];
        for cells in cases.into_iter().map(|c| vec![c]) {
            let mut ctx = DirectedCtx::new(directed_tx(&taker, vec![taker_addr], cells, &maker));
            let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
            assert!(
                err.contains("settlement")
                    || err.contains("system address")
                    || err.contains("out of range")
                    || err.contains("not a free pointer"),
                "{err}"
            );
        }
    }

    /// Directed endpoints must be ordinary privkey user addresses. Multisig
    /// and contract addresses (non-privkey versions) are rejected on both the
    /// pay target and the get source: directed cells move balances through
    /// `hac_sub`/`hac_add` without the transfer hook machinery, so touching a
    /// contract or script address would skip its authorize/receive hooks, and
    /// the dispatcher's signature consent does not apply to non-privkey
    /// sources (a directed get from a multisig balance would have no consent
    /// at all).
    #[test]
    fn directed_endpoints_reject_multisig_and_contract_addresses() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let taker_addr = Address::from(*taker.address());
        // Version bytes 1 (contract) and 5 (scriptmh/multisig family).
        let contract_addr = Address::from([
            1, 0x11, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let script_addr = Address::from([
            5, 0x22, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let cases = vec![
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Addr(contract_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Addr(script_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(contract_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(script_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedDiaPay {
                to: AddrOrPtr::Addr(contract_addr),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedAssetGet {
                from: AddrOrPtr::Addr(script_addr),
                asset: AssetAmt {
                    serial: Fold64::from(11).unwrap(),
                    amount: Fold64::from(5).unwrap(),
                },
            },
        ];
        for cells in cases.into_iter().map(|c| vec![c]) {
            let mut ctx = DirectedCtx::new(directed_tx(&taker, vec![taker_addr], cells, &maker));
            let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
            assert!(err.contains("must be PRIVAKEY type"), "{err}");
        }
    }

    /// The authorizer itself is not a valid directed endpoint: all eight
    /// directed cells reject a self-transfer uniformly at the guard (before,
    /// Zhu/Sat/Asset cells silently no-oped while the diamond move rejected
    /// with "cannot transfer to self"). `run_directed_cells` executes the
    /// action directly, so this exercises the exec-level guard itself.
    #[test]
    fn directed_cells_reject_self_transfers() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let asset = |n: u64| AssetAmt {
            serial: Fold64::from(11).unwrap(),
            amount: Fold64::from(n).unwrap(),
        };
        let cases = vec![
            TexCell::DirectedZhuPay {
                to: AddrOrPtr::Addr(maker_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(maker_addr),
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedSatPay {
                to: AddrOrPtr::Addr(maker_addr),
                satnum: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedSatGet {
                from: AddrOrPtr::Addr(maker_addr),
                satnum: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedDiaPay {
                to: AddrOrPtr::Addr(maker_addr),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedDiaGet {
                from: AddrOrPtr::Addr(maker_addr),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedAssetPay {
                to: AddrOrPtr::Addr(maker_addr),
                asset: asset(5),
            },
            TexCell::DirectedAssetGet {
                from: AddrOrPtr::Addr(maker_addr),
                asset: asset(5),
            },
        ];
        for cells in cases.into_iter().map(|c| vec![c]) {
            let mut ctx = DirectedCtx::new(directed_tx(&taker, vec![taker_addr], cells, &maker));
            let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
            assert!(err.contains("authorizer"), "{err}");
        }

        // A Ptr endpoint that resolves to the authorizer (index 1 of the tx
        // address list) is caught the same way.
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr, maker_addr],
            vec![TexCell::DirectedZhuGet {
                from: AddrOrPtr::Ptr(1),
                haczhu: Fold64::from(1000).unwrap(),
            }],
            &maker,
        ));
        CoreState::wrap(ctx.layer()).balance_set(
            &maker_addr,
            &Balance::hac(Amount::coin_u128(1000, UNIT_ZHU)),
        );
        let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("authorizer"), "{err}");
        // The guard fires before any balance moves.
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(1000, UNIT_ZHU)
        );
    }

    /// DirectedGet from the settlement address is rejected by the direct
    /// endpoint guard: consent moved to the dispatcher (`req_sign` +
    /// `check_sign`), so the rule that remains at exec time is the
    /// system-address guard — which also holds under fast sync, where
    /// `check_sign` is skipped and this guard is the last line of defense.
    #[test]
    fn directed_get_rejects_the_settlement_address_as_source() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let taker_addr = Address::from(*taker.address());
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            vec![TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(crate::params::SETTLEMENT_ADDR),
                haczhu: Fold64::from(1000).unwrap(),
            }],
            &maker,
        ));
        let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
        assert!(
            err.contains("cannot involve the tex settlement address"),
            "{err}"
        );
    }

    /// A co-signer source: `from: Ptr(1)` resolves to the second entry of the
    /// transaction address list, which co-signed the tx, so the get succeeds.
    #[test]
    fn directed_get_accepts_a_cosigner_source_resolved_by_ptr() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let cosigner = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let cosigner_addr = Address::from(*cosigner.address());
        let mut tx = directed_tx(
            &taker,
            vec![taker_addr, cosigner_addr],
            vec![TexCell::DirectedZhuGet {
                from: AddrOrPtr::Ptr(1),
                haczhu: Fold64::from(300).unwrap(),
            }],
            &maker,
        );
        tx.fill_sign_account(&cosigner).unwrap();
        let mut ctx = DirectedCtx::new(tx);
        CoreState::wrap(ctx.layer()).balance_set(
            &cosigner_addr,
            &Balance::hac(Amount::coin_u128(500, UNIT_ZHU)),
        );

        run_directed_cells(&mut ctx).unwrap();
        assert_eq!(
            hac_balance(&mut ctx, &cosigner_addr),
            Amount::coin_u128(200, UNIT_ZHU)
        );
        // The get pays its authorizer (the maker), not the tx main.
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(300, UNIT_ZHU)
        );
        assert_eq!(hac_balance(&mut ctx, &taker_addr), Amount::default());
    }

    /// A static third-party source is legal when it co-signs every executing
    /// transaction: the address joins the tx address list AND signs the
    /// envelope, so the dispatcher's req_sign `check_sign` passes and the get
    /// moves its funds (per-execution consent; replay needs a fresh signature).
    #[test]
    fn directed_get_accepts_a_static_source_that_cosigns_the_transaction() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let source = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let source_addr = Address::from(*source.address());
        let mut tx = directed_tx(
            &taker,
            vec![taker_addr, source_addr],
            vec![TexCell::DirectedZhuGet {
                from: AddrOrPtr::Addr(source_addr),
                haczhu: Fold64::from(300).unwrap(),
            }],
            &maker,
        );
        tx.fill_sign_account(&source).unwrap();
        let mut ctx = DirectedCtx::new(tx);
        CoreState::wrap(ctx.layer()).balance_set(
            &source_addr,
            &Balance::hac(Amount::coin_u128(500, UNIT_ZHU)),
        );

        let act = ctx.tx.actions()[0].clone();
        ActionDispatcher::dispatch_top(ctx.as_context_mut(), &act).unwrap();
        assert_eq!(
            hac_balance(&mut ctx, &source_addr),
            Amount::coin_u128(200, UNIT_ZHU)
        );
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(300, UNIT_ZHU)
        );
        assert_eq!(hac_balance(&mut ctx, &taker_addr), Amount::default());
        assert_ledger_residual_zero(&ctx);
    }

    // ======================== deferred HACD clearing ========================

    /// Type-3 tx carrying arbitrary pre-built actions (several TEX bundles
    /// signed by different authorizers, ordinary transfers, ...), all executed
    /// by `main`.
    fn typed_tx(
        main: &sys::Account,
        addrlist: Vec<Address>,
        actions: Vec<ActionRef>,
    ) -> StdTransaction {
        let mut tx = StdTransaction::new_by(
            hacash_params::TX_TYPE_3,
            addrlist[0],
            Amount::from("1:244").unwrap(),
            1_700_000_000,
        );
        tx.addrlist = field::AddrOrList::from_list(addrlist).unwrap();
        for act in actions {
            tx.push_action_in(act);
        }
        tx.fill_sign_account(main).unwrap();
        tx
    }

    fn tex_bundle(
        authorizer: &sys::Account,
        cells: Vec<TexCell>,
    ) -> ActionRef {
        Arc::new(
            TexCellExecute::create_signed(Address::from(*authorizer.address()), cells, authorizer)
                .unwrap(),
        ) as ActionRef
    }

    fn run_all_actions(ctx: &mut DirectedCtx) -> Ret<()> {
        let actions = ctx.tx.actions().to_vec();
        for act in &actions {
            ActionDispatcher::dispatch_top(ctx.as_context_mut(), act).map(|_| ())?;
        }
        Ok(())
    }

    fn dia_owner(ctx: &mut DirectedCtx, name: &DiamondName) -> Address {
        CoreState::wrap(ctx.layer())
            .diamond(name)
            .unwrap()
            .unwrap()
            .address
    }

    fn dia_balance(ctx: &mut DirectedCtx, addr: &Address) -> u64 {
        CoreState::wrap(ctx.layer())
            .balance(addr)
            .unwrap()
            .map(|b| b.diamond.uint())
            .unwrap_or(0)
    }

    /// Deferred clearing happy path: the DiaPay only records (payer, names)
    /// and the payer keeps its diamonds until settlement, which then delivers
    /// them straight from the payer to the recipient — the settlement address
    /// never holds them.
    #[test]
    fn deferred_dia_pay_delivers_diamonds_directly_at_settlement() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let names = vec![hacd_name(1), hacd_name(2)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &payer,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names.clone()).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(2u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &payer_addr, &names);

        run_all_actions(&mut ctx).unwrap();
        // Deferred: nothing has moved yet — the payer still owns everything.
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), payer_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 2);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 0);

        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), recipient_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 2);
        assert_eq!(
            dia_balance(&mut ctx, &crate::params::SETTLEMENT_ADDR),
            0
        );
        assert_ledger_diamonds_drained(&ctx);
    }

    /// FIFO across multiple payers: one DiaGet whose count spans two DiaPay
    /// records is delivered in pay order, each segment transferred by its
    /// original payer — a wrong grouping would fail the ownership re-verify.
    #[test]
    fn deferred_settlement_walks_pay_records_fifo_across_payers() {
        let payer1 = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let payer2 = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer1_addr = Address::from(*payer1.address());
        let payer2_addr = Address::from(*payer2.address());
        let recipient_addr = Address::from(*recipient.address());
        let names1 = vec![hacd_name(1), hacd_name(2)];
        let names2 = vec![hacd_name(3)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &payer1,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names1.clone()).unwrap(),
                    }],
                ),
                tex_bundle(
                    &payer2,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names2.clone()).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(3u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &payer1_addr, &names1);
        seed_diamonds(&mut ctx, &payer2_addr, &names2);

        run_all_actions(&mut ctx).unwrap();
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        for name in names1.iter().chain(names2.iter()) {
            assert_eq!(dia_owner(&mut ctx, name), recipient_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer1_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &payer2_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 3);
        assert_ledger_diamonds_drained(&ctx);
    }

    /// Σ pays ≠ Σ gets fails settlement. In strict mode the `dia` residual
    /// catches both directions; under fast sync the residual check is skipped
    /// but the FIFO walk still fails on a short pool and still delivers on a
    /// long one (the leftover records stay in the ledger, as before).
    #[test]
    fn deferred_settlement_fails_when_pays_and_gets_do_not_match() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let names = vec![hacd_name(1), hacd_name(2)];
        let pay_get = |pays: usize, gets: u32| {
            typed_tx(
                &recipient,
                vec![recipient_addr],
                vec![
                    tex_bundle(
                        &payer,
                        vec![TexCell::DiaPay {
                            diamonds: DiamondNameListMax200::from(
                                names.iter().take(pays).cloned().collect::<Vec<_>>(),
                            )
                            .unwrap(),
                        }],
                    ),
                    tex_bundle(
                        &recipient,
                        vec![TexCell::DiaGet {
                            dianum: DiamondNumber::from(gets),
                        }],
                    ),
                ],
            )
        };

        // Pays > gets, strict mode: the nonzero dia residual fails first.
        let mut ctx = DirectedCtx::new(pay_get(2, 1));
        seed_diamonds(&mut ctx, &payer_addr, &names);
        run_all_actions(&mut ctx).unwrap();
        let err = crate::exec::tex::do_settlement(&mut ctx)
            .unwrap_err()
            .to_string();
        assert!(err.contains("coin settlement check failed"), "{err}");

        // Pays < gets, fast sync: the residual check is skipped, the walk is not.
        let mut ctx = DirectedCtx::new(pay_get(1, 2));
        ctx.env.chain.fast_sync = true;
        seed_diamonds(&mut ctx, &payer_addr, &names);
        run_all_actions(&mut ctx).unwrap();
        let err = crate::exec::tex::do_settlement(&mut ctx)
            .unwrap_err()
            .to_string();
        assert!(err.contains("diamonds settlement check failed"), "{err}");

        // Pays > gets, fast sync: delivery runs and the leftover stays recorded.
        let mut ctx = DirectedCtx::new(pay_get(2, 1));
        ctx.env.chain.fast_sync = true;
        seed_diamonds(&mut ctx, &payer_addr, &names);
        run_all_actions(&mut ctx).unwrap();
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        assert_eq!(dia_owner(&mut ctx, &names[0]), recipient_addr);
        assert_eq!(dia_owner(&mut ctx, &names[1]), payer_addr);
        assert_eq!(ctx.tex.diamonds.len(), 1);
    }

    /// Record-time duplicate rejection: with deferred clearing nothing moves,
    /// so the ownership check alone would let overlapping DiaPays through —
    /// the explicit dedup must reject the second pay immediately.
    #[test]
    fn deferred_dia_pay_rejects_a_name_already_recorded_in_the_tx() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let names = vec![hacd_name(1), hacd_name(2), hacd_name(3)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &payer,
            vec![payer_addr],
            vec![tex_bundle(
                &payer,
                vec![
                    TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(vec![names[0], names[1]]).unwrap(),
                    },
                    TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(vec![names[0], names[2]]).unwrap(),
                    },
                ],
            )],
        ));
        seed_diamonds(&mut ctx, &payer_addr, &names);

        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("already recorded for settlement"), "{err}");
        // Only the first pay's records are in the ledger.
        assert_eq!(ctx.tex.diamonds.len(), 2);
        assert_eq!(ctx.tex.diamonds[0], (payer_addr, names[0]));
        assert_eq!(ctx.tex.diamonds[1], (payer_addr, names[1]));
    }

    /// Mid-tx movement of recorded diamonds reverts settlement: after the
    /// DiaPay records the names, a legitimate main-signed transfer moves them
    /// away; the ownership re-verify in do_settlement fails and the caller
    /// discards the whole transaction's state changes.
    #[test]
    fn deferred_settlement_reverts_when_recorded_diamonds_move_mid_tx() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let other = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let other_addr = Address::from(*other.address());
        let names = vec![hacd_name(1), hacd_name(2)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &payer,
            vec![payer_addr],
            vec![
                tex_bundle(
                    &payer,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names.clone()).unwrap(),
                    }],
                ),
                Arc::new(TransferHacdTo::new(
                    other_addr,
                    DiamondNameListMax200::from(names.clone()).unwrap(),
                )) as ActionRef,
                tex_bundle(
                    &other,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(2u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &payer_addr, &names);
        let snapshot = ctx.layer.0.clone();

        // Both actions are legitimate on their own: the payer still owns the
        // diamonds when the transfer executes.
        run_all_actions(&mut ctx).unwrap();
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), other_addr);
        }
        let err = crate::exec::tex::do_settlement(&mut ctx)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not belong to address"), "{err}");

        // The block apply discards a failed tx wholesale; mirror that here.
        ctx.layer.0 = snapshot;
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), payer_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 2);
        assert_eq!(dia_balance(&mut ctx, &other_addr), 0);
    }

    /// The DiaPay ownership check runs at record time: committing diamonds the
    /// authorizer does not own (or that do not exist) fails the cell
    /// immediately and leaves the ledger untouched.
    #[test]
    fn deferred_dia_pay_rejects_diamonds_the_authorizer_does_not_own() {
        let owner = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let owner_addr = Address::from(*owner.address());
        let payer_addr = Address::from(*payer.address());
        let owned = hacd_name(1);
        let missing = hacd_name(2);
        let pay_tx = |diamonds: DiamondNameListMax200| {
            typed_tx(
                &payer,
                vec![payer_addr],
                vec![tex_bundle(
                    &payer,
                    vec![TexCell::DiaPay { diamonds }],
                )],
            )
        };

        // The diamond exists but belongs to someone else.
        let mut ctx = DirectedCtx::new(pay_tx(DiamondNameListMax200::from(vec![owned]).unwrap()));
        seed_diamonds(&mut ctx, &owner_addr, &[owned]);
        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("does not belong to address"), "{err}");

        // The diamond does not exist at all.
        let mut ctx = DirectedCtx::new(pay_tx(DiamondNameListMax200::from(vec![missing]).unwrap()));
        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("not found"), "{err}");

        assert!(ctx.tex.diamonds.is_empty());
        assert_eq!(ctx.tex.dia, 0);
    }

    /// Directed diamond cells and the deferred pool coexist: the directed move
    /// delivers directly while the pool path defers to settlement. But a
    /// directed move of a name a DiaPay recorded trips the ownership
    /// re-verify at settlement and reverts the transaction.
    #[test]
    fn directed_and_pool_dia_cells_coexist_and_conflicts_revert() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let payer2 = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let payer2_addr = Address::from(*payer2.address());
        let recipient_addr = Address::from(*recipient.address());
        let directed_name = hacd_name(4);
        let pool_name = hacd_name(5);
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &maker,
                    vec![TexCell::DirectedDiaPay {
                        to: AddrOrPtr::Addr(recipient_addr),
                        diamonds: DiamondNameListMax200::from(vec![directed_name]).unwrap(),
                    }],
                ),
                tex_bundle(
                    &payer2,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(vec![pool_name]).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(1u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &maker_addr, &[directed_name]);
        seed_diamonds(&mut ctx, &payer2_addr, &[pool_name]);

        run_all_actions(&mut ctx).unwrap();
        // The directed move already delivered; the pool record is still parked.
        assert_eq!(dia_owner(&mut ctx, &directed_name), recipient_addr);
        assert_eq!(dia_owner(&mut ctx, &pool_name), payer2_addr);
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        assert_eq!(dia_owner(&mut ctx, &pool_name), recipient_addr);
        assert_eq!(dia_balance(&mut ctx, &payer2_addr), 0);
        assert_ledger_diamonds_drained(&ctx);

        // Conflict: the pool records the name, then a directed move takes it.
        let pool_name = hacd_name(6);
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &maker,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(vec![pool_name]).unwrap(),
                    }],
                ),
                tex_bundle(
                    &maker,
                    vec![TexCell::DirectedDiaPay {
                        to: AddrOrPtr::Addr(recipient_addr),
                        diamonds: DiamondNameListMax200::from(vec![pool_name]).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(1u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &maker_addr, &[pool_name]);
        run_all_actions(&mut ctx).unwrap();
        let err = crate::exec::tex::do_settlement(&mut ctx)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not belong to address"), "{err}");
    }

    /// Fast sync skips the residual checks but the deferred distribution must
    /// still run and still deliver the diamonds payer-to-recipient.
    #[test]
    fn deferred_settlement_distribution_runs_under_fast_sync() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let names = vec![hacd_name(1), hacd_name(2)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &payer,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names.clone()).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(2u32),
                    }],
                ),
            ],
        ));
        ctx.env.chain.fast_sync = true;
        seed_diamonds(&mut ctx, &payer_addr, &names);

        run_all_actions(&mut ctx).unwrap();
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), recipient_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 2);
        assert_ledger_diamonds_drained(&ctx);
    }

    /// Actual HACD conditions read the settled `Balance.diamond` count, while
    /// the projected variants account for the pending `DiaPay` records. With a
    /// pay of 2 pending and 3 settled names, actual is 3 and projected is 1 —
    /// both conditions can be asserted in one bundle and genuinely disagree.
    #[test]
    fn dia_pay_actual_condition_ignores_pending_pay_while_projected_deducts_it() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let names = vec![hacd_name(1), hacd_name(2), hacd_name(3)];
        let bundle = |extra: Vec<TexCell>| {
            let mut cells = vec![TexCell::DiaPay {
                diamonds: DiamondNameListMax200::from(names[..2].to_vec()).unwrap(),
            }];
            cells.extend(extra);
            typed_tx(
                &recipient,
                vec![recipient_addr],
                vec![tex_bundle(&payer, cells)],
            )
        };

        // Actual still sees the pre-settlement 3; projected is 3 - 2 = 1.
        let mut ctx = DirectedCtx::new(bundle(vec![
            TexCell::CondDiaActualEq {
                diamond: Fold64::from(3).unwrap(),
            },
            TexCell::CondDiaProjectedEq {
                diamond: Fold64::from(1).unwrap(),
            },
            TexCell::CondDiaProjectedAtMost {
                diamond: Fold64::from(1).unwrap(),
            },
            TexCell::CondDiaProjectedAtLeast {
                diamond: Fold64::from(1).unwrap(),
            },
        ]));
        seed_diamonds(&mut ctx, &payer_addr, &names);
        run_all_actions(&mut ctx).unwrap();
        assert_eq!(ctx.tex.dia_pay_pending.get(&payer_addr).copied(), Some(2));
        assert!(ctx.tex.dia_get_pending.is_empty());

        // Disagreement, direction one: claiming the actual count is 1 fails.
        let mut ctx = DirectedCtx::new(bundle(vec![TexCell::CondDiaActualEq {
            diamond: Fold64::from(1).unwrap(),
        }]));
        seed_diamonds(&mut ctx, &payer_addr, &names);
        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("actual dia check failed"), "{err}");

        // Disagreement, direction two: claiming the projected count is 3 fails.
        let mut ctx = DirectedCtx::new(bundle(vec![TexCell::CondDiaProjectedEq {
            diamond: Fold64::from(3).unwrap(),
        }]));
        seed_diamonds(&mut ctx, &payer_addr, &names);
        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("projected dia check failed"), "{err}");
    }

    /// Actual HACD conditions ignore the pending `DiaGet` receipts; the
    /// projected variants add them. The recipient settles at 1 name, books a
    /// get of 2, so actual is 1 and projected is 3 while the cells run.
    #[test]
    fn dia_get_actual_condition_ignores_pending_get_while_projected_adds_it() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let payer_names = vec![hacd_name(1), hacd_name(2)];
        let recipient_names = vec![hacd_name(3)];
        let bundle = |extra: Vec<TexCell>| {
            let mut cells = vec![TexCell::DiaGet {
                dianum: DiamondNumber::from(2u32),
            }];
            cells.extend(extra);
            typed_tx(
                &recipient,
                vec![recipient_addr],
                vec![
                    tex_bundle(
                        &payer,
                        vec![TexCell::DiaPay {
                            diamonds: DiamondNameListMax200::from(payer_names.clone()).unwrap(),
                        }],
                    ),
                    tex_bundle(&recipient, cells),
                ],
            )
        };

        let mut ctx = DirectedCtx::new(bundle(vec![
            TexCell::CondDiaActualEq {
                diamond: Fold64::from(1).unwrap(),
            },
            TexCell::CondDiaProjectedEq {
                diamond: Fold64::from(3).unwrap(),
            },
            TexCell::CondDiaProjectedAtLeast {
                diamond: Fold64::from(3).unwrap(),
            },
            TexCell::CondDiaProjectedAtMost {
                diamond: Fold64::from(3).unwrap(),
            },
        ]));
        seed_diamonds(&mut ctx, &payer_addr, &payer_names);
        seed_diamonds(&mut ctx, &recipient_addr, &recipient_names);
        run_all_actions(&mut ctx).unwrap();
        assert_eq!(ctx.tex.dia_pay_pending.get(&payer_addr).copied(), Some(2));
        assert_eq!(
            ctx.tex.dia_get_pending.get(&recipient_addr).copied(),
            Some(2)
        );

        // The pending get is only projected: the actual count is still 1.
        let mut ctx = DirectedCtx::new(bundle(vec![TexCell::CondDiaActualEq {
            diamond: Fold64::from(3).unwrap(),
        }]));
        seed_diamonds(&mut ctx, &payer_addr, &payer_names);
        seed_diamonds(&mut ctx, &recipient_addr, &recipient_names);
        let err = run_all_actions(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("actual dia check failed"), "{err}");

        // Settlement then delivers the pay records and the settled count takes
        // over; the projected addition is gone once the ledger is drained.
        let mut ctx = DirectedCtx::new(bundle(vec![]));
        seed_diamonds(&mut ctx, &payer_addr, &payer_names);
        seed_diamonds(&mut ctx, &recipient_addr, &recipient_names);
        run_all_actions(&mut ctx).unwrap();
        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        for name in &payer_names {
            assert_eq!(dia_owner(&mut ctx, name), recipient_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 3);
    }

    /// Regression guard: this feature reads the ledger but does not change
    /// settlement. A valid pay/get pair still moves every named diamond from
    /// the payer to the recipient, and the settled actual count equals the
    /// named ownership on both sides.
    #[test]
    fn settlement_still_moves_names_and_actual_count_matches_named_ownership() {
        let payer = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let recipient = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let payer_addr = Address::from(*payer.address());
        let recipient_addr = Address::from(*recipient.address());
        let names = vec![hacd_name(1), hacd_name(2)];
        let mut ctx = DirectedCtx::new(typed_tx(
            &recipient,
            vec![recipient_addr],
            vec![
                tex_bundle(
                    &payer,
                    vec![TexCell::DiaPay {
                        diamonds: DiamondNameListMax200::from(names.clone()).unwrap(),
                    }],
                ),
                tex_bundle(
                    &recipient,
                    vec![TexCell::DiaGet {
                        dianum: DiamondNumber::from(2u32),
                    }],
                ),
            ],
        ));
        seed_diamonds(&mut ctx, &payer_addr, &names);

        // Before settlement nothing has moved: actual counts are the settled ones.
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 2);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr), 0);
        run_all_actions(&mut ctx).unwrap();
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 2);
        assert_eq!(ctx.tex.dia_pay_pending.get(&payer_addr).copied(), Some(2));
        assert_eq!(
            ctx.tex.dia_get_pending.get(&recipient_addr).copied(),
            Some(2)
        );

        crate::exec::tex::do_settlement(&mut ctx).unwrap();
        for name in &names {
            assert_eq!(dia_owner(&mut ctx, name), recipient_addr);
        }
        assert_eq!(dia_balance(&mut ctx, &payer_addr), 0);
        assert_eq!(dia_balance(&mut ctx, &recipient_addr) as usize, names.len());
        assert_ledger_diamonds_drained(&ctx);
    }
}
