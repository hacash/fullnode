//! Native VM adapter.
//!
//! The VM is entered through `base::VmEntry`. Raw entries carry VM-owned request
//! data; transfer notifications from Call-path ACTIONs are driven directly by
//! the VM's recursive interpreter path, while Top/Ast entries retain the
//! protocol dispatcher bridge.

use std::sync::Arc;
use std::time::Instant;

use base::{GasBuckets, IntentScope};

use crate::rt::{AbstCall, CodeType, EntryKind};
use crate::value::{ContractAddress, Value};

mod deferred;
mod entry;
mod host;
mod intent;
mod interpreter;
mod limits;
mod loader;
mod runtime;
mod sandbox;
mod service;
mod transfer;

pub use deferred::DeferredRegistry;
pub use host::VmHost;
pub use intent::{IntentRuntime, IntentRuntimeLimits};
pub(crate) use interpreter::VmMachine;
pub use limits::peek_vm_runtime_limits;
pub(crate) use loader::ResolvedCallPlan;
pub use runtime::{Runtime, VolatileState};
pub use sandbox::{
    SANDBOX_TX_FEE, SandboxSpec, parse_sandbox_params, resolve_sandbox_gas, sandbox_call,
};
#[allow(unused_imports)]
pub use sandbox::{SandboxResult, build_call_codes};

pub enum VmRequest {
    Main {
        code_type: CodeType,
        codes: Arc<[u8]>,
    },
    SandboxMain {
        code_type: CodeType,
        codes: Arc<[u8]>,
    },
    Abst {
        kind: AbstCall,
        contract_addr: ContractAddress,
        intent_scope: IntentScope,
        param: Value,
    },
}

#[derive(Clone, Copy)]
struct EntryFrame {
    kind: EntryKind,
    gas_base: GasBuckets,
    call_base: i64,
}

pub struct StubVm {
    runtime: Runtime,
    entries: Vec<EntryFrame>,
    host_action_count: usize,
    deadline: Option<Instant>,
}
