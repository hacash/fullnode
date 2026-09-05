//! Engine-level lifecycle tests (§10.2): listener abort, post-commit callback
//! abort, query tri-state and optimistic-consumer propagation, on in-memory backends.

#![cfg(test)]

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base::{
    ActOut, ActionRef, ApplyMode, BinaryCodecs, BlkPkg, Block, BlockAcceptStatus, BlockHasherFn,
    BlockProducer, BlockRef, ChainId, ChainListener, ChainView, Consensus, ConsensusNodeHooks,
    Context, DiskDB, Engine, EngineConfig, Env, ExecFrom, ExecutionServices, ForkChoice,
    JsonCodecs, LogEntry, MemDB, MintParams, PkgOrigin, PkgSource, STATE_READ_FAILED_CODE,
    StateChunkRef, StateLayer, StateRead, Store, TexLedger, Transaction, TxPolicy, TxRef, Vm,
    VmExecutionParams, VmHostActionDef, VmHostCallKind,
};
use field::{Address, Amount, Encode, Hash};
use sys::{Rerr, Ret, errf};

use crate::engine::ChainEngine;

// =============================================================
// Mocks
// =============================================================

/// A prelude-only transaction that executes without touching the context.
#[derive(Debug, Clone)]
struct TestTx {
    main: Address,
    fee: Amount,
}

impl TestTx {
    fn prelude() -> Self {
        Self {
            main: Address::default(),
            fee: Amount::zero(),
        }
    }
}

impl Encode for TestTx {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}

