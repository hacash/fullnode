use base::{ActionCodecBinding, StructSchema, TxCodecBinding, WireRegistry};
use field::AddrHac;
use sys::Rerr;

use crate::codec::action::*;
use crate::codec::tx::*;

/// Protocol-owned transaction codecs (types 1/2/3). Coinbase lives in `mint`.
pub const TX_CODECS: &[TxCodecBinding] = &[
    TxCodecBinding {
        ty: TransactionType1::TYPE,
        decode_wire: create_transaction_type1,
    },
    TxCodecBinding {
        ty: TransactionType2::TYPE,
        decode_wire: create_transaction_type2,
    },
    TxCodecBinding {
        ty: TransactionType3::TYPE,
        decode_wire: create_transaction_type3,
    },
];

/// Complete protocol-owned action catalog, including VM env/view opcodes.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(HacToTrs, create_hac_transfer),
    base::action_codec_binding!(HacFromTrs, create_hac_transfer),
    base::action_codec_binding!(HacFromToTrs, create_hac_transfer),
    base::action_codec_binding!(SatToTrs, create_sat_transfer),
    base::action_codec_binding!(SatFromTrs, create_sat_transfer),
    base::action_codec_binding!(SatFromToTrs, create_sat_transfer),
    base::action_codec_binding!(AssetToTrs, create_asset_transfer),
    base::action_codec_binding!(AssetFromTrs, create_asset_transfer),
    base::action_codec_binding!(AssetFromToTrs, create_asset_transfer),
    base::action_codec_binding!(TxMessage, create_blob_action),
    base::action_codec_binding!(TxBlob, create_blob_action),
    base::action_codec_binding!(ChainAllow, create_chain_guard_action),
    base::action_codec_binding!(HeightScope, create_chain_guard_action),
    base::action_codec_binding!(BalanceFloor, create_chain_guard_action),
    base::action_codec_binding!(ReqSignList, create_chain_guard_action),
    base::action_codec_binding!(DiaSingleTrs, create_diamond_transfer),
    base::action_codec_binding!(DiaFromToTrs, create_diamond_transfer),
    base::action_codec_binding!(DiaToTrs, create_diamond_transfer),
    base::action_codec_binding!(DiaFromTrs, create_diamond_transfer),
    base::action_codec_binding!(EnvHeight, create_envfunc_action),
    base::action_codec_binding!(EnvMainAddr, create_envfunc_action),
    base::action_codec_binding!(EnvBlockAuthorAddr, create_envfunc_action),
    base::action_codec_binding!(ViewBalance, create_envfunc_action),
    base::action_codec_binding!(ViewAssetBalance, create_envfunc_action),
    base::action_codec_binding!(ViewCheckSign, create_envfunc_action),
    base::action_codec_binding!(ViewDiaInscNum, create_envfunc_action),
    base::action_codec_binding!(ViewDiaInscGet, create_envfunc_action),
    base::action_codec_binding!(ViewDiaNameList, create_envfunc_action),
    base::action_codec_binding!(ViewDiaOwnerAddrs, create_envfunc_action),
    base::action_codec_binding!(AstSelect, create_ast_select, decode_ast_select_json),
    base::action_codec_binding!(AstIf, create_ast_if, decode_ast_if_json),
    base::action_codec_binding!(TexCellAct, create_tex_cell_act),
];

/// Nested structs referenced by protocol action schemas.
pub const STRUCT_SCHEMAS: &[StructSchema] = &[
    TEX_CELL_SCHEMA,
    <AddrHac as base::StructSchemaProvider>::STRUCT_SCHEMA,
];

/// Installs the complete protocol-owned wire surface into a dynamic profile.
pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    for binding in TX_CODECS {
        reg.register_tx_codec(*binding)?;
    }
    for binding in ACTION_CODECS {
        reg.register_action_codec(*binding)?;
    }
    Ok(())
}
