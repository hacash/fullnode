//! Diamond mint action (kind 4, moved from mint; execution body gated by the `execute` feature;
//! the x16rs/protocol dependencies compile only when the execution body is enabled).

use std::sync::Arc;

use field::{
    Address, Decode, DiamondName, DiamondNumber, Encode, Fixed8, Hash, Reader, Uint2,
    json_decode_value, json_object_fields,
};
use sys::Ret;

#[cfg(feature = "execute")]
pub use crate::exec::diamond::calculate_diamond_visual_gene;

field::impl_action_json!(HacdMint { d });

fn consensus_rules() -> &'static hacash_params::DiamondRules {
    &hacash_params::MAINNET_PARAMS.mint_rules.diamond
}

fn custom_message_present(number: DiamondNumber) -> bool {
    number.uint() > consensus_rules().custom_message_after
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
#[field_codec(json_only, optional custom_message when has_custom_message)]
pub struct HacdMintData {
    pub diamond: DiamondName,
    pub number: DiamondNumber,
    pub prev_hash: Hash,
    pub nonce: Fixed8,
    pub address: Address,
    pub custom_message: Hash,
}

impl Default for HacdMintData {
    fn default() -> Self {
        Self {
            diamond: DiamondName::default(),
            number: DiamondNumber::default(),
            prev_hash: Hash::default(),
            nonce: Fixed8::default(),
            address: Address::default(),
            custom_message: Hash::default(),
        }
    }
}

impl HacdMintData {
    fn has_custom_message(&self) -> bool {
        custom_message_present(self.number)
    }
}

impl Encode for HacdMintData {
    fn size(&self) -> usize {
        self.diamond.size()
            + self.number.size()
            + self.prev_hash.size()
            + self.nonce.size()
            + self.address.size()
            + if custom_message_present(self.number) {
                self.custom_message.size()
            } else {
                0
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.diamond.encode_to(out);
        self.number.encode_to(out);
        self.prev_hash.encode_to(out);
        self.nonce.encode_to(out);
        self.address.encode_to(out);
        if custom_message_present(self.number) {
            self.custom_message.encode_to(out);
        }
    }
}

impl Decode for HacdMintData {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let diamond: DiamondName = r.read()?;
        let number: DiamondNumber = r.read()?;
        let prev_hash: Hash = r.read()?;
        let nonce: Fixed8 = r.read()?;
        let address: Address = r.read()?;
        let custom_message = if custom_message_present(number) {
            r.read()?
        } else {
            Hash::default()
        };
        Ok((
            Self {
                diamond,
                number,
                prev_hash,
                nonce,
                address,
                custom_message,
            },
            r.used(),
        ))
    }
}

#[base::action(
    kind = 4,
    tx_min = 2,
    scope = TOP_ONLY,
    audit = "full",
    wire = manual,
    ctor = none,
    extra9 = |this: &HacdMint| this.d.number.uint() > consensus_rules().burn_90_percent_after,
    description = |this: &HacdMint| format!(
        "Mint diamond <{}> number {}",
        this.d.diamond.to_readable(),
        this.d.number.uint()
    ),
)]
#[derive(Debug, Clone)]
pub struct HacdMint {
    pub kind: Uint2,
    pub d: HacdMintData,
}

impl Default for HacdMint {
    fn default() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            d: HacdMintData::default(),
        }
    }
}

impl HacdMint {
    pub fn with(diamond: DiamondName, number: DiamondNumber) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            d: HacdMintData {
                diamond,
                number,
                ..Default::default()
            },
        }
    }
}

base::impl_action_codec!(HacdMint);

impl base::ActionSchemaProvider for HacdMint {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: "hacd_mint",
        audit_class: base::AuditClass::Full,
        blob: false,
        has_code: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("d", base::FieldWire::Struct("HacdMintData")),
        ],
    };
}

impl Encode for HacdMint {
    fn size(&self) -> usize {
        self.kind.size() + self.d.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.d.encode_to(out);
    }
}

