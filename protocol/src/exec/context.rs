//! `ContextInst` — standard Hacash execution context.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use base::{
    ActOut, ActionDispatcher, Context, Env, ExecFrom, ExecutionServices, LogEntry, P2sh,
    StateChunkRef, StateLayer, TexLedger, Transaction, TxRef, Vm,
};
use field::{Address, Hash, Sign};
use sys::{Rerr, Ret, errf};

use super::gas::{GasDiag, TxGasMeter, gas_initialize_on, gas_refund_on};

/// Diagnostic snapshot of `ContextInst` (reserved for future debug tooling).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDiag {
    pub exec_from: ExecFrom,
    pub p2sh_count: usize,
    pub gas: GasDiag,
    pub vm_initialized: bool,
}

pub struct ContextInst {
    env: Env,
    services: Arc<dyn ExecutionServices>,
    chunk: StateChunkRef,
    tx: TxRef,
    gas: TxGasMeter,
    exec_from: ExecFrom,
    psh: HashMap<Address, Box<dyn P2sh>>,
    check_sign_cache: HashMap<Address, Ret<()>>,
    bundle_sign_cache: HashMap<Hash, Ret<()>>,
    /// Test-only: counts real ECDSA verifications behind `check_bundle_sign`,
    /// so tests can assert same-tx replays hit the cache.
    #[cfg(test)]
    bundle_verify_count: usize,
    /// `None` with `vm_active == false` means lazily uninitialized; `None`
    /// with `vm_active == true` means the VM currently owns the call stack.
    vm: Option<Box<dyn Vm>>,
    vm_active: bool,
    tex: TexLedger,
}

#[allow(dead_code)] // diagnostic / reserved methods for future debug tooling
impl ContextInst {
    pub fn new(
        env: Env,
        services: Arc<dyn ExecutionServices>,
        chunk: StateChunkRef,
        tx: TxRef,
    ) -> Ret<Self> {
        chunk.validate_tx_identity(&tx.hash())?;
        Ok(Self {
            env,
            services,
            chunk,
            tx,
            gas: TxGasMeter::new(),
            exec_from: ExecFrom::Top,
            psh: HashMap::new(),
            check_sign_cache: HashMap::new(),
            bundle_sign_cache: HashMap::new(),
            #[cfg(test)]
            bundle_verify_count: 0,
            vm: None,
            vm_active: false,
            tex: TexLedger::default(),
        })
    }

    pub fn services_ref(&self) -> &Arc<dyn ExecutionServices> {
        &self.services
    }

    pub fn gas_diag(&self) -> GasDiag {
        self.gas.diag()
    }

    pub fn diag(&self) -> ContextDiag {
        ContextDiag {
            exec_from: self.exec_from,
            p2sh_count: self.psh.len(),
            gas: self.gas_diag(),
            vm_initialized: self.vm.is_some(),
        }
    }

    fn ensure_vm_ready(&mut self) {
        if self.vm.is_none() && !self.vm_active {
            self.vm = Some(
                self.services
                    .assign_vm(self.env.block.height)
                    .unwrap_or_else(|| Box::new(base::EmptyVm)),
            );
        }
    }

    /// Real signature verification with per-tx caching. Mode-independent by
    /// construction: both `check_sign` and `check_sign_data` resolve here, so
    /// admission gating and consensus-visible queries can never disagree in
    /// strict mode.
    fn check_sign_cached(&mut self, adr: &Address) -> Rerr {
        if let Some(cached) = self.check_sign_cache.get(adr) {
            return cached.clone();
        }
        if adr.is_privkey_unknown() {
            let err: Rerr = errf!(
                "address {} is a system address (value < u32::MAX) with unknown private key",
                adr.to_readable()
            );
            self.check_sign_cache.insert(*adr, err.clone());
            return err;
        }
        if let Err(e) = adr.must_privkey() {
            self.check_sign_cache.insert(*adr, Err(e.clone()));
            return Err(e);
        }
        let isok = crate::tx_std::verify_target_signature(adr, self.tx.as_ref()).map(|_| ());
        self.check_sign_cache.insert(*adr, isok.clone());
        isok
    }
}

