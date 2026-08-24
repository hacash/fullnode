use base::{ActionCodecBinding, StructSchema, WireRegistry};
use sys::Rerr;

use crate::action::asset::AssetCreate;
use crate::action::channel::{ChannelClose, ChannelOpen};
use crate::action::diamond::{HacdMint, HacdMintData, create_hacd_mint, decode_hacd_mint_json};
use crate::inscription::{HacdInscClean, HacdInscDrop, HacdInscEdit, HacdInscMove, HacdInscPush, decode_hacd_insc_json};

/// Complete mint-core-owned action catalog.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(HacdInscPush, decode_hacd_insc_json),
    base::action_codec_binding!(HacdInscClean, decode_hacd_insc_json),
    base::action_codec_binding!(HacdInscEdit, decode_hacd_insc_json),
    base::action_codec_binding!(HacdInscMove, decode_hacd_insc_json),
    base::action_codec_binding!(HacdInscDrop, decode_hacd_insc_json),
    base::action_codec_binding!(ChannelOpen),
    base::action_codec_binding!(ChannelClose),
    base::action_codec_binding!(AssetCreate),
    base::action_codec_binding!(HacdMint, create_hacd_mint, decode_hacd_mint_json),
];

/// Nested structs referenced by mint-core action schemas.
pub const STRUCT_SCHEMAS: &[StructSchema] = &[
    <field::AssetSmelt as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <HacdMintData as base::StructSchemaProvider>::STRUCT_SCHEMA,
];

/// Installs the complete mint-core-owned wire surface into a dynamic profile.
pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    for binding in ACTION_CODECS {
        reg.register_action_codec(*binding)?;
    }
    Ok(())
}
