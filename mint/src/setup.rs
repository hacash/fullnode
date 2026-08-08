//! mint  Registry `mint::setup::register`

use base::{RegistryWriter, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType};
use sys::Rerr;

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

    reg.register_vm_host_def(VmHostActionDef {
        id: DiaInscEdit::KIND as u8,
        name: "hacd_insc_edit",
        kind: VmHostCallKind::Action,
        ret: VmValueType::Nil,
        argc: 5,
        pass_body: true,
        have_retv: false,
        allowed_policy: VmHostAllowedPolicy::TopOnly,
    })?;
    Ok(())
}
