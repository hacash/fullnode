use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use base::{ExecutionServices, TransactionSign};
use field::{Address, Amount};
use hacash_params::{HacashParams, ProtocolParams};
use sys::Ret;

use super::context::TestContext;
use super::logs::MemLogs;
use super::state::FlatMemState;
use super::tx::{StubTx, StubTxBuilder};

/// Serializes tests that mutate process-wide external state (for example a
/// caller's own registry cache). Testkit itself has no global setup.
pub fn test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub fn vm_main_addr() -> Address {
    Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").expect("valid test address")
}
pub fn vm_alt_addr() -> Address {
    Address::from_readable("1EuGe2GU8tDKnHLNfBsgyffx66buK7PP6g").expect("valid test address")
}

pub fn make_stub_tx(ty: u8, main: Address, addrs: Vec<Address>, gas_max: u8) -> StubTx {
    StubTxBuilder::new()
        .ty(ty)
        .main(main)
        .addrs(addrs)
        .fee(Amount::unit238(10_000_000))
        .gas_max(gas_max)
        .tx_size(128)
        .fee_purity(3_200)
        .build()
}

pub fn standard_services() -> Ret<Arc<dyn ExecutionServices>> {
    Ok(Arc::new(app::standard_registry()?))
}

// Trusted in-process profilers need to measure code above the production
// per-call compute/resource/storage ceilings.  Keep every other consensus
// parameter identical to mainnet, but disable only those three VM dimensions.
static UNBOUNDED_VM_PROFILE: HacashParams = HacashParams {
    protocol: ProtocolParams {
        vm: base::VmExecutionParams {
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
            ..hacash_params::MAINNET_PARAMS.protocol.vm
        },
        ..hacash_params::MAINNET_PARAMS.protocol
    },
    ..hacash_params::MAINNET_PARAMS
};

pub fn unbounded_vm_services() -> Ret<Arc<dyn ExecutionServices>> {
    Ok(Arc::new(app::standard_registry_with_params(
        &UNBOUNDED_VM_PROFILE,
    )?))
}

pub fn standard_context(tx: Arc<dyn TransactionSign>) -> Ret<TestContext> {
    Ok(TestContext::new(standard_services()?, tx))
}

/// Construct a fully VM-enabled in-memory context for integration tests.
/// `MemLogs` is accepted to keep call sites explicit even though `TestContext`
/// owns its execution log entries directly in the 1.0 context model.
pub fn make_ctx_from_tx(
    height: u64,
    tx: &StubTx,
    state: FlatMemState,
    _logs: MemLogs,
) -> TestContext {
    let services = standard_services().expect("standard test services");
    let mut ctx = TestContext::new(services.clone(), Arc::new(tx.clone()));
    ctx.layer = state;
    ctx.env.block.height = height;
    if let Some(vm) = services.assign_vm(height) {
        ctx.set_vm(vm);
    }
    ctx
}

/// Construct a normal in-memory context whose VM category ceilings are
/// disabled.  It is for trusted gas/timing profilers only, never consensus or
/// public sandbox tests.
pub fn make_ctx_from_tx_unbounded_vm(
    height: u64,
    tx: &StubTx,
    state: FlatMemState,
    _logs: MemLogs,
) -> TestContext {
    let services = unbounded_vm_services().expect("unbounded test services");
    let mut ctx = TestContext::new(services.clone(), Arc::new(tx.clone()));
    ctx.layer = state;
    ctx.env.block.height = height;
    if let Some(vm) = services.assign_vm(height) {
        ctx.set_vm(vm);
    }
    ctx
}
