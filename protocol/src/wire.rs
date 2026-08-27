use base::{ActionCodecBinding, StructSchema, TxCodecBinding, WireRegistry};
use field::{AddrHac, Sign};
use sys::Rerr;

use crate::codec::action::*;
use crate::codec::tx::*;

/// Protocol-owned transaction codecs (types 1/2/3). Coinbase lives in `mint`.
pub const TX_CODECS: &[TxCodecBinding] = &[
    TxCodecBinding {
        ty: hacash_params::TX_TYPE_1,
        decode_wire: create_transaction_type1,
    },
    TxCodecBinding {
        ty: hacash_params::TX_TYPE_2,
        decode_wire: create_transaction_type2,
    },
    TxCodecBinding {
        ty: hacash_params::TX_TYPE_3,
        decode_wire: create_transaction_type3,
    },
];

/// Complete protocol-owned action catalog, including VM env/view opcodes.
pub const ACTION_CODECS: &[ActionCodecBinding] = &[
    base::action_codec_binding!(TransferHacTo),
    base::action_codec_binding!(TransferHacFrom),
    base::action_codec_binding!(TransferHacFromTo),
    base::action_codec_binding!(TransferSatTo),
    base::action_codec_binding!(TransferSatFrom),
    base::action_codec_binding!(TransferSatFromTo),
    base::action_codec_binding!(TransferAssetTo),
    base::action_codec_binding!(TransferAssetFrom),
    base::action_codec_binding!(TransferAssetFromTo),
    base::action_codec_binding!(Message),
    base::action_codec_binding!(Blob),
    base::action_codec_binding!(ChainAllow),
    base::action_codec_binding!(HeightScope),
    base::action_codec_binding!(BalanceFloor),
    base::action_codec_binding!(RequiredSigners),
    base::action_codec_binding!(TransferHacdSingleTo),
    base::action_codec_binding!(TransferHacdFromTo),
    base::action_codec_binding!(TransferHacdTo),
    base::action_codec_binding!(TransferHacdFrom),
    base::action_codec_binding!(EnvHeight),
    base::action_codec_binding!(TxMainAddr),
    base::action_codec_binding!(BlockAuthorAddr),
    base::action_codec_binding!(BalanceCoin),
    base::action_codec_binding!(BalanceAsset),
    base::action_codec_binding!(CheckSignature),
    base::action_codec_binding!(HacdInscNum),
    base::action_codec_binding!(HacdInscGet),
    base::action_codec_binding!(HacdNameList),
    base::action_codec_binding!(HacdOwnerAddrs),
    base::action_codec_binding!(TxMessage),
    base::action_codec_binding!(TxBlob),
    base::action_codec_binding!(TxBlobSize),
    base::action_codec_binding!(TxMessageNum),
    base::action_codec_binding!(TxBlobNum),
    base::action_codec_binding!(AstSelect, create_ast_select, decode_ast_select_json),
    base::action_codec_binding!(AstIf, create_ast_if, decode_ast_if_json),
    base::action_codec_binding!(TexCellExecute),
];

/// Nested structs referenced by protocol action schemas.
pub const STRUCT_SCHEMAS: &[StructSchema] = &[
    TEX_CELL_SCHEMA,
    <AddrHac as base::StructSchemaProvider>::STRUCT_SCHEMA,
    <Sign as base::StructSchemaProvider>::STRUCT_SCHEMA,
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
