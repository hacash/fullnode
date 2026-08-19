//! `mint-core::setup` — registration entries for inscription (32-36), channel
//! (2/3), asset (16) and diamond mint (4).
//!
//! `register_wire` installs the codec surface (binary + JSON + schema +
//! friendly family) and is shared by the full node, the WASM SDK and
//! `codec-schema-gen`; it matches the fullnode runtime action set exactly, so
//! the schema registration capture is naturally in sync with the runtime.
//! `register_exec` installs the EXTACTION host definition (DiaInscEdit) and is
//! called by the full node composition root only.

use base::{
    ExecRegistry, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType, WireRegistry,
};
use sys::{Rerr, errf};

use crate::action::asset::{AssetCreate, create_asset_create};
use crate::action::channel::{ChannelClose, ChannelOpen, create_channel_close, create_channel_open};
use crate::action::diamond::{
    DiamondMint, create_diamond_mint, decode_diamond_mint_json,
};
use crate::inscription::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, create_dia_insc_action,
    decode_dia_insc_json,
};

/// EXTACTION host defs: the wire id is the kind itself, so a kind larger than
/// `0xff` is rejected instead of silently truncating into the u8 opcode id.
fn register_action_def(
    reg: &mut dyn ExecRegistry,
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

/// Wire codec set: every mint-core action codec with its schema and friendly
/// family. The five inscription kinds are separate friendly variants, so the
/// shared inscription decoder is registered as five single-kind groups.
pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    base::register_custom_actions!(
        reg,
        "insc_push", create_dia_insc_action, decode_dia_insc_json => [DiaInscPush],
    )?;
    base::register_custom_actions!(
        reg,
        "insc_clean", create_dia_insc_action, decode_dia_insc_json => [DiaInscClean],
    )?;
    base::register_custom_actions!(
        reg,
        "insc_edit", create_dia_insc_action, decode_dia_insc_json => [DiaInscEdit],
    )?;
    base::register_custom_actions!(
        reg,
        "insc_move", create_dia_insc_action, decode_dia_insc_json => [DiaInscMove],
    )?;
    base::register_custom_actions!(
        reg,
        "insc_drop", create_dia_insc_action, decode_dia_insc_json => [DiaInscDrop],
    )?;
    base::register_regular_actions!(
        reg,
        "channel_open", create_channel_open => [ChannelOpen],
        "channel_close", create_channel_close => [ChannelClose],
        "asset_create", create_asset_create => [AssetCreate],
    )?;
    base::register_custom_actions!(
        reg,
        "diamond_mint",
        create_diamond_mint,
        decode_diamond_mint_json => [DiamondMint],
    )?;
    Ok(())
}

/// Execution services owned by this crate: the DiaInscEdit EXTACTION host
/// definition (its kind doubles as the opcode id).
pub fn register_exec(reg: &mut dyn ExecRegistry) -> Rerr {
    register_action_def(reg, DiaInscEdit::KIND, DiaInscEdit::NAME, 5)?;
    Ok(())
}
