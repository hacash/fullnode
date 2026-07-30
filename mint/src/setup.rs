//! mint  Registry `mint::setup::register`

use base::{RegistryWriter, VmHostActionDef, VmHostAllowedPolicy, VmHostCallKind, VmValueType};
use sys::Rerr;

use crate::action::asset::{AssetCreate, create_asset_create};
use crate::action::channel::{
    ChannelClose, ChannelOpen, create_channel_close, create_channel_open,
};
use crate::action::diamond::{DiamondMint, create_diamond_mint};
use crate::action::diamond_insc::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, create_dia_insc_clean,
    create_dia_insc_drop, create_dia_insc_edit, create_dia_insc_move, create_dia_insc_push,
};
use crate::tx_coinbase::{CoinbaseTx, create_coinbase};

pub fn register(reg: &mut dyn RegistryWriter) -> Rerr {
    reg.register_tx(CoinbaseTx::TYPE, create_coinbase)?;
    reg.register_action(&[ChannelOpen::KIND], create_channel_open)?;
    reg.register_action(&[ChannelClose::KIND], create_channel_close)?;
    reg.register_action(&[DiamondMint::KIND], create_diamond_mint)?;
    reg.register_action(&[DiaInscPush::KIND], create_dia_insc_push)?;
    reg.register_action(&[DiaInscClean::KIND], create_dia_insc_clean)?;
    reg.register_action(&[DiaInscEdit::KIND], create_dia_insc_edit)?;
    reg.register_action(&[DiaInscMove::KIND], create_dia_insc_move)?;
    reg.register_action(&[DiaInscDrop::KIND], create_dia_insc_drop)?;
    reg.register_action(&[AssetCreate::KIND], create_asset_create)?;

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