impl Transaction for TestTx {
    fn ty(&self) -> u8 {
        1
    }
    fn main(&self) -> Address {
        self.main
    }
    fn fee(&self) -> &Amount {
        &self.fee
    }
    fn is_block_prelude(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::TransactionSign for TestTx {
    fn hash(&self) -> Hash {
        Hash::default()
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn base::TransactionExecute> {
        Some(self)
    }
}

// `chain` is a fullnode-only crate: its `base` edge always carries `execute`,
// so the execute impl is unconditional here.
impl base::TransactionExecute for TestTx {
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        Ok(())
    }
}

/// A transaction whose execution fails with a core `Abort`.
#[derive(Debug)]
struct AbortTx;

impl Encode for AbortTx {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}

impl Transaction for AbortTx {
    fn ty(&self) -> u8 {
        2
    }
    fn main(&self) -> Address {
        Address::default()
    }
    fn fee(&self) -> &Amount {
        static ZERO: OnceLock<Amount> = OnceLock::new();
        ZERO.get_or_init(Amount::zero)
    }
    fn is_block_prelude(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::TransactionSign for AbortTx {
    fn hash(&self) -> Hash {
        Hash::from([0xab; 32])
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn base::TransactionExecute> {
        Some(self)
    }
}

impl base::TransactionExecute for AbortTx {
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        Err(sys::Error::abort("state backend down").with_code(STATE_READ_FAILED_CODE))
    }
}

/// A minimal block carrying its own merkle root (computed by the caller,
/// mirroring `verify::merkle_root` over `hash_with_fee`).
#[derive(Debug)]
struct TestBlock {
    height: u64,
    hash: Hash,
    prev_hash: Hash,
    timestamp: u64,
    mrklroot: Hash,
    txs: Vec<TxRef>,
}

impl Encode for TestBlock {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}

impl Block for TestBlock {
    fn version(&self) -> u8 {
        1
    }
    fn height(&self) -> u64 {
        self.height
    }
    fn hash(&self) -> Hash {
        self.hash
    }
    fn prev_hash(&self) -> Hash {
        self.prev_hash
    }
    fn mrklroot(&self) -> Hash {
        self.mrklroot
    }
    fn timestamp(&self) -> u64 {
        self.timestamp
    }
    fn transactions(&self) -> &[TxRef] {
        &self.txs
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Context stub: never used by the mock transactions, but `release_chunk`
/// must hand the chunk back for the apply path to commit it.
struct TestContext {
    chunk: StateChunkRef,
    services: Arc<dyn ExecutionServices>,
    env: Env,
}

impl TestContext {
    fn new(services: Arc<dyn ExecutionServices>, chunk: StateChunkRef, env: Env) -> Self {
        Self {
            chunk,
            services,
            env,
        }
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
        static TX: OnceLock<TestTx> = OnceLock::new();
        TX.get_or_init(TestTx::prelude)
    }
    fn exec_from(&self) -> ExecFrom {
        ExecFrom::Call
    }
    fn exec_from_set(&mut self, _from: ExecFrom) {}
    fn check_sign(&mut self, _adr: &Address) -> Rerr {
        Ok(())
    }
    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.chunk
    }
    fn emit_log(&mut self, _entry: LogEntry) {}
    fn gas_remaining(&self) -> i64 {
        0
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
    fn snapshot_volatile(&self) -> Box<dyn Any> {
        Box::new(())
    }
    fn restore_volatile(&mut self, _snap: Box<dyn Any>) {}
    fn action_call(&mut self, _kind: u16, _body: Vec<u8>) -> Ret<ActOut> {
        errf!("test context has no actions")
    }
    fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
        None
    }
    fn vm_put(&mut self, _vm: Box<dyn Vm>) {}
    fn as_context_mut(&mut self) -> &mut dyn Context {
        self
    }
    fn release_chunk(self: Box<Self>) -> Ret<StateChunkRef> {
        Ok(self.chunk)
    }
    fn tex_ledger(&self) -> &TexLedger {
        static LEDGER: OnceLock<TexLedger> = OnceLock::new();
        LEDGER.get_or_init(TexLedger::default)
    }
    fn p2sh_set(&mut self, _addr: Address, _p2sh: Box<dyn base::P2sh>) -> Rerr {
        Ok(())
    }
}

/// Execution services stub: no decoding, no VM; `create_context` returns the
/// test context that hands the chunk back untouched. Carries the VM execution
/// params so tests can install an active contract storage budget schedule.
struct TestServices {
    params: VmExecutionParams,
}

impl TestServices {
    fn new(params: VmExecutionParams) -> Self {
        Self { params }
    }
}

impl Default for TestServices {
    fn default() -> Self {
        static PARAMS: VmExecutionParams = VmExecutionParams {
            contract_store_perm_periods: 10_000,
            contract_storage_fee: base::ContractStorageFeeParams::disabled(),
            initial_fee_purity_floor: 100,
            fee_purity_reductions: &[],
            gas_budget_lookup: &base::GAS_BUDGET_LOOKUP_NONE,
            tx_gas_budget_cap_byte: 0,
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
        };
        Self::new(PARAMS)
    }
}

impl BinaryCodecs for TestServices {
    fn decode_action(&self, _buf: &[u8]) -> Ret<(ActionRef, usize)> {
        errf!("test services: decode_action")
    }
    fn decode_transaction(&self, _buf: &[u8]) -> Ret<(TxRef, usize)> {
        errf!("test services: decode_transaction")
    }
    fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
        errf!("test services: decode_block")
    }
    fn peek_block_size(&self, _buf: &[u8]) -> Ret<usize> {
        errf!("test services: peek_block_size")
    }
    fn block_hash(&self, _height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }
    fn block_hasher_fn(&self) -> BlockHasherFn {
        |_, _| [0u8; base::HASH_SIZE]
    }
}

impl JsonCodecs for TestServices {
    fn decode_action_json(&self, _json: &str) -> Ret<ActionRef> {
        errf!("test services: decode_action_json")
    }
}

impl ExecutionServices for TestServices {
    fn assign_vm(&self, _height: u64) -> Option<Box<dyn Vm>> {
        None
    }
    fn vm_host_def(&self, _kind: VmHostCallKind, _id: u8) -> Option<&VmHostActionDef> {
        None
    }
    fn vm_params(&self) -> Ret<&VmExecutionParams> {
        Ok(&self.params)
    }
    fn execution_profile(&self) -> Ret<&'static dyn base::ExecutionProfile> {
        static PROFILE: TestProfile = TestProfile;
        Ok(&PROFILE)
    }
    fn create_context(
        self: Arc<Self>,
        env: Env,
        chunk: StateChunkRef,
        _tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        Ok(Box::new(TestContext::new(self, chunk, env)))
    }
}

struct TestProfile;

/// Configurable consensus runtime: genesis plus optional failing
/// post-commit callbacks.
struct TestConsensus {
    genesis: BlockRef,
    on_block_accepted: Mutex<Option<Rerr>>,
}

impl TestConsensus {
    fn new(genesis: BlockRef) -> Self {
        Self {
            genesis,
            on_block_accepted: Mutex::new(None),
        }
    }
}

impl Consensus for TestConsensus {
    fn name(&self) -> &str {
        "test"
    }
    fn chain_id(&self) -> ChainId {
        ChainId::MAINNET
    }
    fn mint_params(&self) -> MintParams {
        MintParams {
            max_block_txs: 0,
            max_block_size: 0,
            max_tx_size: 0,
            difficulty_adjust_blocks: 0,
            difficulty_group_blocks: 0,
            each_block_target_time: 0,
        }
    }
    fn genesis_block(&self) -> BlockRef {
        self.genesis.clone()
    }
    fn on_block_accepted(&self, _pkg: &base::BlkPkg, _view: &dyn base::ChainView) -> Rerr {
        match self.on_block_accepted.lock().unwrap().take() {
            Some(result) => result,
            None => Ok(()),
        }
    }
}

impl ForkChoice for TestConsensus {}
impl TxPolicy for TestConsensus {}
impl BlockProducer for TestConsensus {}
impl ConsensusNodeHooks for TestConsensus {}

/// A listener whose `on_block_accepted` fails with an `Abort`.
struct AbortListener;

impl ChainListener for AbortListener {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) -> Rerr {
        Err(sys::Error::abort("listener state write failed").with_code("core_failed"))
    }
}

/// A listener that records being called.
struct FlagListener {
    called: Arc<AtomicBool>,
}

impl ChainListener for FlagListener {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) -> Rerr {
        self.called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// A listener that fails with an ordinary non-fatal error.
struct ErrorListener;

impl ChainListener for ErrorListener {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) -> Rerr {
        sys::errf!("listener transient failure")
    }
}

/// A store whose state-disk `try_write` fails after `fails` successes; the
/// genesis write counts as the first, so the engine opens and the first roll fails.
struct FailAfterStore {
    inner: Arc<dyn Store>,
    fails: Arc<AtomicUsize>,
}

impl Store for FailAfterStore {
    fn status(&self) -> Ret<base::ChainStatus> {
        self.inner.status()
    }
    fn state_status(&self) -> Ret<base::StateStatus> {
        self.inner.state_status()
    }
    fn state_get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        self.inner.state_get(key)
    }
    fn stable_state(&self) -> Arc<dyn StateRead> {
        self.inner.stable_state()
    }
    fn disk(&self) -> Arc<dyn DiskDB> {
        Arc::new(FailAfterDisk {
            inner: self.inner.disk(),
            fails: self.fails.clone(),
        })
    }
    fn block_store(&self) -> Arc<dyn base::BlockStore> {
        self.inner.block_store()
    }
    fn log_backend(&self) -> Arc<dyn base::LogBackend> {
        self.inner.log_backend()
    }
}

struct FailAfterDisk {
    inner: Arc<dyn DiskDB>,
    fails: Arc<AtomicUsize>,
}

impl DiskDB for FailAfterDisk {
    fn read(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        self.inner.read(key)
    }
    fn save(&self, key: &[u8], val: &[u8]) {
        self.inner.save(key, val);
    }
    fn remove(&self, key: &[u8]) {
        self.inner.remove(key);
    }
    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let attempts = self.fails.fetch_add(1, Ordering::SeqCst);
        if attempts >= 1 {
            return sys::errf!("disk full");
        }
        self.inner.try_write(memkv)
    }
}

