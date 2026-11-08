//! TexCellExecute execute body and TEX cell state changes.

use base::{
    Context, CoreState, ExecFrom, asset_add, asset_sub, hac_add, hac_sub, sat_add, sat_sub,
};
use field::{AddrOrPtr, Address, Amount, Fold64, Hash, Satoshi, Sign, ToJSON};
use sys::{Account, Rerr, Ret, errf};

use crate::codec::action::tex::{TexCell, TexCellExecute};
use crate::exec::apply::{DiamondMove, diamonds_transfer};
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

fn verify_signature(hash: &Hash, addr: &Address, sign: &Sign) -> bool {
    let got = Address::from(Account::get_address_by_public_key(
        sign.publickey.into_array(),
    ));
    got == *addr && Account::verify_signature(&hash.0, &sign.publickey, &sign.signature)
}

/// Directed cells move value outside the TEX escrow, so the endpoint they name
/// must be an ordinary user address: the settlement address and every other
/// system address (privkey unknown, including the blackhole) are rejected,
/// mirroring the diamond `DiamondMove::User` guard.
fn check_direct_endpoint(addr: &Address) -> Rerr {
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

/// Execution gate for DirectedGet. The cell turns the transaction signer into
/// a counterparty of the bundle (funds are pulled from the signer), and the
/// product decision on enabling that semantics is still open. Chains that have
/// already grown past this height (mainnet) refuse execution, while young test
/// networks (height starting from 0) exercise the cells. Lift deliberately and
/// with a consensus-coordinated activation.
const DIRECTED_GET_OPEN_UNTIL_HEIGHT: u64 = 790_000;

fn directed_get_enabled(ctx: &dyn Context) -> Rerr {
    if ctx.env().block.height > DIRECTED_GET_OPEN_UNTIL_HEIGHT {
        return errf!(
            "directed get cells are disabled above height {} (enablement pending product decision)",
            DIRECTED_GET_OPEN_UNTIL_HEIGHT
        );
    }
    Ok(())
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
                diamonds_transfer(
                    ctx,
                    taradr,
                    &SETTLEMENT_ADDR,
                    diamonds,
                    DiamondMove::TexEscrow,
                )?;
                let max = crate::execution_params(ctx.services().as_ref())?.tex_diamond_pay_max;
                ctx.tex_ledger_mut_top()?.record_diamond_pay(diamonds, max)
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
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(&target)?;
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
                directed_get_enabled(ctx)?;
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(&source)?;
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
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(&target)?;
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, taradr, &sat)?;
                sat_add(ctx, &target, &sat)?;
                Ok(())
            }
            Self::DirectedSatGet { from, satnum } => {
                directed_get_enabled(ctx)?;
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(&source)?;
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, &source, &sat)?;
                sat_add(ctx, taradr, &sat)?;
                Ok(())
            }
            Self::DirectedDiaPay { to, diamonds } => {
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(&target)?;
                diamonds_transfer(ctx, taradr, &target, diamonds, DiamondMove::User)?;
                Ok(())
            }
            Self::DirectedDiaGet { from, diamonds } => {
                directed_get_enabled(ctx)?;
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(&source)?;
                diamonds_transfer(ctx, &source, taradr, diamonds, DiamondMove::User)?;
                Ok(())
            }
            Self::DirectedAssetPay { to, asset } => {
                let target = to.real(&ctx.env().tx.addrs)?;
                check_direct_endpoint(&target)?;
                let mut state = CoreState::wrap(ctx.layer());
                asset_sub(&mut state, taradr, asset)?;
                asset_add(&mut state, &target, asset)?;
                Ok(())
            }
            Self::DirectedAssetGet { from, asset } => {
                directed_get_enabled(ctx)?;
                let source = resolve_directed_source(ctx, from)?;
                check_direct_endpoint(&source)?;
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
            Self::CondDiaAtMost { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() >= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaAtLeast { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() <= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaEq { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() == dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
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
            let thx = self.get_sign_stuff();
            if !verify_signature(&thx, &self.addr, &self.sign) {
                return errf!(
                    "address {} signature verification failed in tex cell action",
                    self.addr.to_readable()
                );
            }
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
        ActionDispatcher, ActionExecute, BinaryCodecs, BlockHasherFn, BlockRef, DIAMOND_STATUS_NORMAL,
        Env, ExecutionServices, JsonCodecs, StateLayer, StateRead, TexLedger, Transaction, TxInfo,
        TxRef, Vm, VmExecutionParams, VmHostActionDef, VmHostCallKind,
    };
    use field::{
        AssetAmt, AssetAmtW1, Balance, BlockHeight, DiamondName, DiamondNameListMax200,
        DiamondNumber, DiamondNumberAuto, DiamondOwnedForm, DiamondSto, Inscripts, SatoshiAuto,
        Uint8, UNIT_ZHU,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

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
        assert!(ctx.tex.diatrs.is_empty());
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
        assert_eq!(hac_balance(&mut ctx, &maker_addr), Amount::default());
    }

    #[test]
    fn directed_get_is_height_gated_until_enablement_is_decided() {
        let maker = sys::Account::create_by_secret_key_value([9u8; 32]).unwrap();
        let taker = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let maker_addr = Address::from(*maker.address());
        let taker_addr = Address::from(*taker.address());
        let cells = vec![TexCell::DirectedZhuGet {
            from: AddrOrPtr::Ptr(0),
            haczhu: Fold64::from(1000).unwrap(),
        }];

        // Above the open-until height (mainnet-shaped): refused before any
        // balance moves.
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            cells.clone(),
            &maker,
        ));
        ctx.env.block.height = DIRECTED_GET_OPEN_UNTIL_HEIGHT + 1;
        CoreState::wrap(ctx.layer()).balance_set(
            &taker_addr,
            &Balance::hac(Amount::coin_u128(5000, UNIT_ZHU)),
        );
        let err = run_directed_cells(&mut ctx).unwrap_err().to_string();
        assert!(err.contains("disabled above height"), "{err}");
        assert_eq!(
            hac_balance(&mut ctx, &taker_addr),
            Amount::coin_u128(5000, UNIT_ZHU)
        );
        assert_eq!(hac_balance(&mut ctx, &maker_addr), Amount::default());

        // At the boundary height exactly (young test network): still open.
        let mut ctx = DirectedCtx::new(directed_tx(
            &taker,
            vec![taker_addr],
            cells,
            &maker,
        ));
        ctx.env.block.height = DIRECTED_GET_OPEN_UNTIL_HEIGHT;
        CoreState::wrap(ctx.layer()).balance_set(
            &taker_addr,
            &Balance::hac(Amount::coin_u128(5000, UNIT_ZHU)),
        );
        CoreState::wrap(ctx.layer()).balance_set(
            &maker_addr,
            &Balance::hac(Amount::coin_u128(2000, UNIT_ZHU)),
        );
        run_directed_cells(&mut ctx).unwrap();
        assert_eq!(
            hac_balance(&mut ctx, &maker_addr),
            Amount::coin_u128(3000, UNIT_ZHU)
        );
        assert_eq!(
            hac_balance(&mut ctx, &taker_addr),
            Amount::coin_u128(4000, UNIT_ZHU)
        );
        assert_ledger_residual_zero(&ctx);
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
            // addresses for directed moves, mirroring DiamondMove::User.
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
                    || err.contains("out of range"),
                "{err}"
            );
        }
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
}
