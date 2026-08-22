//! Mint-core execution registration, used only by the fullnode composition.

use sys::Rerr;

use base::{ExecRegistry, VmHostActionDef};

use crate::inscription::DiaInscEdit;

/// Execution services owned by this crate: the DiaInscEdit EXTACTION host
/// definition (its kind doubles as the opcode id).
pub fn register_exec(reg: &mut dyn ExecRegistry) -> Rerr {
    reg.register_vm_host_def(VmHostActionDef::action_host(
        DiaInscEdit::KIND,
        DiaInscEdit::NAME,
        5,
    )?)
}