impl Decode for HacdMint {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let kind: Uint2 = r.read()?;
        if kind.uint() != Self::KIND {
            return sys::normalf!(
                "action kind mismatch: expected {} got {}",
                Self::KIND,
                kind.uint()
            );
        }
        let d: HacdMintData = r.read()?;
        Ok((Self { kind, d }, r.used()))
    }
}

// Codec entry points — `create_hacd_mint` is derived from the type name.
// JSON uses regular `Default + FromJSON` (`kind` + `d` only).
base::action_codec_entries! { HacdMint {
    wire = (_reg, buf) {
        let (mint, used) = HacdMint::decode(buf)?;
        Ok((Arc::new(mint), used))
    },
}}

impl field::FromJSON for HacdMint {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let mut kind = None;
        let mut data = None;
        json_object_fields(json, &["kind", "d"], &mut |key, value| {
            match key {
                "kind" => kind = Some(value),
                "d" => data = Some(json_decode_value(value)?),
                _ => return sys::errf!("HacdMint JSON field {} is unknown", key),
            }
            Ok(())
        })?;
        let kind_raw = kind.ok_or_else(|| sys::Error::normal("HacdMint JSON missing kind"))?;
        let kind = Uint2::from(field::json_action_kind(kind_raw, HacdMint::NAME, HacdMint::KIND)?);
        let d: HacdMintData = data.ok_or_else(|| sys::Error::normal("HacdMint JSON missing d"))?;
        *self = HacdMint { kind, d };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{BinaryCodecs, WireCodecTable};
    use field::{Address, Decode, DiamondName, DiamondNumber, Encode, Fixed8, Hash};

    struct StubReg {
        table: WireCodecTable,
    }

    impl StubReg {
        fn new() -> Self {
            let mut table = WireCodecTable::new();
            table
                .add_action(base::action_codec_binding!(HacdMint, create_hacd_mint))
                .unwrap();
            Self { table }
        }
    }

    base::test_codec_host!(StubReg);

    fn sample(number: u32, message: Hash) -> HacdMint {
        let mut mint = HacdMint::with(
            DiamondName::from_readable("WTYUIA").unwrap(),
            DiamondNumber::from(number),
        );
        mint.d.prev_hash = Hash::from([1; 32]);
        mint.d.nonce = Fixed8::from([2; 8]);
        mint.d.address = Address::default();
        mint.d.custom_message = message;
        mint
    }

    #[test]
    fn hacd_mint_wire_roundtrip_gates_custom_message() {
        let below = sample(20_000, Hash::from([9; 32]));
        assert_eq!(below.size(), below.encode().len());
        let wire = below.encode();
        let (decoded, used) = HacdMint::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.d.custom_message, Hash::default());
        assert_eq!(decoded.size(), decoded.encode().len());

        let above = sample(20_001, Hash::from([9; 32]));
        assert_eq!(above.size(), above.encode().len());
        let wire = above.encode();
        let (decoded, used) = HacdMint::decode(&wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);
        assert_eq!(decoded.d.custom_message, Hash::from([9; 32]));
    }

    #[test]
    fn hacd_mint_wrong_kind_rejected_by_decode_and_registry() {
        let reg = StubReg::new();
        let mint = sample(20_001, Hash::from([9; 32]));
        let mut wire = mint.encode();
        wire[1] = 1;
        assert!(HacdMint::decode(&wire).is_err());
        assert!(create_hacd_mint(&reg, &wire).is_err());
        assert!(reg.decode_action(&wire).is_err());

        let good = mint.encode();
        let (via_decode, used_d) = HacdMint::decode(&good).unwrap();
        let (via_create, used_c) = create_hacd_mint(&reg, &good).unwrap();
        let (via_reg, used_r) = reg.decode_action(&good).unwrap();
        assert_eq!(used_d, good.len());
        assert_eq!(used_c, good.len());
        assert_eq!(used_r, good.len());
        assert_eq!(via_decode.encode(), good);
        assert_eq!(via_create.encode(), good);
        assert_eq!(via_reg.encode(), good);
    }
}
