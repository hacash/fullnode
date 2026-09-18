use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use base::{ExecutionServices, TransactionSign};
use field::{Address, Amount};
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
