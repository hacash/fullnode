//! `mint-core::setup::register` —— 铭刻动作（32-36）注册入口。
//!
//! 供全节点（app/mint）与 WASM SDK 共用：SDK 的 `RegistryWriter` 忽略
//! VM host 定义（`register_vm_host_def` 为 no-op），因此同一入口两端可用。

use base::{
    RegistryWriter, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType,
};
use sys::{Rerr, errf};

use crate::inscription::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, create_dia_insc_action,
    decode_dia_insc_json,
};

/// EXTACTION host defs: the wire id is the kind itself, so a kind larger than
/// `0xff` is rejected instead of silently truncating into the u8 opcode id.
fn register_action_def(
    reg: &mut dyn RegistryWriter,
    kind: u16,
    name: &'static str,
    argc: usize,
) -> Rerr {
    if kind > 0xff {
        return errf!(
            "VM ACTION host {} kind {:#06x} cannot fit the u8 opcode id",
            name,
            kind
        );
    }
    reg.register_vm_host_def(VmHostActionDef {
        id: kind as u8,
        name,
        kind: VmHostCallKind::Action,
        ret: VmValueType::Nil,
        argc,
        allowed_policy: VmHostAllowedPolicy::TopOnly,
    })
}

pub fn register(reg: &mut dyn RegistryWriter) -> Rerr {
    base::register_custom_actions!(
        reg,
        create_dia_insc_action,
        decode_dia_insc_json => [DiaInscPush, DiaInscClean, DiaInscEdit, DiaInscMove, DiaInscDrop],
    )?;
    register_action_def(reg, DiaInscEdit::KIND, DiaInscEdit::NAME, 5)?;
    Ok(())
}
