//! TexCellExecute (kind 22) and TEX cell codecs.

#[cfg(feature = "execute")]
use field::Hash;
use field::{
    Address, AssetAmt, BlockHeight, Decode, DiamondNameListMax200, DiamondNumber, Encode, Fold64,
    FromJSON, ListW1, Sign, Uint4, json_decode_value, json_split_object,
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
            // Field lists are short (cellid + one payload field); a `Vec` with
            // linear lookup keeps the hash-table machinery out of the wasm graph.
            let mut fields: Vec<(&str, &str)> = Vec::new();
            for (key, value) in json_split_object(json)? {
                if fields.iter().any(|(k, _)| *k == key) {
                    return sys::normalf!("TEX cell JSON field {} is duplicated", key);
                }
                fields.push((key, value));
            }
            let cid: field::Uint1 = fields
                .iter()
                .find(|(k, _)| *k == "cellid")
                .map(|(_, v)| *v)
                .ok_or_else(|| sys::Error::normal("TEX cell JSON missing cellid"))
                .and_then(json_decode_value)?;
            match cid.uint() {
                $($id => {
                    let field_name = stringify!($field);
                    if let Some(unknown) = fields.iter().find(|(k, _)| *k != "cellid" && *k != field_name) {
                        return sys::normalf!("TEX cell {} JSON field {} is unknown", $id, unknown.0);
                    }
                    let raw = fields
                        .iter()
                        .find(|(k, _)| *k == field_name)
                        .map(|(_, v)| *v)
                        .ok_or_else(|| {
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

base::action_simple! { TexCellExecute, 22, 3, TOP, {
    addr: Address,
    pub(crate) cells: ListW1<TexCell>,
    sign: Sign
}, this, {
    extra9: this.has_asset_transfer_cell(),
    ctor: none,
    description: format!("Execute {} tex cells by {}", this.cells.len(), this.addr.to_readable())
}}

impl TexCellExecute {
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

#[cfg(test)]
mod tests {
    use super::*;
    use field::{Decode, Encode, Fold64, FromJSON, ListW1, ToJSON};

    fn sample_sign() -> Sign {
        Sign {
            publickey: [0x02; Sign::PUBLICKEY_SIZE].into(),
            signature: [0xab; Sign::SIGNATURE_SIZE].into(),
        }
    }

    fn sample_action() -> TexCellExecute {
        TexCellExecute {
            kind: field::Uint2::from(TexCellExecute::KIND),
            addr: Address::default(),
            cells: ListW1::from(vec![TexCell::ZhuPay {
                haczhu: Fold64::from(1).unwrap(),
            }])
            .unwrap(),
            sign: sample_sign(),
        }
    }

    #[test]
    fn tex_cell_execute_json_uses_sign_object_and_rejects_hex_blob() {
        let action = sample_action();
        let json = action.to_json();
        assert!(json.contains("\"publickey\""), "{json}");
        assert!(json.contains("\"signature\""), "{json}");

        let mut back = sample_action();
        back.from_json(&json).unwrap();
        assert_eq!(back.encode(), action.encode());

        let sign_json = action.sign.to_json();
        let hex = format!("\"0x{}\"", hex::encode(action.sign.encode()));
        let as_hex = json.replace(&sign_json, &hex);
        assert!(
            TexCellExecute::default().from_json(&as_hex).is_err(),
            "{as_hex}"
        );

        let only_pk = format!(
            "{{\"publickey\":\"0x{}\"}}",
            hex::encode(action.sign.publickey)
        );
        let missing_sig = json.replace(&sign_json, &only_pk);
        assert!(
            TexCellExecute::default().from_json(&missing_sig).is_err(),
            "{missing_sig}"
        );
    }

    #[test]
    fn tex_cell_wire_roundtrip_size_and_unknown_variant() {
        let action = sample_action();
        assert_eq!(action.size(), action.encode().len());
        let wire = action.encode();
        let (decoded, used) = TexCellExecute::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);

        let mut wrong_kind = wire.clone();
        wrong_kind[1] = 1;
        assert!(TexCellExecute::decode(&wrong_kind).is_err());

        let cell = TexCell::ZhuPay {
            haczhu: Fold64::from(1).unwrap(),
        };
        assert_eq!(cell.size(), cell.encode().len());
        let mut cell_wire = cell.encode();
        cell_wire[0] = 99;
        assert!(TexCell::decode(&cell_wire).is_err());
    }
}
