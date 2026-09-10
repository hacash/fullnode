//! VM syscall action execute bodies.

use base::CoreState;
use field::{Address, AddressW1, DiamondName, Encode};
use sys::errf;

use crate::codec::action::{
    BalanceAsset, BalanceCoin, BlockAuthorAddr, CheckSignature, EnvHeight, HacdInscGet,
    HacdInscNum, HacdNameList, HacdOwnerAddrs, SigsetAtLeast, SigsetCount, TxBlob, TxBlobNum,
    TxBlobSize, TxMainAddr, TxMessage, TxMessageNum,
};

/// Temporary upgrade gate for the tx message/blob read syscalls (0x0615/0x0616/
/// 0x0617/0x0704/0x0705), which take effect at height 784_000. Hand-written on
/// purpose — remove the const, this helper and the five call sites together with
/// the syscalls in the next release.
const TX_MSG_BLOB_ENABLE_HEIGHT: u64 = 784_000;

fn tx_message_blob_gate(ctx: &dyn base::Context) -> sys::Rerr {
    // Mainnet activates at height 784_000. Non-mainnet (hacash-testnet chain_id=1)
    // enables immediately so deposit hooks can ViewMessage on a local chain.
    if ctx.env().chain.id.is_mainnet() && ctx.env().block.height < TX_MSG_BLOB_ENABLE_HEIGHT {
        return errf!(
            "tx message/blob syscall not enabled until height {}",
            TX_MSG_BLOB_ENABLE_HEIGHT
        );
    }
    Ok(())
}

base::impl_action_execute! {
    TxMessage {
        (self, ctx) {
            tx_message_blob_gate(ctx)?;
            let mut n = 0u8;
            for action in ctx.tx().actions() {
                if let Some(msg) = action.as_any().downcast_ref::<crate::codec::action::Message>() {
                    if n == self.idx.uint() { return Ok(msg.data.as_ref().to_vec()); }
                    n = n.saturating_add(1);
                }
            }
            errf!("transaction message index {} out of range", self.idx.uint())
        }
    }
}

base::impl_action_execute! {
    TxBlob {
        (self, ctx) {
            tx_message_blob_gate(ctx)?;
            let mut n = 0u8;
            for action in ctx.tx().actions() {
                if let Some(blob) = action.as_any().downcast_ref::<crate::codec::action::Blob>() {
                    if n == self.idx.uint() {
                        let start = self.start.uint() as usize;
                        let end = self.end.uint() as usize;
                        let data = blob.data.as_ref();
                        if start > end || end > data.len() {
                            return errf!("blob range [{}..{}] out of range for size {}", start, end, data.len());
                        }
                        return Ok(data[start..end].to_vec());
                    }
                    n = n.saturating_add(1);
                }
            }
            errf!("transaction blob index {} out of range", self.idx.uint())
        }
    }
}

