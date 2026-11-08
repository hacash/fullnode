//! TexCellExecute (kind 22) and TEX cell codecs.

use field::Hash;
use field::{
    json_decode_value, json_split_object, AddrOrPtr, Address, AssetAmt, BlockHeight, Decode,
    DiamondNameListMax200, DiamondNumber, Encode, Fold64, FromJSON, ListW1, Sign, Uint4,
};
use sys::{Account, errf, Ret};

macro_rules! define_tex_cells {
    ($( $variant:ident = $id:literal { $( $field:ident : $ty:ty ),+ $(,)? } asset=$asset:literal ),+ $(,)?) => {
        #[derive(Debug, Clone)]
        /// One TEX settlement or condition cell. This is public so trusted
        /// wallet/test tooling can assemble a signed `TexCellExecute` without
        /// relying on wire-format internals.
        pub enum TexCell {
            $( $variant { $( $field: $ty ),+ } ),+
        }

impl TexCell {
    fn is_asset_transfer(&self) -> bool {
        match self {
            $(Self::$variant { .. } => $asset),+
        }
    }
}

/// Per-cell one-line behavior rendering for the `TexCellExecute` description.
/// Pay/get cells move value between the executing address and the TEX escrow;
/// condition cells only compare and never move anything. `zhu` amounts render
/// as exact HAC decimals; everything else keeps its integer wire unit.
impl TexCell {
    pub(crate) fn describe(&self) -> String {
        let hac = |zhu: &Fold64| field::Amount::zhu(zhu.uint()).to_hac_string();
        match self {
            Self::ZhuPay { haczhu } => format!("pay {} HAC into TEX escrow", hac(haczhu)),
            Self::ZhuGet { haczhu } => format!("get {} HAC from TEX escrow", hac(haczhu)),
            Self::SatPay { satnum } => format!("pay {} SAT into TEX escrow", satnum.uint()),
            Self::SatGet { satnum } => format!("get {} SAT from TEX escrow", satnum.uint()),
            Self::DiaPay { diamonds } => {
                format!("commit HACD ({}) for settlement delivery", diamonds.splitstr())
            }
            Self::DiaGet { dianum } => format!("get {} HACD from TEX escrow", dianum.uint()),
            Self::AssetPay { asset } => format!(
                "pay {} atoms of Asset #{} into TEX escrow",
                asset.amount.uint(),
                asset.serial.uint()
            ),
            Self::AssetGet { asset } => format!(
                "get {} atoms of Asset #{} from TEX escrow",
                asset.amount.uint(),
                asset.serial.uint()
            ),
            Self::DirectedZhuPay { to, haczhu } => format!(
                "pay {} HAC directly to {}",
                hac(haczhu),
                field::descfmt::addr_or_ptr_text(to)
            ),
            Self::DirectedZhuGet { from, haczhu } => format!(
                "take {} HAC directly from {} (the source must be a transaction signer)",
                hac(haczhu),
                field::descfmt::addr_or_ptr_text(from)
            ),
            Self::DirectedSatPay { to, satnum } => format!(
                "pay {} SAT directly to {}",
                satnum.uint(),
                field::descfmt::addr_or_ptr_text(to)
            ),
            Self::DirectedSatGet { from, satnum } => format!(
                "take {} SAT directly from {} (the source must be a transaction signer)",
                satnum.uint(),
                field::descfmt::addr_or_ptr_text(from)
            ),
            Self::DirectedDiaPay { to, diamonds } => format!(
                "pay HACD ({}) directly to {}",
                diamonds.splitstr(),
                field::descfmt::addr_or_ptr_text(to)
            ),
            Self::DirectedDiaGet { from, diamonds } => format!(
                "take HACD ({}) directly from {} (the source must be a transaction signer)",
                diamonds.splitstr(),
                field::descfmt::addr_or_ptr_text(from)
            ),
            Self::DirectedAssetPay { to, asset } => format!(
                "pay {} atoms of Asset #{} directly to {}",
                asset.amount.uint(),
                asset.serial.uint(),
                field::descfmt::addr_or_ptr_text(to)
            ),
            Self::DirectedAssetGet { from, asset } => format!(
                "take {} atoms of Asset #{} directly from {} (the source must be a transaction signer)",
                asset.amount.uint(),
                asset.serial.uint(),
                field::descfmt::addr_or_ptr_text(from)
            ),
            Self::CondZhuAtMost { haczhu } => {
                format!("require HAC balance at most {} HAC", hac(haczhu))
            }
            Self::CondZhuAtLeast { haczhu } => {
                format!("require HAC balance at least {} HAC", hac(haczhu))
            }
            Self::CondZhuEq { haczhu } => format!("require HAC balance equal to {} HAC", hac(haczhu)),
            Self::CondSatAtMost { satoshi } => {
                format!("require SAT balance at most {}", satoshi.uint())
            }
            Self::CondSatAtLeast { satoshi } => {
                format!("require SAT balance at least {}", satoshi.uint())
            }
            Self::CondSatEq { satoshi } => {
                format!("require SAT balance equal to {}", satoshi.uint())
            }
            Self::CondDiaActualAtMost { diamond } => {
                format!("require actual HACD count at most {}", diamond.uint())
            }
            Self::CondDiaActualAtLeast { diamond } => {
                format!("require actual HACD count at least {}", diamond.uint())
            }
            Self::CondDiaActualEq { diamond } => {
                format!("require actual HACD count equal to {}", diamond.uint())
            }
            Self::CondDiaProjectedAtMost { diamond } => {
                format!("require projected HACD count at most {}", diamond.uint())
            }
            Self::CondDiaProjectedAtLeast { diamond } => {
                format!("require projected HACD count at least {}", diamond.uint())
            }
            Self::CondDiaProjectedEq { diamond } => {
                format!("require projected HACD count equal to {}", diamond.uint())
            }
            Self::CondAssetAtMost { asset } => format!(
                "require Asset #{} balance at most {} atoms",
                asset.serial.uint(),
                asset.amount.uint()
            ),
            Self::CondAssetAtLeast { asset } => format!(
                "require Asset #{} balance at least {} atoms",
                asset.serial.uint(),
                asset.amount.uint()
            ),
            Self::CondAssetEq { asset } => format!(
                "require Asset #{} balance equal to {} atoms",
                asset.serial.uint(),
                asset.amount.uint()
            ),
            Self::CondHeightAtMost { height } => {
                format!("require block height at most {}", height.uint())
            }
            Self::CondHeightAtLeast { height } => {
                format!("require block height at least {}", height.uint())
            }
            Self::CondChainIdEq { chainid } => {
                format!("require chain ID equal to {}", chainid.uint())
            }
        }
    }
}

        impl Encode for TexCell {
            fn size(&self) -> usize {
                1 + match self {
                    $(Self::$variant { $( $field ),+ } => 0 $( + $field.size() )+),+
                }
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                match self {
                    $(Self::$variant { $( $field ),+ } => {
                        field::Uint1::from($id).encode_to(out);
                        $( $field.encode_to(out); )+
                    }),+
                }
            }
        }

        impl Decode for TexCell {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut reader = field::Reader::new(buf);
                let cid: field::Uint1 = reader.read()?;
                let value = match cid.uint() {
                    $($id => Self::$variant { $( $field: reader.read()? ),+ }),+,
                    id => return errf!("cannot find tex cell id '{}'", id),
                };
                Ok((value, reader.used()))
            }
        }

        impl field::ToJSON for TexCell {
            fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
                match self {
                    $(Self::$variant { $( $field ),+ } => {
                        let mut json = format!("{{\"cellid\":{}", $id);
                        $( json.push_str(&format!(",\"{}\":", stringify!($field)));
                        json.push_str(&field::ToJSON::to_json_fmt($field, fmt)); )+
                        json.push('}');
                        json
                    }),+
                }
            }
        }

        fn decode_tex_cell_json(json: &str) -> Ret<TexCell> {
            // Field lists are short (cellid + a few payload fields); a `Vec` with
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
                    let field_names: &[&str] = &[$( stringify!($field) ),+];
                    if let Some(unknown) = fields
                        .iter()
                        .find(|(k, _)| *k != "cellid" && !field_names.contains(k))
                    {
                        return sys::normalf!("TEX cell {} JSON field {} is unknown", $id, unknown.0);
                    }
                    Ok(TexCell::$variant {
                        $( $field: {
                            let field_name = stringify!($field);
                            let raw = fields
                                .iter()
                                .find(|(k, _)| *k == field_name)
                                .map(|(_, v)| *v)
                                .ok_or_else(|| {
                                    sys::Error::normal(format!(
                                        "TEX cell {} JSON missing {}",
                                        $id, field_name
                                    ))
                                })?;
                            json_decode_value(raw)?
                        } ),+
                    })
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
    CondDiaActualAtMost = 17 { diamond: Fold64 } asset=false,
    CondDiaActualAtLeast = 18 { diamond: Fold64 } asset=false,
    CondDiaActualEq = 19 { diamond: Fold64 } asset=false,
    CondAssetAtMost = 20 { asset: AssetAmt } asset=false,
    CondAssetAtLeast = 21 { asset: AssetAmt } asset=false,
    CondAssetEq = 22 { asset: AssetAmt } asset=false,
    CondHeightAtMost = 23 { height: BlockHeight } asset=false,
    CondHeightAtLeast = 24 { height: BlockHeight } asset=false,
    CondChainIdEq = 25 { chainid: Uint4 } asset=false,
    DirectedZhuPay = 26 { to: AddrOrPtr, haczhu: Fold64 } asset=false,
    DirectedZhuGet = 27 { from: AddrOrPtr, haczhu: Fold64 } asset=false,
    DirectedSatPay = 28 { to: AddrOrPtr, satnum: Fold64 } asset=false,
    DirectedSatGet = 29 { from: AddrOrPtr, satnum: Fold64 } asset=false,
    DirectedDiaPay = 30 { to: AddrOrPtr, diamonds: DiamondNameListMax200 } asset=false,
    DirectedDiaGet = 31 { from: AddrOrPtr, diamonds: DiamondNameListMax200 } asset=false,
    DirectedAssetPay = 32 { to: AddrOrPtr, asset: AssetAmt } asset=true,
    DirectedAssetGet = 33 { from: AddrOrPtr, asset: AssetAmt } asset=true,
    CondDiaProjectedAtMost = 34 { diamond: Fold64 } asset=false,
    CondDiaProjectedAtLeast = 35 { diamond: Fold64 } asset=false,
    CondDiaProjectedEq = 36 { diamond: Fold64 } asset=false,
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
    // DirectedGet sources are pinned to the transaction main (ptr 0) by the
    // exec-time form guard, so no bundle-derived signer set exists: the trait
    // default (empty req_sign) applies and only the transaction main must
    // sign, which the envelope already requires.
    extra9: this.has_asset_transfer_cell(),
    ctor: none,
    description: format!(
        "Execute TEX cells signed by {}: {}",
        this.addr.to_readable(),
        this.cells.as_list().iter().map(|c| c.describe()).collect::<Vec<_>>().join("; ")
    )
}}

