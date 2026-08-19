//! vm Registry `vm::setup::register`
//!
//! Installs the `vm_assigner` (assigns `StubVm` per height). The four VM
//! transaction actions (`ContractDeploy`/`ContractUpdate`/`ContractMainCall`/
//! `P2SHScriptProve`) are part of the chain codec surface and are registered
//! by `chain-codec::register_standard` (`action::register_actions`), so the
//! SDK and the full node assemble the same action set.
//!
//! Host capability metadata (which EXTACTION/env/view ids exist) is registered
//! by `protocol::register_standard`. This crate only installs `vm_assigner`.

use base::ExecRegistry;
use sys::Rerr;

use crate::machine::StubVm;

pub fn register(reg: &mut dyn ExecRegistry) -> Rerr {
    reg.set_vm_assigner(|_reg, height| Box::new(StubVm::new(height)))?;
    Ok(())
}
