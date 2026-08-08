//! Shared minimal `Context` + `ExecutionServices` stubs for machine-level tests.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;

use base::{
    ActOut, ActionRef, BinaryCodecs, BlockHasherFn, BlockRef, Context, Env, ExecFrom,
    ExecutionServices, JsonCodecs, LogEntry, P2sh, StateChunkRef, StateLayer, StateRead, TexLedger,
    Transaction, TxRef, Vm, VmExecutionParams, VmHostActionDef, VmHostCallKind,
};
use field::{Address, Amount, Encode, Hash};
use sys::{Rerr, Ret, errf};

/// In-memory KV layer backing `TestCtx::layer`.
#[derive(Default)]
pub struct MemLayer(pub HashMap<Vec<u8>, Vec<u8>>);

impl StateRead for MemLayer {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
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

fn stub_block_hasher(_height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
    [0u8; base::HASH_SIZE]
}

/// Registry-less services stub: no host defs, no VM assignment. Enough for
/// bytecode verification that does not reference ACTION/ACTENV/ACTVIEW ids.
pub struct StubServices;

impl BinaryCodecs for StubServices {
    fn decode_action(&self, _buf: &[u8]) -> Ret<(ActionRef, usize)> {
        errf!("stub services: decode_action")
    }
    fn decode_action_exact(&self, _buf: &[u8]) -> Ret<ActionRef> {
        errf!("stub services: decode_action_exact")
    }
    fn decode_transaction(&self, _buf: &[u8]) -> Ret<(TxRef, usize)> {
        errf!("stub services: decode_transaction")
    }
    fn decode_transaction_exact(&self, _buf: &[u8]) -> Ret<TxRef> {
        errf!("stub services: decode_transaction_exact")
    }
    fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
        errf!("stub services: decode_block")
    }
    fn decode_block_exact(&self, _buf: &[u8]) -> Ret<BlockRef> {
        errf!("stub services: decode_block_exact")
    }
    fn peek_block_size(&self, _buf: &[u8]) -> Ret<usize> {
        errf!("stub services: peek_block_size")
    }
    fn block_hash(&self, _height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }
    fn block_hasher_fn(&self) -> BlockHasherFn {
        stub_block_hasher
    }
}

impl JsonCodecs for StubServices {
    fn decode_tx_json(&self, _ty: u8, _json: &str) -> Ret<Option<TxRef>> {
        errf!("stub services: decode_tx_json")
    }

    fn decode_action_json(&self, _kind: u16, _json: &str) -> Ret<Option<ActionRef>> {
        errf!("stub services: decode_action_json")
    }
}

impl ExecutionServices for StubServices {
    fn assign_vm(&self, _height: u64) -> Option<Box<dyn Vm>> {
        None
    }
    fn vm_host_def(&self, _kind: VmHostCallKind, _id: u8) -> Option<&VmHostActionDef> {
        None
    }
    fn vm_host_defs(&self, _kind: VmHostCallKind) -> Vec<&VmHostActionDef> {
        vec![]
    }
    fn vm_params(&self) -> Ret<&VmExecutionParams> {
        errf!("stub services: vm_params")
    }
    fn execution_profile(&self) -> Ret<&'static (dyn std::any::Any + Send + Sync)> {
        errf!("stub services: execution_profile")
    }
    fn create_context(
        self: Arc<Self>,
        _env: Env,
        _chunk: StateChunkRef,
        _tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        errf!("stub services: create_context")
    }
}

/// Type-3 transaction stub carrying only a main address.
#[derive(Debug, Clone, Default)]
pub struct DummyTx(pub Address);

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
    fn hash(&self) -> Hash {
        Hash::default()
    }
    fn main(&self) -> Address {
        self.0
    }
    fn fee(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        errf!("stub tx: execute")
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Test context tracking `exec_from` explicitly so entry semantics can assert
/// the VM executes under `ExecFrom::Call` and restores the caller's value.
pub struct TestCtx {
    pub env: Env,
    pub tx: DummyTx,
    pub layer: MemLayer,
    pub exec_from: ExecFrom,
    pub tex: TexLedger,
    pub gas: i64,
}

impl TestCtx {
    pub fn new() -> Self {
        let mut env = Env::default();
        env.tx.ty = 3;
        let tx = DummyTx(Address::default());
        env.tx.main = tx.0;
        Self {
            env,
            tx,
            layer: MemLayer::default(),
            exec_from: ExecFrom::Top,
            tex: TexLedger::default(),
            gas: 1 << 30,
        }
    }

    pub fn charge(&mut self, gas: i64) -> Rerr {
        self.gas -= gas;
        if self.gas < 0 {
            return errf!("test ctx out of gas");
        }
        Ok(())
    }
}

impl Context for TestCtx {
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
    fn check_sign(&mut self, _adr: &Address) -> Rerr {
        Ok(())
    }
    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.layer
    }
    fn emit_log(&mut self, _entry: LogEntry) {}
    fn gas_remaining(&self) -> i64 {
        self.gas
    }
    fn gas_charge(&mut self, gas: i64) -> Rerr {
        self.charge(gas)
    }
    fn gas_rebate(&mut self, gas: i64) -> Rerr {
        self.gas += gas;
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
        errf!("stub ctx: action_call")
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
        Ok(())
    }
}
