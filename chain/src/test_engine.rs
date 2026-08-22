//! Engine-level lifecycle tests (§10.2): listener abort, post-commit callback
//! abort, query tri-state and optimistic-consumer propagation, on in-memory backends.

#![cfg(test)]

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base::{
    ActOut, ActionRef, BinaryCodecs, Block, BlockAcceptStatus, BlockHasherFn, BlockProducer,
    BlockRef, ChainId, ChainListener, ChainView, Consensus, ConsensusNodeHooks, Context, DiskDB,
    Engine, EngineConfig, Env, ExecFrom, ExecutionServices, ForkChoice, JsonCodecs, LogEntry,
    MemDB, MintParams, PkgOrigin, PkgSource, STATE_READ_FAILED_CODE, StateChunkRef, StateLayer,
    StateRead, Store, TexLedger, Transaction, TxPolicy, TxRef, Vm, VmExecutionParams,
    VmHostActionDef, VmHostCallKind,
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

/// A minimal block with one prelude transaction.
#[derive(Debug)]
struct TestBlock {
    height: u64,
    hash: Hash,
    prev_hash: Hash,
    timestamp: u64,
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
        // Single-tx merkle root is the tx hash itself (see `verify::merkle_root`).
        self.txs.first().map(|tx| tx.hash()).unwrap_or_default()
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
}

impl TestContext {
    fn new(chunk: StateChunkRef) -> Self {
        Self { chunk }
    }
}

fn test_env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| Env {
        chain: Default::default(),
        block: Default::default(),
        tx: Default::default(),
    })
}

impl Context for TestContext {
    fn services(&self) -> Arc<dyn ExecutionServices> {
        Arc::new(TestServices)
    }
    fn env(&self) -> &Env {
        test_env()
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
/// test context that hands the chunk back untouched.
struct TestServices;

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
    fn decode_action_json(&self, _kind: u16, _json: &str) -> Ret<Option<ActionRef>> {
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
        static PARAMS: VmExecutionParams = VmExecutionParams {
            contract_store_perm_periods: 10_000,
            initial_fee_purity_floor: 100,
            fee_purity_reductions: &[],
        };
        Ok(&PARAMS)
    }
    fn execution_profile(&self) -> Ret<&'static dyn base::ExecutionProfile> {
        static PROFILE: TestProfile = TestProfile;
        Ok(&PROFILE)
    }
    fn create_context(
        self: Arc<Self>,
        _env: Env,
        chunk: StateChunkRef,
        _tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        Ok(Box::new(TestContext::new(chunk)))
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

fn open_engine(consensus: TestConsensus, store: Arc<dyn Store>) -> Arc<ChainEngine> {
    let services: Arc<dyn ExecutionServices> = Arc::new(TestServices);
    ChainEngine::open(
        services,
        test_config(),
        Arc::new(consensus),
        store,
        sys::Waiter::new(),
        0,
    )
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
        txs: vec![Arc::new(TestTx::prelude())],
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
