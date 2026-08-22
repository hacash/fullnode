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
    base::action_codec_binding!(HacToTrs),
    base::action_codec_binding!(HacFromTrs),
    base::action_codec_binding!(HacFromToTrs),
    base::action_codec_binding!(SatToTrs),
    base::action_codec_binding!(SatFromTrs),
    base::action_codec_binding!(SatFromToTrs),
    base::action_codec_binding!(AssetToTrs),
    base::action_codec_binding!(AssetFromTrs),
    base::action_codec_binding!(AssetFromToTrs),
    base::action_codec_binding!(TxMessage),
    base::action_codec_binding!(TxBlob),
    base::action_codec_binding!(ChainAllow),
    base::action_codec_binding!(HeightScope),
    base::action_codec_binding!(BalanceFloor),
    base::action_codec_binding!(ReqSignList),
    base::action_codec_binding!(DiaSingleTrs),
    base::action_codec_binding!(DiaFromToTrs),
    base::action_codec_binding!(DiaToTrs),
    base::action_codec_binding!(DiaFromTrs),
    base::action_codec_binding!(EnvHeight),
    base::action_codec_binding!(EnvMainAddr),
    base::action_codec_binding!(EnvBlockAuthorAddr),
    base::action_codec_binding!(ViewBalance),
    base::action_codec_binding!(ViewAssetBalance),
    base::action_codec_binding!(ViewCheckSign),
    base::action_codec_binding!(ViewDiaInscNum),
    base::action_codec_binding!(ViewDiaInscGet),
    base::action_codec_binding!(ViewDiaNameList),
    base::action_codec_binding!(ViewDiaOwnerAddrs),
    base::action_codec_binding!(AstSelect, create_ast_select, decode_ast_select_json),
    base::action_codec_binding!(AstIf, create_ast_if, decode_ast_if_json),
    base::action_codec_binding!(TexCellAct),
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