impl Context for ContextInst {
    fn services(&self) -> Arc<dyn ExecutionServices> {
        self.services.clone()
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
    /// Admission gating: fast sync trusts the synced chain (PoW), so the
    /// check is skipped. The early return stays before the cache so the
    /// mode-dependent `Ok` can never be observed by `check_sign_data`.
    fn check_sign(&mut self, adr: &Address) -> Rerr {
        if self.env.chain.fast_sync {
            return Ok(());
        }
        self.check_sign_cached(adr)
    }
    /// Consensus-visible query: real verification in every mode, or fast-sync
    /// replay of a strict-executed block would write different state.
    fn check_sign_data(&mut self, adr: &Address) -> Rerr {
        self.check_sign_cached(adr)
    }

    /// Bundle signatures: fast sync skips like `check_sign`; strict mode
    /// caches by (message, public key, signature) so replaying one bundle N
    /// times in a transaction runs one ECDSA verification, not N. Bundle
    /// results never feed consensus-visible state, so there is no
    /// `check_sign_data` counterpart to keep mode-independent.
    fn check_bundle_sign(&mut self, msg: &Hash, addr: &Address, sign: &Sign) -> Rerr {
        if self.env.chain.fast_sync {
            return Ok(());
        }
        let mut key_bytes = Vec::with_capacity(Hash::SIZE + 33 + 64);
        key_bytes.extend_from_slice(msg.as_ref());
        key_bytes.extend_from_slice(&sign.publickey.into_array());
        key_bytes.extend_from_slice(sign.signature.as_ref());
        let key = Hash::from(sys::calculate_hash(&key_bytes));
        if let Some(cached) = self.bundle_sign_cache.get(&key) {
            return cached.clone();
        }
        let result = base::verify_standalone_sign(msg, addr, sign);
        if result.is_ok() {
            #[cfg(test)]
            {
                self.bundle_verify_count += 1;
            }
        }
        self.bundle_sign_cache.insert(key, result.clone());
        result
    }

    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.chunk
    }

    fn emit_log(&mut self, entry: LogEntry) {
        self.chunk.emit_log(entry);
    }