// =============================================================
// Harness helpers
// =============================================================

fn genesis() -> BlockRef {
    Arc::new(TestBlock {
        height: 0,
        hash: Hash::from([0x11; 32]),
        prev_hash: Hash::default(),
        timestamp: 1,
        mrklroot: Hash::default(), // merkle_root([default]) == default
        txs: vec![Arc::new(TestTx::prelude())],
    })
}

fn test_config() -> EngineConfig {
    EngineConfig {
        // Every accepted block is immediately stable, so `roll_root` runs on
        // every insert (the post-commit callback paths are exercised).
        unstable_block: 0,
        ..EngineConfig::default()
    }
}

/// Config that keeps a window of blocks in the in-memory fork tree (no durable
/// root roll below `unstable_block`), used by the fork/reorg lifecycle tests.
fn fork_window_config() -> EngineConfig {
    EngineConfig {
        unstable_block: 64,
        recent_blocks: false,
        average_fee_purity: false,
        ..EngineConfig::default()
    }
}

fn open_engine(consensus: TestConsensus, store: Arc<dyn Store>) -> Arc<ChainEngine> {
    open_engine_with(consensus, store, VmExecutionParams::default())
}

fn open_engine_with(
    consensus: TestConsensus,
    store: Arc<dyn Store>,
    params: VmExecutionParams,
) -> Arc<ChainEngine> {
    open_engine_with_config(
        Arc::new(consensus) as Arc<dyn base::ConsensusRuntime>,
        store,
        params,
        test_config(),
    )
}

fn open_engine_with_config(
    consensus: Arc<dyn base::ConsensusRuntime>,
    store: Arc<dyn Store>,
    params: VmExecutionParams,
    config: EngineConfig,
) -> Arc<ChainEngine> {
    let services: Arc<dyn ExecutionServices> = Arc::new(TestServices::new(params));
    ChainEngine::open(services, config, consensus, store, sys::Waiter::new(), 0)
        .expect("engine must open")
}

fn open_default() -> Arc<ChainEngine> {
    open_engine(
        TestConsensus::new(genesis()),
        Arc::new(db::StoreInst::new()),
    )
}

fn pkg_at(height: u64, prev_hash: Hash) -> base::BlkPkg {
    let block = TestBlock {
        height,
        hash: Hash::from([height as u8; 32]),
        prev_hash,
        timestamp: 100 + height,
        mrklroot: Hash::default(),
        txs: vec![Arc::new(TestTx::prelude())],
    };
    base::BlkPkg::from_block(Arc::new(block), PkgSource::new(PkgOrigin::Broadcast))
}

