use base::{ActionCodecBinding, StructSchema, WireRegistry};
use sys::Rerr;

use crate::action::asset::{AssetCreate, create_asset_create};
use crate::action::channel::{
    ChannelClose, ChannelOpen, create_channel_close, create_channel_open,
};
use crate::action::diamond::{
    DiamondMint, DiamondMintData, create_diamond_mint, decode_diamond_mint_json,
};
use crate::inscription::{
    DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, create_dia_insc_action,
    decode_dia_insc_json,
};

/// Complete mint-core-owned action catalog.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(DiaInscPush, create_dia_insc_action, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscClean, create_dia_insc_action, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscEdit, create_dia_insc_action, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscMove, create_dia_insc_action, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscDrop, create_dia_insc_action, decode_dia_insc_json),
    base::action_codec_binding!(ChannelOpen, create_channel_open),
    base::action_codec_binding!(ChannelClose, create_channel_close),
    base::action_codec_binding!(AssetCreate, create_asset_create),
    base::action_codec_binding!(DiamondMint, create_diamond_mint, decode_diamond_mint_json),
];

/// Nested structs referenced by mint-core action schemas.
pub const STRUCT_SCHEMAS: &[StructSchema] = &[
    <field::AssetSmelt as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <DiamondMintData as base::StructSchemaProvider>::STRUCT_SCHEMA,
];

/// Installs the complete mint-core-owned wire surface into a dynamic profile.
pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    for binding in ACTION_CODECS {
        reg.register_action_codec(*binding)?;
    }
    Ok(())
}
