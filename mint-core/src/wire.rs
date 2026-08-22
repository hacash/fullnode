use base::{ActionCodecBinding, StructSchema, WireRegistry};
use sys::Rerr;

use crate::action::asset::AssetCreate;
use crate::action::channel::{ChannelClose, ChannelOpen};
use crate::action::diamond::{DiamondMint, DiamondMintData, create_diamond_mint, decode_diamond_mint_json};
use crate::inscription::{DiaInscClean, DiaInscDrop, DiaInscEdit, DiaInscMove, DiaInscPush, decode_dia_insc_json};

/// Complete mint-core-owned action catalog.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(DiaInscPush, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscClean, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscEdit, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscMove, decode_dia_insc_json),
    base::action_codec_binding!(DiaInscDrop, decode_dia_insc_json),
    base::action_codec_binding!(ChannelOpen),
    base::action_codec_binding!(ChannelClose),
    base::action_codec_binding!(AssetCreate),
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