/// Two-tx block: prelude + one executing transaction. The merkle root is the
/// `verify::merkle_root` of the pair (leaf hashes are `hash_with_fee`).
fn pkg_with_budget_use(
    height: u64,
    prev_hash: Hash,
    prev_timestamp: u64,
    budget_tx: TxRef,
) -> base::BlkPkg {
    let prelude: TxRef = Arc::new(TestTx::prelude());
    let mut layer = vec![prelude.hash(), budget_tx.hash()];
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2 + 1);
        for pair in layer.chunks(2) {
            let right = pair.get(1).unwrap_or(&pair[0]);
            let mut buf = Vec::with_capacity(64);
            buf.extend_from_slice(pair[0].as_ref());
            buf.extend_from_slice(right.as_ref());
            next.push(Hash::from(sys::calculate_hash(buf)));
        }
        layer = next;
    }
    let block = TestBlock {
        height,
        hash: Hash::from([height as u8; 32]),
        prev_hash,
        timestamp: prev_timestamp + 1,
        mrklroot: layer[0],
        txs: vec![prelude, budget_tx],
    };
    base::BlkPkg::from_block(Arc::new(block), PkgSource::new(PkgOrigin::Broadcast))
}

fn first_block(eng: &ChainEngine) -> base::BlkPkg {
    pkg_at(1, eng.latest_block().hash())
}

// =============================================================
// §10.2 behavior tests
// =============================================================

/// Test 1: a listener's ordinary error warns only; other listeners are still
/// notified and the block result is unchanged.
#[test]
fn listener_ordinary_error_keeps_block_accepted_and_notifies_others() {
    let eng = open_default();
    let called = Arc::new(AtomicBool::new(false));
    eng.add_chain_listener(Arc::new(ErrorListener)).unwrap();
    eng.add_chain_listener(Arc::new(FlagListener {
        called: called.clone(),
    }))
    .unwrap();

    let result = eng.discover_block(first_block(&eng)).unwrap();
    assert_eq!(result.status, BlockAcceptStatus::Accepted);
    assert!(
        called.load(Ordering::SeqCst),
        "other listeners must still be notified"
    );
    assert!(
        !eng.is_fatal(),
        "an ordinary listener error must not be fatal"
    );
}

/// Test 2: a listener `Abort` keeps the committed block Accepted, marks the
/// engine fatal, and later blocks are no longer processed.
#[test]
fn listener_abort_keeps_accepted_and_stops_future_blocks() {
    let eng = open_default();
    eng.add_chain_listener(Arc::new(AbortListener)).unwrap();

    let result = eng.discover_block(first_block(&eng)).unwrap();
    assert_eq!(result.status, BlockAcceptStatus::Accepted);
    assert!(eng.is_fatal(), "listener abort must mark the engine fatal");
    assert_eq!(
        eng.tree.head_height(),
        1,
        "the committed block must not be rolled back"
    );
    // The fatal state stops further work: the waiter is triggered.
    assert!(
        eng.discover_block(pkg_at(2, eng.latest_block().hash()))
            .is_err(),
        "no further block may be processed after fatal"
    );
}

/// Test 3: a post-commit consensus callback `Abort` returns Accepted, marks
/// the engine fatal, and never rolls back or retries the committed block (§4.2).
#[test]
fn post_commit_callback_abort_returns_accepted_and_marks_fatal() {
    let mut consensus = TestConsensus::new(genesis());
    consensus.on_block_accepted = Mutex::new(Some(Err(sys::Error::abort(
        "consensus auxiliary write failed",
    )
    .with_code("core_failed"))));
    let eng = open_engine(consensus, Arc::new(db::StoreInst::new()));

    let result = eng.discover_block(first_block(&eng)).unwrap();
    assert_eq!(
        result.status,
        BlockAcceptStatus::Accepted,
        "the committed block stays Accepted after a post-commit callback failure"
    );
    assert!(eng.is_fatal());
    assert_eq!(
        eng.tree.head_height(),
        1,
        "the committed block must not be rolled back"
    );
    // Not retried: the same block is not re-accepted after fatal.
    assert!(eng.discover_block(first_block(&eng)).is_err());
}

/// Test 5: a persist/root commit failure returns the original error and
/// stops every pipeline.
#[test]
fn root_commit_failure_returns_error_and_stops_pipeline() {
    let store = Arc::new(FailAfterStore {
        inner: Arc::new(db::StoreInst::new()),
        fails: Arc::new(AtomicUsize::new(0)),
    });
    let eng = open_engine(TestConsensus::new(genesis()), store);

    let err = eng.discover_block(first_block(&eng)).unwrap_err();
    assert!(err.is_abort(), "persist failure must stay Abort");
    assert_eq!(err.code(), Some("persist_failed"));
    assert!(eng.is_fatal());
    assert!(
        eng.discover_block(pkg_at(2, eng.latest_block().hash()))
            .is_err(),
        "the pipeline must stop after a root commit failure"
    );
}

