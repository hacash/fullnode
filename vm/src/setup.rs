//! vm Registry `vm::setup::register`
//!
//! Installs:
//! - `vm_assigner` (assigns `StubVm` per height + registered host-action count)
//! - the four VM transaction actions (`ContractDeploy`/`ContractUpdate`/
//!   `ContractMainCall`/`P2SHScriptProve`) via `action::register_actions`
//!
//! Host capability metadata (which EXTACTION/env/view ids exist) is registered
//! by `protocol::register_standard`. This crate only installs `vm_assigner` + actions.

use base::{RegistryWriter, VmHostCallKind};
use sys::Rerr;

use crate::action::register_actions;
use crate::machine::StubVm;

pub fn register(reg: &mut dyn RegistryWriter) -> Rerr {
    reg.set_vm_assigner(|reg, height| {
        Box::new(StubVm::new(
            height,
            reg.vm_host_defs(VmHostCallKind::Action).len(),
        ))
    })?;
    register_actions(reg)?;
    Ok(())
}
