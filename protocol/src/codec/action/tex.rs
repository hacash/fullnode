//! TexCellAct (kind 22) and TEX cell codecs.

use base::{ActScope, ActionRef, decode_regular_action};
#[cfg(feature = "execute")]
use field::Hash;
use field::{
    Address, AssetAmt, BlockHeight, Decode, DiamondNameListMax200, DiamondNumber, Encode, Fold64,
    FromJSON, ListW1, Sign, Uint2, Uint4, json_decode_value, json_split_object,
};
use sys::{Ret, errf};

macro_rules! define_tex_cells {
    ($( $variant:ident = $id:literal { $field:ident : $ty:ty } asset=$asset:literal ),+ $(,)?) => {
        #[derive(Debug, Clone)]
        pub(crate) enum TexCell {
            $( $variant { $field: $ty } ),+
        }

        impl TexCell {
            fn is_asset_transfer(&self) -> bool {
                match self {
                    $(Self::$variant { .. } => $asset),+
                }
            }
        }

        impl Encode for TexCell {
            fn size(&self) -> usize {
                1 + match self {
                    $(Self::$variant { $field } => $field.size()),+
                }
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                match self {
                    $(Self::$variant { $field } => {
                        field::Uint1::from($id).encode_to(out);
                        $field.encode_to(out);
                    }),+
                }
            }
        }

        impl Decode for TexCell {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut reader = field::Reader::new(buf);
                let cid: field::Uint1 = reader.read()?;
                let value = match cid.uint() {
                    $($id => Self::$variant { $field: reader.read()? }),+,
                    id => return errf!("cannot find tex cell id '{}'", id),
                };
                Ok((value, reader.used()))
            }
        }

        impl field::ToJSON for TexCell {
            fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
                match self {
                    $(Self::$variant { $field } => {
                        let mut json = format!("{{\"cellid\":{},\"{}\":", $id, stringify!($field));
                        json.push_str(&field::ToJSON::to_json_fmt($field, fmt));
                        json.push('}');
                        json
                    }),+
                }
            }
        }

        fn decode_tex_cell_json(json: &str) -> Ret<TexCell> {
            let mut fields = std::collections::HashMap::new();
            for (key, value) in json_split_object(json)? {
                if fields.insert(key, value).is_some() {
                    return sys::normalf!("TEX cell JSON field {} is duplicated", key);
                }
            }
            let cid: field::Uint1 = fields
                .get("cellid")
                .copied()
                .ok_or_else(|| sys::Error::normal("TEX cell JSON missing cellid"))
                .and_then(json_decode_value)?;
            match cid.uint() {
                $($id => {
                    let field_name = stringify!($field);
                    if let Some(unknown) = fields.keys().find(|key| **key != "cellid" && **key != field_name) {
                        return sys::normalf!("TEX cell {} JSON field {} is unknown", $id, unknown);
                    }
                    let raw = fields.get(field_name).copied().ok_or_else(|| {
                        sys::Error::normal(format!("TEX cell {} JSON missing {}", $id, field_name))
                    })?;
                    Ok(TexCell::$variant { $field: json_decode_value(raw)? })
                }),+,
                id => sys::normalf!("cannot find tex cell id '{}'", id),
            }
        }

        impl FromJSON for TexCell {
            fn from_json(&mut self, json: &str) -> Ret<()> {
                *self = decode_tex_cell_json(json)?;
                Ok(())
            }
        }
    };
}

define_tex_cells! {
    ZhuPay = 1 { haczhu: Fold64 } asset=false,
    ZhuGet = 2 { haczhu: Fold64 } asset=false,
    SatPay = 3 { satnum: Fold64 } asset=false,
    SatGet = 4 { satnum: Fold64 } asset=false,
    DiaPay = 5 { diamonds: DiamondNameListMax200 } asset=false,
    DiaGet = 6 { dianum: DiamondNumber } asset=false,
    AssetPay = 7 { asset: AssetAmt } asset=true,
    AssetGet = 8 { asset: AssetAmt } asset=true,
    CondZhuAtMost = 11 { haczhu: Fold64 } asset=false,
    CondZhuAtLeast = 12 { haczhu: Fold64 } asset=false,
    CondZhuEq = 13 { haczhu: Fold64 } asset=false,
    CondSatAtMost = 14 { satoshi: Fold64 } asset=false,
    CondSatAtLeast = 15 { satoshi: Fold64 } asset=false,
    CondSatEq = 16 { satoshi: Fold64 } asset=false,
    CondDiaAtMost = 17 { diamond: Fold64 } asset=false,
    CondDiaAtLeast = 18 { diamond: Fold64 } asset=false,
    CondDiaEq = 19 { diamond: Fold64 } asset=false,
    CondAssetAtMost = 20 { asset: AssetAmt } asset=false,
    CondAssetAtLeast = 21 { asset: AssetAmt } asset=false,
    CondAssetEq = 22 { asset: AssetAmt } asset=false,
    CondHeightAtMost = 23 { height: BlockHeight } asset=false,
    CondHeightAtLeast = 24 { height: BlockHeight } asset=false,
    CondChainIdEq = 25 { chainid: Uint4 } asset=false,
}

impl field::WireElementName for TexCell {
    const NAME: &'static str = "TexCell";
}
impl field::FieldWireShape for TexCell {
    const WIRE: field::FieldWire = field::FieldWire::Struct("TexCell");
}

impl Default for TexCell {
    fn default() -> Self {
        Self::ZhuPay {
            haczhu: Fold64::default(),
        }
    }
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct TexCellAct {
    pub kind: Uint2,
    pub addr: Address,
    pub(crate) cells: ListW1<TexCell>,
    pub sign: Sign,
}

impl TexCellAct {
    pub const KIND: u16 = 22;

    fn has_asset_transfer_cell(&self) -> bool {
        self.cells.iter().any(|c| c.is_asset_transfer())
    }

    #[cfg(feature = "execute")]
    pub(crate) fn get_sign_stuff(&self) -> Hash {
        let mut stf = Vec::with_capacity(self.addr.size() + self.cells.size());
        self.addr.encode_to(&mut stf);
        self.cells.encode_to(&mut stf);
        Hash::from(sys::calculate_hash(stf))
    }
}

base::impl_action_facts! {
    TexCellAct {
        name: "tex_cell_act",
        scope: ActScope::TOP,
        min_tx_type: 3,
        extra9: |this: &TexCellAct| this.has_asset_transfer_cell(),
        req_sign: |_: &TexCellAct| vec![],
        as_transfer_like: none,
        description: |this: &TexCellAct| {
            format!("Execute {} tex cells by {}", this.cells.len(), this.addr.to_readable())
        },

    }
}

pub fn create_tex_cell_act(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (kind_field, _) = Uint2::decode(buf)?;
    if kind_field.uint() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    decode_regular_action::<TexCellAct>(buf)
}

// ================================ wire schema ================================

impl base::StructSchemaProvider for TexCell {
    // TexCell is an enum (variant fields live in tex.rs's Encode/Decode); the schema
    // records it here so TS generation can expand the enum variants.
    const STRUCT_SCHEMA: base::StructSchema = base::StructSchema {
        name: "TexCell",
        fields: &[],
    };
}

pub const TEX_CELL_SCHEMA: base::StructSchema =
    <TexCell as base::StructSchemaProvider>::STRUCT_SCHEMA;