/// Test 9: the query boundary distinguishes fatal (`Err` + EngineUnavailable)
/// from busy (`Ok(None)`); a fresh engine serves `Ok(Some(...))`.
#[test]
fn query_boundary_distinguishes_fatal_busy_and_ok() {
    let eng = open_default();
    assert!(eng.optimistic_canonical().unwrap().is_some());
    assert!(eng.state_canonical().unwrap().is_some());

    // Busy: a concurrent insert holds `inserting`, so `state_canonical`
    // reports `Ok(None)` (retry), never an error.
    let _guard = eng.inserting.lock().unwrap();
    assert!(eng.state_canonical().unwrap().is_none());
    drop(_guard);

    // Fatal: `Err` with `EngineUnavailable`, never `Ok(None)`.
    eng.mark_fatal();
    let err = eng.optimistic_canonical().unwrap_err();
    assert_eq!(err.code(), Some("engine_unavailable"));
    assert!(eng.state_canonical().is_err());
    assert!(eng.state_at_session(&Hash::default()).is_err());
}

/// Test 10: when the engine is fatal/stopping, the optimistic consumers return
/// `Err(EngineUnavailable)`, never a busy skip or ordinary execution failure.
#[test]
fn optimistic_consumers_propagate_engine_unavailable() {
    let eng = open_default();
    eng.mark_fatal();

    let tx: TxRef = Arc::new(TestTx::prelude());
    let err = eng.try_execute_tx(tx.clone()).unwrap_err();
    assert_eq!(err.code(), Some("engine_unavailable"));

    let err = eng.try_execute_batch(vec![tx], 1).unwrap_err();
    assert_eq!(err.code(), Some("engine_unavailable"));
}

/// An `Abort` during optimistic execution must reach the fatal boundary and
/// propagate, never be judged an ordinary execution failure.
#[test]
fn try_execute_tx_propagates_execution_abort_and_marks_fatal() {
    let eng = open_default();
    let err = eng.try_execute_tx(Arc::new(AbortTx)).unwrap_err();
    assert!(err.is_abort());
    assert_eq!(err.code(), Some(STATE_READ_FAILED_CODE));
    assert!(
        eng.is_fatal(),
        "an execution abort must mark the engine fatal"
    );
}

// =============================================================
// §9 contract storage budget lifecycle: engine-level acceptance.
// Uses an active discount schedule (H0 = 3, T = 3, R = 3000, C = 9000)
// so activation, refill, discount settlement and rollback can be observed
// through the real `discover` / `execute_block` path (A04/A11/A12).
// =============================================================

/// A user transaction that consumes `discount_bytes` from the current block
/// discount quota on execute — the same budget helper the real contract
/// deploy/update path calls (`consume_block_contract_storage_discount`).
#[derive(Debug)]
struct BudgetUseTx {
    discount_bytes: usize,
    fail_after_use: bool,
    hash: Hash,
}

impl Encode for BudgetUseTx {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}