impl TexCellExecute {
    fn has_asset_transfer_cell(&self) -> bool {
        self.cells.iter().any(|c| c.is_asset_transfer())
    }

    pub(crate) fn get_sign_stuff(&self) -> Hash {
        let mut stf = Vec::with_capacity(self.addr.size() + self.cells.size());
        self.addr.encode_to(&mut stf);
        self.cells.encode_to(&mut stf);
        Hash::from(sys::calculate_hash(stf))
    }

    /// Final TEX authorization signing digest, shared with external signing tools.
    /// Read-only view of the bundle's cells for SDK review/policy layers.
    pub fn cells(&self) -> &[TexCell] {
        self.cells.as_list()
    }

    pub fn sign_hash(&self) -> Hash {
        self.get_sign_stuff()
    }

    /// Build and sign a TEX bundle. The signature covers exactly `addr` and
    /// the encoded cells, matching consensus verification; the transaction
    /// envelope is intentionally not part of this replayable bundle.
    pub fn create_signed(addr: Address, cells: Vec<TexCell>, account: &Account) -> Ret<Self> {
        account.check_addr(addr.as_bytes())?;
        let mut action = Self {
            kind: field::Uint2::from(Self::KIND),
            addr,
            cells: ListW1::from(cells)?,
            sign: Sign::default(),
        };
        action.sign = Sign::create_by(account, &action.get_sign_stuff());
        Ok(action)
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
    fn create_signed_builds_a_verifiable_replayable_bundle() {
        let account = sys::Account::create_by_password("tex signed builder").unwrap();
        let addr = Address::from(*account.address());
        let action = TexCellExecute::create_signed(
            addr,
            vec![TexCell::ZhuPay {
                haczhu: Fold64::from(1).unwrap(),
            }],
            &account,
        )
        .unwrap();
        let digest = action.get_sign_stuff();
        assert!(sys::Account::verify_signature(
            &digest.0,
            &action.sign.publickey,
            &action.sign.signature,
        ));
        // Directed get sources are pinned to the transaction main, so no
        // extra signers are derived from the cells: the empty req_sign
        // default applies and the envelope's main signature suffices.
        assert!(base::Action::req_sign(&TexCellExecute::default()).is_empty());
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

    #[test]
    fn dia_actual_conditions_keep_legacy_ids_and_projected_conditions_use_new_ids() {
        let cases = [
            (
                TexCell::CondDiaActualAtMost {
                    diamond: Fold64::from(7).unwrap(),
                },
                17u8,
                7u64,
                "require actual HACD count at most 7",
            ),
            (
                TexCell::CondDiaActualAtLeast {
                    diamond: Fold64::from(8).unwrap(),
                },
                18,
                8,
                "require actual HACD count at least 8",
            ),
            (
                TexCell::CondDiaActualEq {
                    diamond: Fold64::from(9).unwrap(),
                },
                19,
                9,
                "require actual HACD count equal to 9",
            ),
            (
                TexCell::CondDiaProjectedAtMost {
                    diamond: Fold64::from(10).unwrap(),
                },
                34,
                10,
                "require projected HACD count at most 10",
            ),
            (
                TexCell::CondDiaProjectedAtLeast {
                    diamond: Fold64::from(11).unwrap(),
                },
                35,
                11,
                "require projected HACD count at least 11",
            ),
            (
                TexCell::CondDiaProjectedEq {
                    diamond: Fold64::from(12).unwrap(),
                },
                36,
                12,
                "require projected HACD count equal to 12",
            ),
        ];
        for (cell, id, value, text) in cases {
            // The legacy actual-count cells keep 17/18/19; the projected
            // variants are new ids and must never collide with them.
            let wire = cell.encode();
            assert_eq!(wire[0], id, "{text}");
            assert_eq!(cell.size(), wire.len(), "{text}");
            let (decoded, used) = TexCell::decode(&wire).unwrap();
            assert_eq!(used, wire.len(), "{text}");
            assert_eq!(decoded.encode(), wire, "{text}");

            let json = cell.to_json();
            assert_eq!(json, format!("{{\"cellid\":{id},\"diamond\":{value}}}"));
            let mut back = TexCell::default();
            back.from_json(&json).unwrap();
            assert_eq!(back.encode(), wire, "{json}");

            assert_eq!(cell.describe(), text);
            assert!(!cell.is_asset_transfer(), "{text}");
        }
    }

    fn directed_sample_cells() -> Vec<TexCell> {
        let to = AddrOrPtr::Addr(Address::from([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]));
        let asset = AssetAmt {
            serial: Fold64::from(11).unwrap(),
            amount: Fold64::from(5).unwrap(),
        };
        vec![
            TexCell::DirectedZhuPay {
                to,
                haczhu: Fold64::from(1000).unwrap(),
            },
            TexCell::DirectedZhuGet {
                from: AddrOrPtr::Ptr(0),
                haczhu: Fold64::from(2000).unwrap(),
            },
            TexCell::DirectedSatPay {
                to,
                satnum: Fold64::from(7).unwrap(),
            },
            TexCell::DirectedSatGet {
                from: AddrOrPtr::Ptr(1),
                satnum: Fold64::from(9).unwrap(),
            },
            TexCell::DirectedDiaPay {
                to,
                diamonds: DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").unwrap(),
            },
            TexCell::DirectedDiaGet {
                from: AddrOrPtr::Ptr(0),
                diamonds: DiamondNameListMax200::from_readable("WTYUIA").unwrap(),
            },
            TexCell::DirectedAssetPay {
                to,
                asset: asset.clone(),
            },
            TexCell::DirectedAssetGet {
                from: AddrOrPtr::Ptr(2),
                asset,
            },
        ]
    }

    #[test]
    fn directed_cells_wire_roundtrip_and_carry_the_asset_flag() {
        for cell in directed_sample_cells() {
            assert_eq!(cell.size(), cell.encode().len());
            let wire = cell.encode();
            let (decoded, used) = TexCell::decode(&wire).unwrap();
            assert_eq!(used, wire.len());
            assert_eq!(decoded.encode(), wire);
            // Asset directed cells ride the asset gas tier like AssetPay;
            // diamond directed cells match their escrow siblings (DiaPay/DiaGet,
            // asset=false), so a directed diamond settlement costs the same as
            // the pool-based one it replaces. Zhu/Sat likewise.
            assert_eq!(
                cell.is_asset_transfer(),
                matches!(
                    &cell,
                    TexCell::DirectedAssetPay { .. }
                        | TexCell::DirectedAssetGet { .. }
                ),
                "{}",
                cell.describe()
            );
        }
    }

    #[test]
    fn directed_cells_json_roundtrip_with_address_and_ptr_forms() {
        let target = Address::from([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        for cell in directed_sample_cells() {
            let json = cell.to_json();
            let mut back = TexCell::default();
            back.from_json(&json).unwrap();
            assert_eq!(back.encode(), cell.encode(), "{json}");
        }
        // Addr renders as the readable address string, Ptr as a bare index.
        let pay = TexCell::DirectedZhuPay {
            to: AddrOrPtr::Addr(target),
            haczhu: Fold64::from(1000).unwrap(),
        };
        assert_eq!(
            pay.to_json(),
            format!("{{\"cellid\":26,\"to\":{},\"haczhu\":1000}}", target.to_json())
        );
        let get = TexCell::DirectedZhuGet {
            from: AddrOrPtr::Ptr(0),
            haczhu: Fold64::from(2000).unwrap(),
        };
        assert_eq!(get.to_json(), "{\"cellid\":27,\"from\":0,\"haczhu\":2000}");
    }

    #[test]
    fn directed_cells_json_rejects_unknown_missing_and_duplicate_fields() {
        let addr_json = Address::from([0u8; 21]).to_json();
        let mut cell = TexCell::default();
        assert!(cell
            .from_json(&format!(
                "{{\"cellid\":26,\"to\":{addr_json},\"haczhu\":1,\"surprise\":2}}"
            ))
            .is_err());
        assert!(cell
            .from_json(&format!("{{\"cellid\":26,\"to\":{addr_json}}}"))
            .is_err());
        assert!(cell
            .from_json(&format!("{{\"cellid\":26,\"to\":{addr_json},\"haczhu\":1,\"haczhu\":2}}"))
            .is_err());
        assert!(cell.from_json("{\"cellid\":40,\"to\":0,\"haczhu\":1}").is_err());
    }

    #[test]
    fn directed_cell_descriptions_name_counterparty_and_direction() {
        let target = Address::from([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let pay = TexCell::DirectedZhuPay {
            to: AddrOrPtr::Addr(target),
            haczhu: Fold64::from(1000).unwrap(),
        };
        assert_eq!(
            pay.describe(),
            format!("pay 0.00001 HAC directly to {}", target.to_readable())
        );
        let get = TexCell::DirectedZhuGet {
            from: AddrOrPtr::Ptr(0),
            haczhu: Fold64::from(2000).unwrap(),
        };
        assert_eq!(
            get.describe(),
            "take 0.00002 HAC directly from <address pointer 0> (the source must be a transaction signer)"
        );
    }
}
