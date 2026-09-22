use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use base::{
    ActOut, ActionDispatcher, Context, Env, ExecFrom, ExecutionServices, LogEntry,
    P2sh, StateLayer, TexLedger, Transaction, TransactionSign, Vm,
};
use field::Address;
use sys::{Rerr, Ret};

use super::state::FlatMemState;
use super::tx::{DummyTx, StubTx};

/// Configurable in-process execution context for action and VM tests.
///
/// This owns an isolated state layer.  It deliberately does not implement the
/// engine's `release_chunk` lifecycle; chain-level tests should use a real
/// `StateChunk` through `MemChain` once a transaction needs commit semantics.
pub struct TestContext {
    services: Arc<dyn ExecutionServices>,
    pub env: Env,
    tx: Arc<dyn TransactionSign>,
    pub layer: FlatMemState,
    pub logs: Vec<LogEntry>,
    pub exec_from: ExecFrom,
    pub tex: TexLedger,
    pub gas: i64,
    vm: Option<Box<dyn Vm>>,
    p2sh: HashMap<Address, Box<dyn P2sh>>,
}

#[derive(Clone)]
struct VolatileSnapshot {
    layer: FlatMemState,
    logs: Vec<LogEntry>,
    exec_from: ExecFrom,
    tex: TexLedger,
    gas: i64,
}

impl TestContext {
    pub fn new(services: Arc<dyn ExecutionServices>, tx: Arc<dyn TransactionSign>) -> Self {
        let mut env = Env::default();
        env.tx.ty = tx.ty();
        env.tx.main = tx.main();
        env.tx.addrs = tx.addrs();
        env.tx.fee = tx.fee().clone();
        Self {
            services,
            env,
            tx,
            layer: FlatMemState::default(),
            logs: Vec::new(),
            exec_from: ExecFrom::Top,
            tex: TexLedger::default(),
            gas: 1 << 30,
            vm: None,
            p2sh: HashMap::new(),
        }
    }

    pub fn with_stub_tx(services: Arc<dyn ExecutionServices>) -> Self {
        Self::new(services, Arc::new(StubTx::default()))
    }

    pub fn with_dummy_tx(services: Arc<dyn ExecutionServices>) -> Self {
        Self::new(services, Arc::new(DummyTx))
    }

    pub fn standard() -> Ret<Self> {
        let services: Arc<dyn ExecutionServices> = Arc::new(app::standard_registry()?);
        Ok(Self::with_stub_tx(services))
    }

    pub fn services_ref(&self) -> Arc<dyn ExecutionServices> {
        self.services.clone()
    }
    pub fn set_vm(&mut self, vm: Box<dyn Vm>) {
        self.vm = Some(vm);
    }
    pub fn take_logs(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.logs)
    }
    pub fn charge(&mut self, amount: i64) -> Rerr {
        self.gas = self
            .gas
            .checked_sub(amount)
            .ok_or_else(|| sys::Error::normal("test context gas overflow"))?;
        if self.gas < 0 {
            return sys::errf!("test context out of gas");
        }
        Ok(())
    }
}

impl Context for TestContext {
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
    fn check_sign(&mut self, address: &Address) -> Rerr {
        if self.tx.req_sign()?.contains(address) {
            Ok(())
        } else {
            sys::errf!(
                "test transaction does not authorize {}",
                address.to_readable()
            )
        }
    }
    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.layer
    }
    fn emit_log(&mut self, entry: LogEntry) {
        self.logs.push(entry);
    }
    fn gas_remaining(&self) -> i64 {
        self.gas
    }
    fn gas_charge(&mut self, gas: i64) -> Rerr {
        self.charge(gas)
    }
    fn gas_rebate(&mut self, gas: i64) -> Rerr {
        self.gas = self
            .gas
            .checked_add(gas)
            .ok_or_else(|| sys::Error::normal("test context gas overflow"))?;
        Ok(())
    }
    fn gas_initialize(&mut self, budget: i64) -> Rerr {
        self.gas = budget;
        Ok(())
    }
    fn gas_refund(&mut self) -> Rerr {
        Ok(())
    }
    fn snapshot_volatile(&self) -> Box<dyn Any> {
        Box::new(VolatileSnapshot {
            layer: self.layer.clone(),
            logs: self.logs.clone(),
            exec_from: self.exec_from,
            tex: self.tex.clone(),
            gas: self.gas,
        })
    }
    fn restore_volatile(&mut self, snapshot: Box<dyn Any>) {
        let snapshot = snapshot
            .downcast::<VolatileSnapshot>()
            .expect("test context volatile snapshot type mismatch");
        self.layer = snapshot.layer;
        self.logs = snapshot.logs;
        self.exec_from = snapshot.exec_from;
        self.tex = snapshot.tex;
        self.gas = snapshot.gas;
    }
    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<ActOut> {
        // Keep in-process VM tests on the same host-action path as the
        // production execution context.  In particular, ACTVIEW calls such
        // as hacd_insc_num / hacd_insc_get must be decodable in TestContext.
        let mut wire = Vec::with_capacity(2 + body.len());
        wire.extend_from_slice(&kind.to_be_bytes());
        wire.extend_from_slice(&body);
        let services = self.services.clone();
        let (action, used) = services.decode_action(&wire)?;
        if used != wire.len() {
            return sys::errf!(
                "test action parse length mismatch: consumed {} but body length is {}",
                used,
                wire.len()
            );
        }
        ActionDispatcher::dispatch_call(self, &action)
    }
    fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
        self.vm.take()
    }
    fn vm_put(&mut self, vm: Box<dyn Vm>) {
        self.vm = Some(vm);
    }
    fn as_context_mut(&mut self) -> &mut dyn Context {
        self
    }
    fn tex_ledger(&self) -> &TexLedger {
        &self.tex
    }
    fn tex_ledger_mut_top(&mut self) -> Ret<&mut TexLedger> {
        Ok(&mut self.tex)
    }
    fn p2sh(&self, address: &Address) -> Ret<&dyn P2sh> {
        self.p2sh
            .get(address)
            .map(|entry| entry.as_ref())
            .ok_or_else(|| sys::Error::normal("p2sh not found"))
    }
    fn p2sh_count(&self) -> usize {
        self.p2sh.len()
    }
    fn p2sh_set(&mut self, address: Address, p2sh: Box<dyn P2sh>) -> Rerr {
        self.p2sh.insert(address, p2sh);
        Ok(())
    }
}

/// Old testkit's `ContextInst` constructor was removed upstream.  This helper
/// supplies its direct replacement for the new context contract.
pub fn make_ctx_with_state(
    services: Arc<dyn ExecutionServices>,
    tx: Arc<dyn TransactionSign>,
    state: FlatMemState,
) -> TestContext {
    let mut ctx = TestContext::new(services, tx);
    ctx.layer = state;
    ctx
}