impl Transaction for BudgetUseTx {
    fn ty(&self) -> u8 {
        2
    }
    fn main(&self) -> Address {
        Address::default()
    }
    fn fee(&self) -> &Amount {
        static ZERO: OnceLock<Amount> = OnceLock::new();
        ZERO.get_or_init(Amount::zero)
    }
    fn is_block_prelude(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::TransactionSign for BudgetUseTx {
    fn hash(&self) -> Hash {
        self.hash
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn base::TransactionExecute> {
        Some(self)
    }
}

impl base::TransactionExecute for BudgetUseTx {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        let vp = {
            let services = ctx.services();
            *services.vm_params()?
        };
        let height = ctx.env().block.height;
        let Some(snap) =
            base::read_block_contract_storage_snapshot(ctx.layer(), &vp, height)?
        else {
            return errf!("budget inactive at this height");
        };
        base::consume_block_contract_storage_discount(ctx.layer(), &snap, self.discount_bytes)?;
        if self.fail_after_use {
            return errf!("injected discount tx failure");
        }
        Ok(())
    }
}

/// Mainnet-shaped storage discount profile with a compact H0 for fast tests.
fn budget_test_params() -> VmExecutionParams {
    VmExecutionParams {
        contract_store_perm_periods: 10_000,
        contract_storage_fee: base::ContractStorageFeeParams {
            rule_version: base::CONTRACT_STORAGE_RULE_V1,
            activation_height: 3,
            target_capacity_blocks: 3,
            curve_steps: 1_000,
            period_floor: 10,
            max_block_discount_bytes: 16_384,
            supplement_schedule: &[(3, 3_000)],
        },
        initial_fee_purity_floor: 100,
        fee_purity_reductions: &[],
        gas_budget_lookup: &base::GAS_BUDGET_LOOKUP_NONE,
        tx_gas_budget_cap_byte: 0,
        compute_limit_byte: 0,
        resource_limit_byte: 0,
        storage_limit_byte: 0,
    }
}

fn open_budget_engine() -> Arc<ChainEngine> {
    open_engine_with(
        TestConsensus::new(genesis()),
        Arc::new(db::StoreInst::new()),
        budget_test_params(),
    )
}

/// Remaining discount budget at the canonical head, read from settled state.
fn head_remaining_budget(eng: &ChainEngine) -> Option<u128> {
    let session = eng
        .state_canonical()
        .ok()
        .flatten()
        .expect("canonical state must be readable after discover");
    base::read_contract_storage_budget(session.view())
        .unwrap()
        .map(|r| r.remaining_bytes.uint())
}

fn insert_next(eng: &ChainEngine) -> u64 {
    let next_height = eng.latest_height() + 1;
    let pkg = pkg_at(next_height, eng.latest_block().hash());
    let result = eng.discover_block(pkg).unwrap();
    assert_eq!(result.status, BlockAcceptStatus::Accepted);
    next_height
}

/// §9 rows through the real insert path: H0 activation with a missing record
/// (B0 = C, born full), idle blocks keep the cap, refill never exceeds C.
#[test]
fn budget_activates_and_fills_over_idle_blocks() {
    let eng = open_budget_engine();
    assert_eq!(head_remaining_budget(&eng), None);
    // pre-activation: heights 1 and 2 use legacy rules, no budget state
    assert_eq!(insert_next(&eng), 1);
    assert_eq!(insert_next(&eng), 2);
    assert_eq!(head_remaining_budget(&eng), None);
    // H0 = 3: missing record interpreted as B = C = 9000 (B0 = C)
    assert_eq!(insert_next(&eng), 3);
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
    // idle blocks stay at the cap: 4/5/6 all read 9000
    assert_eq!(insert_next(&eng), 4);
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
    assert_eq!(insert_next(&eng), 5);
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
    assert_eq!(insert_next(&eng), 6);
    assert_eq!(head_remaining_budget(&eng), Some(9_000), "budget caps at C");
}

/// A discount transaction consumes quota within its block; settlement accounts
/// it: B_next = min(C, B_start - D + R).
#[test]
fn budget_discount_tx_consumes_quota_through_block_settlement() {
    let eng = open_budget_engine();
    insert_next(&eng); // 1
    insert_next(&eng); // 2
    insert_next(&eng); // 3 -> B = 9000 (born full)
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
    // block 4 starts with B = 9000, one 4,000-byte discount tx:
    // settle 9000 - 4000 + 3000 = 8000 (< C, so the usage is visible)
    let tx: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 4_000,
        fail_after_use: false,
        hash: Hash::from([0x55; 32]),
    });
    let pkg = pkg_with_budget_use(4, eng.latest_block().hash(), 104, tx);
    let result = eng.discover_block(pkg).unwrap();
    assert_eq!(result.status, BlockAcceptStatus::Accepted);
    assert_eq!(head_remaining_budget(&eng), Some(9_000 - 4_000 + 3_000));
}

/// A block whose discount transaction fails is rejected whole: no budget write
/// and no partial settlement leaks into the state (§4.4 block-atomicity).
#[test]
fn budget_rolls_back_with_failed_block() {
    let eng = open_budget_engine();
    insert_next(&eng); // 1
    insert_next(&eng); // 2
    insert_next(&eng); // 3 -> B = 9000 (born full)
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
    let prev_hash = eng.latest_block().hash();
    let tx: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 1_000,
        fail_after_use: true,
        hash: Hash::from([0x66; 32]),
    });
    let pkg = pkg_with_budget_use(4, prev_hash, 104, tx);
    assert!(eng.discover_block(pkg).is_err(), "failing block must be rejected");
    assert_eq!(eng.latest_height(), 3, "no block must attach on failure");
    assert_eq!(head_remaining_budget(&eng), Some(9_000), "budget must not leak");
}

/// Pending admission paths must classify against the same budget snapshot:
/// `try_execute_tx` (single candidate) accepts a quota-fitting discount tx and
/// rejects an oversized one.
#[test]
fn pending_tx_admission_uses_the_same_budget_rules() {
    let eng = open_budget_engine();
    insert_next(&eng); // 1
    insert_next(&eng); // 2
    insert_next(&eng); // 3 -> B = 9000; next pending height = 4, B_start = 9000
    assert_eq!(head_remaining_budget(&eng), Some(9_000));

    let ok_tx: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 1_000,
        fail_after_use: false,
        hash: Hash::from([0x77; 32]),
    });
    eng.try_execute_tx(ok_tx).expect("1 KB fits the pending block quota");
    let big_tx: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 10_000, // exceeds quota min(9000, K_max)
        fail_after_use: false,
        hash: Hash::from([0x78; 32]),
    });
    assert!(eng.try_execute_tx(big_tx).is_err(), "oversized discount must be rejected");
    // batch admission shares the draft snapshot across txs: the full quota
    // 9,000 is drained by 5,000 + 4,000, so the trailing 1,000 fails
    let small: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 5_000,
        fail_after_use: false,
        hash: Hash::from([0x79; 32]),
    });
    let fits: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 4_000,
        fail_after_use: false,
        hash: Hash::from([0x7a; 32]),
    });
    let too_much_hash = Hash::from([0x7b; 32]);
    let too_much: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 1_000,
        fail_after_use: false,
        hash: too_much_hash,
    });
    let failed = eng.try_execute_batch(vec![small, fits, too_much], 4).unwrap();
    assert_eq!(failed, vec![too_much_hash], "only the quota-exceeding tx fails");
    // the pending runs never touch persisted budget state
    assert_eq!(head_remaining_budget(&eng), Some(9_000));
}