    fn gas_remaining(&self) -> i64 {
        self.gas.remaining()
    }
    fn gas_charge(&mut self, gas: i64) -> Rerr {
        self.gas.charge(gas)
    }
    fn gas_rebate(&mut self, gas: i64) -> Rerr {
        // Allowed in any exec_from (VM CALL sites rebate; matches mainnet).
        self.gas.rebate(gas)
    }
    fn gas_initialize(&mut self, budget: i64) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!("gas_initialize only allowed in TOP context");
        }
        let mut gas = std::mem::take(&mut self.gas);
        let res = gas_initialize_on(&mut gas, self, budget);
        self.gas = gas;
        res
    }
    fn gas_refund(&mut self) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!("gas_refund only allowed in TOP context");
        }
        let mut gas = std::mem::take(&mut self.gas);
        let res = gas_refund_on(&mut gas, self);
        self.gas = gas;
        res
    }

    fn snapshot_volatile(&self) -> Box<dyn Any> {
        Box::new(self.gas.rebated_checkpoint())
    }
    fn restore_volatile(&mut self, snap: Box<dyn Any>) {
        let rebated = *snap.downcast::<i64>().expect("gas snapshot type mismatch");
        // AST rollback: keep gas_charge effects, roll back rebate only.
        self.gas.restore_rebated(rebated);
    }

    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<ActOut> {
        let mut buf = Vec::with_capacity(2 + body.len());
        buf.extend_from_slice(&kind.to_be_bytes());
        buf.extend_from_slice(&body);

        let reg = self.services.clone();
        let (action, used) = reg.decode_action(&buf)?;
        if used != buf.len() {
            return errf!(
                "action parse length mismatch: consumed {} but body length is {}",
                used,
                buf.len()
            );
        }
        ActionDispatcher::dispatch_call(self, &action)
    }

    fn exec_ast_child(
        &mut self,
        run: &mut dyn FnMut(&mut dyn Context) -> Ret<ActOut>,
    ) -> Ret<ActOut> {
        let child = self.chunk.spawn_ast_child()?;
        let parent = std::mem::replace(&mut self.chunk, child);
        let res = run(self);
        let child = std::mem::replace(&mut self.chunk, parent);
        match res {
            Ok(out) => {
                let parent = child.commit_to_parent()?;
                debug_assert!(parent.ptr_eq(&self.chunk));
                Ok(out)
            }
            Err(err) => {
                child.discard()?;
                Err(err)
            }
        }
    }

    fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
        if self.vm_active {
            return None;
        }
        self.ensure_vm_ready();
        self.vm_active = true;
        self.vm.take()
    }
    fn vm_put(&mut self, vm: Box<dyn Vm>) {
        debug_assert!(self.vm_active, "VM returned without an active take");
        self.vm = Some(vm);
        self.vm_active = false;
    }
    fn vm_peek(&mut self) -> Option<&mut (dyn Vm + 'static)> {
        if self.vm_active {
            return None;
        }
        self.ensure_vm_ready();
        self.vm.as_deref_mut()
    }
    fn as_context_mut(&mut self) -> &mut dyn Context {
        self
    }

    fn release_chunk(self: Box<Self>) -> Ret<StateChunkRef> {
        Ok(self.chunk)
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
        self.tex.ensure_open()?;
        Ok(&mut self.tex)
    }

    fn p2sh(&self, addr: &Address) -> Ret<&dyn P2sh> {
        match self.psh.get(addr) {
            Some(b) => Ok(b.as_ref()),
            None => errf!("p2sh '{}' not found", addr.to_readable()),
        }
    }
    fn p2sh_count(&self) -> usize {
        self.psh.len()
    }
    fn p2sh_set(&mut self, addr: Address, p2sh: Box<dyn P2sh>) -> Rerr {
        if self.exec_from != ExecFrom::Top {
            return errf!(
                "p2sh_set only allowed in TOP context, got {}",
                self.exec_from
            );
        }
        addr.must_scriptmh()?;
        if self.psh.contains_key(&addr) {
            return errf!("p2sh '{}' already proved in current tx", addr.to_readable());
        }
        self.psh.insert(addr, p2sh);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{
        BinaryCodecs, BlockHasherFn, BlockRef, DiskDB, JsonCodecs, TransactionSign,
        VmExecutionParams, VmHostActionDef, VmHostCallKind,
    };
    use field::Amount;

    struct NoDisk;

    impl DiskDB for NoDisk {
        fn read(&self, _key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
            Ok(None)
        }
        fn save(&self, _key: &[u8], _val: &[u8]) {}
        fn remove(&self, _key: &[u8]) {}
        fn try_write(&self, _memkv: &dyn base::MemDB) -> Rerr {
            Ok(())
        }
    }

    fn stub_hasher(_height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }

    struct StubServices;

    impl BinaryCodecs for StubServices {
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

    impl JsonCodecs for StubServices {
        fn decode_action_json(&self, _json: &str) -> Ret<base::ActionRef> {
            errf!("stub: decode_action_json")
        }
    }

    impl base::ExecutionServices for StubServices {
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

    fn signed_tx() -> (crate::codec::tx::StdTransaction, Address, Address) {
        let acc_main = sys::Account::create_by_secret_key_value([7u8; 32]).unwrap();
        let acc_other = sys::Account::create_by_secret_key_value([8u8; 32]).unwrap();
        let main = Address::from(*acc_main.address());
        let other = Address::from(*acc_other.address());
        let mut tx = crate::codec::tx::StdTransaction::new(
            hacash_params::TX_TYPE_2,
            main,
            Amount::mei(1),
        );
        tx.fill_sign_account(&acc_main).unwrap();
        (tx, main, other)
    }

    fn build_ctx(fast_sync: bool) -> (Box<ContextInst>, Address, Address) {
        let (tx, main, other) = signed_tx();
        let mut env = Env::default();
        env.chain.fast_sync = fast_sync;
        env.block.height = 1;
        let block = base::StateChunkRef::block_draft_on_disk(Arc::new(NoDisk), 1);
        let chunk = block.spawn_tx_child(tx.hash()).unwrap();
        let ctx = ContextInst::new(env, Arc::new(StubServices), chunk, Arc::new(tx)).unwrap();
        (Box::new(ctx), main, other)
    }

    /// HVM-0001 regression: the consensus-visible query must verify for real
    /// in every mode, while admission gating keeps its fast-sync bypass.
    #[test]
    fn check_sign_data_is_mode_independent_but_check_sign_is_not() {
        let (mut fast, main, other) = build_ctx(true);

        // Fast sync: admission answers Ok for the non-signer without verifying.
        assert!(fast.check_sign(&other).is_ok());
        // The data path still sees the truth: `other` did not sign this tx.
        assert!(fast.check_sign_data(&other).is_err());
        // And a real signature verifies through the data path too.
        assert!(fast.check_sign_data(&main).is_ok());

        // Strict: both paths agree with real verification.
        let (mut strict, main, other) = build_ctx(false);
        assert!(strict.check_sign(&other).is_err());
        assert!(strict.check_sign_data(&other).is_err());
        assert!(strict.check_sign_data(&main).is_ok());
    }

    /// The fast-sync admission bypass must not poison the shared cache that
    /// the data path reads through.
    #[test]
    fn fast_sync_admission_bypass_does_not_poison_the_data_cache() {
        let (mut fast, main, other) = build_ctx(true);
        assert!(fast.check_sign(&other).is_ok()); // bypass first
        assert!(fast.check_sign_data(&other).is_err()); // still the real answer
        assert!(fast.check_sign_data(&main).is_ok());
        assert!(fast.check_sign(&other).is_ok()); // bypass unaffected by cache
    }

    /// Same-tx replay of one bundle must run one ECDSA verification, not N:
    /// the per-tx bundle-sign cache absorbs the duplicates. A different
    /// signature over the same message is a different cache entry.
    #[test]
    fn bundle_sign_cache_absorbs_same_tx_replays() {
        let (mut ctx, _main, _other) = build_ctx(false);
        let signer = sys::Account::create_by_password("bundle cache").unwrap();
        let addr = Address::from(*signer.address());
        let msg = Hash::from(sys::calculate_hash(b"bundle message bytes".to_vec()));
        let sign = field::Sign::create_by(&signer, &msg);

        ctx.check_bundle_sign(&msg, &addr, &sign).unwrap();
        ctx.check_bundle_sign(&msg, &addr, &sign).unwrap();
        ctx.check_bundle_sign(&msg, &addr, &sign).unwrap();
        assert_eq!(ctx.bundle_verify_count, 1, "replays must hit the cache");

        // A different keypair over the same message is a different entry.
        let other = sys::Account::create_by_password("bundle cache 2").unwrap();
        let sign2 = field::Sign::create_by(&other, &msg);
        ctx.check_bundle_sign(&msg, &Address::from(*other.address()), &sign2)
            .unwrap();
        assert_eq!(ctx.bundle_verify_count, 2);

        // An invalid signature is a real failure and never cached as Ok.
        let mut bad = sign.clone();
        bad.signature = field::Fixed::from([0u8; 64]);
        assert!(ctx.check_bundle_sign(&msg, &addr, &bad).is_err());
    }

    /// Fast sync trusts the synced chain (PoW), so bundle verification is
    /// skipped like every other signature check — even a broken signature
    /// passes, and nothing is written into the strict-mode cache.
    #[test]
    fn bundle_sign_skips_under_fast_sync() {
        let (mut ctx, _main, _other) = build_ctx(true);
        let signer = sys::Account::create_by_password("bundle fast").unwrap();
        let addr = Address::from(*signer.address());
        let msg = Hash::from(sys::calculate_hash(b"bundle message bytes".to_vec()));
        let mut bad = field::Sign::create_by(&signer, &msg);
        bad.signature = field::Fixed::from([0u8; 64]);
        assert!(ctx.check_bundle_sign(&msg, &addr, &bad).is_ok());
        assert_eq!(ctx.bundle_verify_count, 0, "fast sync must not verify");
    }
}
