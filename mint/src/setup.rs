//! mint  Registry `mint::setup::register`

use base::{RegistryWriter, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType};
use sys::{Rerr, errf};

use crate::action::asset::{AssetCreate, create_asset_create};
use crate::action::channel::{
    ChannelClose, ChannelOpen, create_channel_close, create_channel_open,
};
use crate::action::diamond::{DiamondMint, create_diamond_mint, decode_diamond_mint_json};
use crate::action::diamond_insc::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, create_dia_insc_action,
    decode_dia_insc_json,
};
use crate::tx_coinbase::{CoinbaseTx, create_coinbase};

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
    reg.register_tx(CoinbaseTx::TYPE, create_coinbase)?;
    base::register_regular_actions!(
        reg,
        create_channel_open => [ChannelOpen],
        create_channel_close => [ChannelClose],
        create_asset_create => [AssetCreate],
    )?;
    base::register_custom_actions!(
        reg,
        create_dia_insc_action,
        decode_dia_insc_json => [DiaInscPush, DiaInscClean, DiaInscEdit, DiaInscMove, DiaInscDrop],
    )?;
    base::register_custom_actions!(
        reg,
        create_diamond_mint,
        decode_diamond_mint_json => [DiamondMint],
    )?;

    register_action_def(reg, DiaInscEdit::KIND, DiaInscEdit::NAME, 5)?;
    Ok(())
}