// =============================================================
// §A12/A16/A11 engine-level parity: strict vs fast-sync state root equality,
// and fork reorg rolling the discount budget back with the losing branch.
// =============================================================

use std::collections::BTreeMap;

/// Flatten the canonical head logical state (all block deltas from the root
/// chunk down to the head) into one key→value map, for cross-engine parity.
fn flatten_head_state(eng: &ChainEngine) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let (_, _, _, head_view, _) = eng.tree.head_snapshot();
    let mut layers: Vec<StateChunkRef> = Vec::new();
    let mut cur = Some(head_view);
    while let Some(chunk) = cur {
        layers.push(chunk.clone());
        cur = chunk.parent();
    }
    let mut map = BTreeMap::new();
    for chunk in layers.iter().rev() {
        if let Some(frozen) = chunk.frozen_state() {
            frozen.for_each(&mut |key, value| match value {
                Some(bytes) => {
                    map.insert(key.to_vec(), bytes.to_vec());
                }
                None => {
                    map.remove(key);
                }
            });
        }
    }
    map
}

fn open_budget_fork_engine(consensus: Arc<dyn base::ConsensusRuntime>) -> Arc<ChainEngine> {
    open_engine_with_config(
        consensus,
        Arc::new(db::StoreInst::new()),
        budget_test_params(),
        fork_window_config(),
    )
}

fn discover_linear(eng: &ChainEngine, pkg: &base::BlkPkg) {
    let result = eng.discover_block(pkg.clone()).unwrap();
    assert_eq!(result.status, BlockAcceptStatus::Accepted);
}

fn fast_sync_linear(eng: &ChainEngine, pkg: &base::BlkPkg) {
    crate::apply::insert_block(eng, pkg, ApplyMode::FastSync, true)
        .expect("fast-sync block must apply");
}

/// §A12: the same block sequence executed strictly and through the fast-sync
/// pipeline yields byte-identical head state (discount budget included).
#[test]
fn strict_and_fast_sync_converge_on_the_same_budget_state() {
    let eng_s = open_budget_fork_engine(Arc::new(TestConsensus::new(genesis())));
    let eng_f = open_budget_fork_engine(Arc::new(TestConsensus::new(genesis())));
    for h in 1..=3u64 {
        let ps = pkg_at(h, eng_s.latest_block().hash());
        let pf = pkg_at(h, eng_f.latest_block().hash());
        discover_linear(&eng_s, &ps);
        fast_sync_linear(&eng_f, &pf);
        assert_eq!(
            head_remaining_budget(&eng_s),
            head_remaining_budget(&eng_f),
            "budget must match at height {h}"
        );
    }
    // discount block on top of H0 (B0 = C, so a 4,000-byte discount tx settles
    // the full head to 9000 - 4000 + 3000 = 8000)
    let tx_s: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 4_000,
        fail_after_use: false,
        hash: Hash::from([0x41; 32]),
    });
    let pkg_s = pkg_with_budget_use(4, eng_s.latest_block().hash(), 105, tx_s.clone());
    discover_linear(&eng_s, &pkg_s);
    let pkg_f = pkg_with_budget_use(4, eng_f.latest_block().hash(), 105, tx_s);
    fast_sync_linear(&eng_f, &pkg_f);
    assert_eq!(head_remaining_budget(&eng_s), head_remaining_budget(&eng_f));
    assert_eq!(head_remaining_budget(&eng_s), Some(9_000 - 4_000 + 3_000));
    // identical logical state roots (all keys, all values)
    let state_s = flatten_head_state(&eng_s);
    let state_f = flatten_head_state(&eng_f);
    assert_eq!(state_s, state_f, "strict and fast-sync must converge to one state root");
}

/// Single-prelude block with an explicit block hash (used to give the reorging
/// branch a deterministic, higher fork-choice score).
fn pkg_at_hash(height: u64, prev_hash: Hash, hash: Hash, timestamp: u64) -> base::BlkPkg {
    let block = TestBlock {
        height,
        hash,
        prev_hash,
        timestamp,
        mrklroot: Hash::default(),
        txs: vec![Arc::new(TestTx::prelude())],
    };
    base::BlkPkg::from_block(Arc::new(block), PkgSource::new(PkgOrigin::Broadcast))
}

/// A consensus whose branch score is the raw block hash, so a competing higher-hash
/// block at the same height deterministically reorgs the head.
struct ForkedConsensus {
    inner: TestConsensus,
}

