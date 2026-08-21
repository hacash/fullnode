//! VM execution registration: installs the `vm_assigner` (`NativeVm` per height). The four VM
//! transaction actions are the static `ACTION_CODECS` catalog (`vm::register_wire`); only `vm_assigner` is installed here.

use base::ExecRegistry;
use sys::Rerr;

use crate::machine::NativeVm;

pub fn register_exec(reg: &mut dyn ExecRegistry) -> Rerr {
    reg.set_vm_assigner(|_reg, height| Box::new(NativeVm::new(height)))?;
    Ok(())
}
