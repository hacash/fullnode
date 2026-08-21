//! Mint-core execution registration, used only by the fullnode composition.

use sys::Rerr;
use sys::errf;

use base::{ExecRegistry, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType};

use crate::inscription::DiaInscEdit;

/// Execution services owned by this crate: the DiaInscEdit EXTACTION host
/// definition (its kind doubles as the opcode id).
pub fn register_exec(reg: &mut dyn ExecRegistry) -> Rerr {
    if DiaInscEdit::KIND > 0xff {
        return errf!(
            "VM ACTION host {} kind {:#06x} cannot fit the u8 opcode id",
            DiaInscEdit::NAME,
            DiaInscEdit::KIND
        );
    }
    reg.register_vm_host_def(VmHostActionDef {
        id: DiaInscEdit::KIND as u8,
        name: DiaInscEdit::NAME,
        kind: VmHostCallKind::Action,
        ret: VmValueType::Nil,
        argc: 5,
        allowed_policy: VmHostAllowedPolicy::TopOnly,
    })
}