impl Consensus for ForkedConsensus {
    fn name(&self) -> &str {
        "forked"
    }
    fn chain_id(&self) -> ChainId {
        self.inner.chain_id()
    }
    fn mint_params(&self) -> MintParams {
        self.inner.mint_params()
    }
    fn genesis_block(&self) -> BlockRef {
        self.inner.genesis_block()
    }
    fn on_block_accepted(&self, pkg: &base::BlkPkg, view: &dyn base::ChainView) -> Rerr {
        self.inner.on_block_accepted(pkg, view)
    }
}
impl ForkChoice for ForkedConsensus {
    fn fork_choice_key(
        &self,
        block: &BlkPkg,
        _parent_key: &base::ForkChoiceKey,
        _history: &dyn base::BlockHistory,
    ) -> sys::Ret<base::ForkChoiceKey> {
        Ok(base::ForkChoiceKey::new(block.block().hash().as_bytes().to_vec()))
    }
}
impl TxPolicy for ForkedConsensus {}
impl BlockProducer for ForkedConsensus {}
impl ConsensusNodeHooks for ForkedConsensus {}

/// A two-tx block (prelude + discount tx) with an explicit block hash and mrklroot.
fn pkg_with_budget_use_at(
    height: u64,
    prev_hash: Hash,
    block_hash: Hash,
    timestamp: u64,
    budget_tx: TxRef,
) -> base::BlkPkg {
    let prelude: TxRef = Arc::new(TestTx::prelude());
    let mut layer = vec![prelude.hash(), budget_tx.hash()];
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2 + 1);
        for pair in layer.chunks(2) {
            let right = pair.get(1).unwrap_or(&pair[0]);
            let mut buf = Vec::with_capacity(64);
            buf.extend_from_slice(pair[0].as_ref());
            buf.extend_from_slice(right.as_ref());
            next.push(Hash::from(sys::calculate_hash(buf)));
        }
        layer = next;
    }
    let block = TestBlock {
        height,
        hash: block_hash,
        prev_hash,
        timestamp,
        mrklroot: layer[0],
        txs: vec![prelude, budget_tx],
    };
    base::BlkPkg::from_block(Arc::new(block), PkgSource::new(PkgOrigin::Broadcast))
}

/// §A11: when a fork reorg replaces the canonical head, the losing branch's
/// discount usage rolls back with its whole block state.
#[test]
fn fork_reorg_rolls_the_discount_budget_back_with_the_losing_branch() {
    let eng = open_budget_fork_engine(Arc::new(ForkedConsensus {
        inner: TestConsensus::new(genesis()),
    }));
    discover_linear(&eng, &pkg_at(1, eng.latest_block().hash()));
    discover_linear(&eng, &pkg_at(2, eng.latest_block().hash()));
    // H0 = 3 (B0 = C = 9,000) with a 6,000-byte discount tx: the activation block
    // itself spends more than the refill, so the head leaves the cap: 6,000.
    let prev2 = eng.latest_block().hash();
    let tx_h0: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 6_000,
        fail_after_use: false,
        hash: Hash::from([0x60; 32]),
    });
    let pkg_h0 = pkg_with_budget_use_at(3, prev2, Hash::from([0x05; 32]), 105, tx_h0);
    discover_linear(&eng, &pkg_h0);
    assert_eq!(head_remaining_budget(&eng), Some(9_000 - 6_000 + 3_000), "H0 spend leaves the cap");

    // Branch A (low hash): height 4 with a 1,000-byte discount tx → would settle
    // to 8,000 if canonical. Branch B (high hash): height 4 idle → settles to 9,000.
    let prev3 = eng.latest_block().hash();
    let tx_a: TxRef = Arc::new(BudgetUseTx {
        discount_bytes: 1_000,
        fail_after_use: false,
        hash: Hash::from([0x61; 32]),
    });
    let pkg_a = pkg_with_budget_use_at(4, prev3, Hash::from([0x10; 32]), 106, tx_a);
    discover_linear(&eng, &pkg_a);
    assert_eq!(head_remaining_budget(&eng), Some(6_000 - 1_000 + 3_000), "A is head first");
    assert_eq!(eng.latest_height(), 4);

    // Branch B: same parent, idle, with a block hash above A's → deterministic reorg.
    let pkg_b = pkg_at_hash(4, prev3, Hash::from([0xf0; 32]), 107);
    let accepted = eng.discover_block(pkg_b.clone()).unwrap();
    assert_eq!(accepted.status, BlockAcceptStatus::Accepted);
    assert_eq!(eng.latest_height(), 4);
    // The canonical head is now B: A's 1,000-byte discount usage rolled back with
    // its whole branch, so the head budget is the idle-B result (9,000), never A's
    // 8,000 or any partial leak.
    assert_eq!(
        head_remaining_budget(&eng),
        Some(6_000 + 3_000),
        "losing branch discount usage must roll back with the reorg"
    );
    // and the winning branch has no transient budget keys left (settled cleanly)
    let session = eng.state_canonical().ok().flatten().expect("head state");
    assert_eq!(
        base::peek_block_budget_remaining(session.view(), &budget_test_params(), 5).unwrap(),
        None,
        "no transient budget keys may survive settlement on the canonical head"
    );
}