base::impl_action_execute! {
    TxMessageNum { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let n = ctx.tx().actions().iter().filter(|a| a.as_any().is::<crate::codec::action::Message>()).count();
        if n > u8::MAX as usize { return errf!("message count exceeds u8"); }
        Ok(vec![n as u8])
    } }
}
base::impl_action_execute! {
    TxBlobNum { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let n = ctx.tx().actions().iter().filter(|a| a.as_any().is::<crate::codec::action::Blob>()).count();
        if n > u8::MAX as usize { return errf!("blob count exceeds u8"); }
        Ok(vec![n as u8])
    } }
}
base::impl_action_execute! {
    TxBlobSize { (self, ctx) {
        tx_message_blob_gate(ctx)?;
        let mut n = 0u8;
        for action in ctx.tx().actions() {
            if let Some(blob) = action.as_any().downcast_ref::<crate::codec::action::Blob>() {
                if n == self.idx.uint() {
                    let len = blob.data.as_ref().len();
                    if len > u16::MAX as usize {
                        return errf!("transaction blob {} size {} exceeds u16::MAX", self.idx.uint(), len);
                    }
                    return Ok((len as u16).to_be_bytes().to_vec());
                }
                n = n.saturating_add(1);
            }
        }
        errf!("transaction blob index {} out of range", self.idx.uint())
    } }
}
base::impl_action_execute! {
    EnvHeight {
        (self, ctx) {
            Ok(ctx.env().block.height.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    TxMainAddr {
        (self, ctx) {
            Ok(ctx.env().tx.main.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    BlockAuthorAddr {
        (self, ctx) {
            Ok(ctx.env().block.author.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    BalanceCoin {
        (self, ctx) {
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let dia = bls.diamond.uint();
            if dia > u32::MAX as u64 {
                return errf!(
                    "address {} diamond count {} exceeds u32::MAX",
                    self.addr.to_readable(),
                    dia
                );
            }
            let hac = bls.hacash.encode();
            let mut res = Vec::with_capacity(12 + hac.len());
            res.extend_from_slice(&(dia as u32).to_be_bytes());
            res.extend_from_slice(&bls.satoshi.uint().to_be_bytes());
            res.extend_from_slice(&hac);
            Ok(res)
        }
    }
}

base::impl_action_execute! {
    BalanceAsset {
        (self, ctx) {
            let serial = self.serial.uint();
            if serial == 0 {
                return errf!("asset serial cannot be zero");
            }
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let amt = bls
                .assets
                .as_list()
                .iter()
                .find(|a| a.serial.uint() == serial)
                .map(|a| a.amount.uint())
                .unwrap_or(0);
            Ok(amt.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    CheckSignature {
        (self, ctx) {
            let ok = match ctx.check_sign(&self.addr) {
                Ok(()) => 1u8,
                Err(_) => 0u8,
            };
            Ok(vec![ok])
        }
    }
}

const SIGSET_MAX: u8 = 200;

fn sigset_validate_keys(keys: &AddressW1) -> sys::Ret<u8> {
    let n = keys.length();
    if n == 0 || n > SIGSET_MAX as usize {
        return errf!("sigset key count {} not in 1..={}", n, SIGSET_MAX);
    }
    let list = keys.as_list();
    for (i, addr) in list.iter().enumerate() {
        addr.must_privkey()?;
        if addr.as_ref() == &[0u8; Address::SIZE] {
            return errf!("sigset address cannot be the zero address");
        }
        if addr.is_privkey_unknown() {
            return errf!(
                "sigset address {} is a system address with unknown private key",
                addr.to_readable()
            );
        }
        if list[..i].contains(addr) {
            return errf!("sigset address {} is duplicated", addr.to_readable());
        }
    }
    Ok(n as u8)
}

fn sigset_signed_count(ctx: &mut dyn base::Context, keys: &AddressW1) -> sys::Ret<u8> {
    sigset_validate_keys(keys)?;
    let mut count = 0u8;
    for addr in keys.as_list() {
        if ctx.check_sign(addr).is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

base::impl_action_execute! {
    SigsetCount {
        (self, ctx) {
            Ok(vec![sigset_signed_count(ctx, &self.keys)?])
        }
    }
}

base::impl_action_execute! {
    SigsetAtLeast {
        (self, ctx) {
            let n = sigset_validate_keys(&self.keys)?;
            let threshold = self.threshold.uint();
            if threshold == 0 || threshold > n {
                return errf!("sigset threshold {} not in 1..={}", threshold, n);
            }
            let mut count = 0u8;
            for addr in self.keys.as_list() {
                if ctx.check_sign(addr).is_ok() {
                    count += 1;
                }
            }
            Ok(vec![u8::from(count >= threshold)])
        }
    }
}

base::impl_action_execute! {
    HacdInscNum {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            if num > u8::MAX as usize {
                return errf!(
                    "diamond {} inscripts number invalid",
                    self.diamond.to_readable()
                );
            }
            Ok(vec![num as u8])
        }
    }
}

base::impl_action_execute! {
    HacdInscGet {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            let idx = self.inscidx.uint() as usize;
            if idx >= num {
                return errf!(
                    "diamond {} inscripts number overflow",
                    self.diamond.to_readable()
                );
            }
            Ok(diaobj.inscripts.as_list()[idx].content.to_vec())
        }
    }
}

base::impl_action_execute! {
    HacdNameList {
        (self, ctx) {
            const DNM_SZ: usize = DiamondName::SIZE;
            let owned = CoreState::wrap(ctx.layer())
                .diamond_owned(&self.addr)?
                .unwrap_or_default();
            let names = owned.names.as_ref();
            if names.len() % DNM_SZ != 0 {
                return errf!(
                    "address {} diamond names length {} invalid",
                    self.addr.to_readable(),
                    names.len()
                );
            }
            let limit = self.limit.uint() as usize;
            if limit > 200 {
                return errf!("limit {} cannot exceed 200", limit);
            }
            if limit == 0 {
                return Ok(vec![]);
            }
            let page = self.page.uint() as usize;
            let unit = limit * DNM_SZ;
            let start = page.saturating_mul(unit);
            if start >= names.len() {
                return Ok(vec![]);
            }
            let end = start.saturating_add(unit).min(names.len());
            Ok(names[start..end].to_vec())
        }
    }
}

base::impl_action_execute! {
    HacdOwnerAddrs {
        (self, ctx) {
            let num = self.diamonds.check()?;
            if num > 50 {
                return errf!("diamond list length {} cannot exceed 50", num);
            }
            let state = CoreState::wrap(ctx.layer());
            let mut res = Vec::with_capacity(num * Address::SIZE);
            for dian in self.diamonds.as_list() {
                let Some(diaobj) = state.diamond(dian)? else {
                    return errf!("diamond {} not found", dian.to_readable());
                };
                res.extend_from_slice(diaobj.address.as_ref());
            }
            Ok(res)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{
        ActOut, ActionExecute, ActionRef, BinaryCodecs, BlockHasherFn, BlockRef, Context, Env,
        ExecFrom, ExecutionServices, JsonCodecs, LogEntry, P2sh, StateLayer, StateRead, TexLedger,
        Transaction, TxRef, Vm, VmExecutionParams, VmHostActionDef, VmHostCallKind,
    };
    use field::{Amount, Encode, Uint1};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use sys::{Rerr, Ret};

    fn privkey_addr(n: u8) -> Address {
        let mut bytes = [0u8; Address::SIZE];
        bytes[1] = n;
        Address::from(bytes)
    }

    fn keys(addrs: Vec<Address>) -> AddressW1 {
        AddressW1::from(addrs).unwrap()
    }

    fn contract_addr() -> Address {
        let mut bytes = [0u8; Address::SIZE];
        bytes[0] = Address::VERSION_CONTRACT;
        bytes[1] = 1;
        Address::from(bytes)
    }

    fn zero_addr() -> Address {
        Address::from([0u8; Address::SIZE])
    }

    fn unknown_addr() -> Address {
        crate::params::SETTLEMENT_ADDR
    }

    #[derive(Debug)]
    struct DummyTx;

    impl Encode for DummyTx {
        fn size(&self) -> usize {
            0
        }
        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }

    impl Transaction for DummyTx {
        fn ty(&self) -> u8 {
            3
        }
        fn main(&self) -> Address {
            privkey_addr(1)
        }
        fn fee(&self) -> &Amount {
            Amount::zero_ref()
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

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

    struct StubServices;

    fn stub_hasher(_height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }

    impl BinaryCodecs for StubServices {
        fn decode_action(&self, _buf: &[u8]) -> Ret<(ActionRef, usize)> {
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

    impl JsonCodecs for StubServices {
        fn decode_action_json(&self, _json: &str) -> Ret<ActionRef> {
            errf!("stub: decode_action_json")
        }
    }

    impl ExecutionServices for StubServices {
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
            errf!("stub: execution_profile")
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

    struct SigsetCtx {
        env: Env,
        tx: DummyTx,
        layer: MemLayer,
        exec_from: ExecFrom,
        tex: TexLedger,
        signed: HashSet<Address>,
    }

    impl SigsetCtx {
        fn new(signed: impl IntoIterator<Item = Address>) -> Self {
            Self {
                env: Env::default(),
                tx: DummyTx,
                layer: MemLayer::default(),
                exec_from: ExecFrom::Call,
                tex: TexLedger::default(),
                signed: signed.into_iter().collect(),
            }
        }
    }

    impl Context for SigsetCtx {
        fn services(&self) -> Arc<dyn ExecutionServices> {
            Arc::new(StubServices)
        }
        fn env(&self) -> &Env {
            &self.env
        }
        fn tx(&self) -> &dyn Transaction {
            &self.tx
        }
        fn exec_from(&self) -> ExecFrom {
            self.exec_from
        }
        fn exec_from_set(&mut self, from: ExecFrom) {
            self.exec_from = from;
        }
        fn check_sign(&mut self, adr: &Address) -> Rerr {
            if self.signed.contains(adr) {
                Ok(())
            } else {
                errf!("unsigned")
            }
        }
        fn layer(&mut self) -> &mut dyn StateLayer {
            &mut self.layer
        }
        fn emit_log(&mut self, _entry: LogEntry) {}
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

    fn run_count(addrs: AddressW1, signed: &[Address]) -> sys::Ret<ActOut> {
        ActionExecute::execute(
            &SigsetCount::new(addrs),
            &mut SigsetCtx::new(signed.iter().copied()),
        )
    }

    fn run_at_least(addrs: AddressW1, threshold: u8, signed: &[Address]) -> sys::Ret<ActOut> {
        ActionExecute::execute(
            &SigsetAtLeast::new(addrs, Uint1::from(threshold)),
            &mut SigsetCtx::new(signed.iter().copied()),
        )
    }

    fn assert_err(res: sys::Ret<ActOut>) {
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test]
    fn sigset_malformed_sets_are_errors_not_zero_or_false() {
        assert_err(run_count(keys(vec![]), &[]));
        assert_err(run_at_least(keys(vec![]), 1, &[]));

        let n51 = keys((1u8..=51).map(privkey_addr).collect());
        let (_, ret) = run_count(n51.clone(), &[]).unwrap();
        assert_eq!(ret, vec![0]);
        let (_, ret) = run_at_least(n51, 1, &[]).unwrap();
        assert_eq!(ret, vec![0]);

        let n201 = keys((1u8..=201).map(privkey_addr).collect());
        assert_err(run_count(n201.clone(), &[]));
        assert_err(run_at_least(n201, 1, &[]));

        assert_err(run_count(keys(vec![zero_addr()]), &[]));
        assert_err(run_at_least(keys(vec![zero_addr()]), 1, &[]));

        assert_err(run_count(keys(vec![unknown_addr()]), &[]));
        assert_err(run_at_least(keys(vec![unknown_addr()]), 1, &[]));

        assert_err(run_count(keys(vec![contract_addr()]), &[]));
        assert_err(run_at_least(keys(vec![contract_addr()]), 1, &[]));

        let a = privkey_addr(1);
        assert_err(run_count(keys(vec![a, a]), &[a]));
        assert_err(run_at_least(keys(vec![a, a]), 1, &[a]));

        assert_err(run_at_least(keys(vec![a]), 0, &[a]));
        assert_err(run_at_least(keys(vec![a]), 2, &[a]));
    }

    #[test]
    fn sigset_duplicate_after_threshold_still_fails() {
        let a = privkey_addr(1);
        let b = privkey_addr(2);
        assert_err(run_at_least(keys(vec![a, b, a]), 2, &[a, b]));
        assert_err(run_count(keys(vec![a, b, a]), &[a, b]));
    }

    #[test]
    fn sigset_legal_counts_match_check_sign() {
        let a = privkey_addr(1);
        let (gas, ret) = run_count(keys(vec![a]), &[a]).unwrap();
        assert_eq!(ret, vec![1]);
        assert_eq!(gas, Encode::size(&SigsetCount::new(keys(vec![a]))) as u32);
        let (_, ret) = run_at_least(keys(vec![a]), 1, &[a]).unwrap();
        assert_eq!(ret, vec![1]);

        let b = privkey_addr(2);
        let c = privkey_addr(3);
        let trio = keys(vec![a, b, c]);
        let signed = [a, b];
        let (_, ret) = run_count(trio.clone(), &signed).unwrap();
        assert_eq!(ret, vec![2]);
        let (_, ret) = run_at_least(trio.clone(), 2, &signed).unwrap();
        assert_eq!(ret, vec![1]);
        let (_, ret) = run_at_least(trio, 3, &signed).unwrap();
        assert_eq!(ret, vec![0]);
    }

    #[test]
    fn sigset_n50_executes_and_costs_at_least_n1() {
        let n50 = keys((1u8..=50).map(privkey_addr).collect());
        let n1 = keys(vec![privkey_addr(1)]);
        let (gas50, ret) = run_count(n50.clone(), &[]).unwrap();
        assert_eq!(ret, vec![0]);
        let (gas1, _) = run_count(n1, &[]).unwrap();
        assert!(gas50 >= gas1);
        assert_eq!(gas50, Encode::size(&SigsetCount::new(n50.clone())) as u32);

        let (_, ret) = run_at_least(n50, 1, &[]).unwrap();
        assert_eq!(ret, vec![0]);
    }

    #[test]
    fn sigset_n200_executes_and_costs_at_least_n1() {
        let n200 = keys((1u8..=200).map(privkey_addr).collect());
        let n1 = keys(vec![privkey_addr(1)]);
        let (gas200, ret) = run_count(n200.clone(), &[]).unwrap();
        assert_eq!(ret, vec![0]);
        let (gas1, _) = run_count(n1, &[]).unwrap();
        assert!(gas200 >= gas1);
        assert_eq!(gas200, Encode::size(&SigsetCount::new(n200.clone())) as u32);

        let (_, ret) = run_at_least(n200, 1, &[]).unwrap();
        assert_eq!(ret, vec![0]);
    }
}
