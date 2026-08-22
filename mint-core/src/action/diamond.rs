//! Diamond mint action (kind 4, moved from mint; execution body gated by the `execute` feature;
//! the x16rs/protocol dependencies compile only when the execution body is enabled).

use std::sync::Arc;

use base::{ActScope, ActionRef};
use field::{
    Address, DiamondName, DiamondNumber, Encode, Fixed8, FromJSON, Hash, Reader, Uint2,
    json_decode_value, json_split_object,
};
use sys::Ret;

#[cfg(feature = "execute")]
pub use crate::exec::diamond::calculate_diamond_visual_gene;

field::impl_struct_json!(DiamondMintData {
    diamond,
    number,
    prev_hash,
    nonce,
    address
} optional custom_message when has_custom_message);
field::impl_action_json!(DiamondMint { d });

fn wire_rules() -> &'static hacash_params::DiamondRules {
    &hacash_params::MAINNET_PARAMS.mint_rules.diamond
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiamondMintData {
    pub diamond: DiamondName,
    pub number: DiamondNumber,
    pub prev_hash: Hash,
    pub nonce: Fixed8,
    pub address: Address,
    pub custom_message: Hash,
}

// `custom_message` exists on the wire only above the consensus threshold (see
// `has_custom_message`/`Encode`); declared optional, so the provider is written out (no marker syntax).
impl field::StructSchemaProvider for DiamondMintData {
    const STRUCT_SCHEMA: field::StructSchema = field::StructSchema {
        name: "DiamondMintData",
        fields: &[
            field::FieldSchema::new("diamond", field::FieldWire::DiamondName),
            field::FieldSchema::new("number", field::FieldWire::DiamondNumber),
            field::FieldSchema::new("prev_hash", field::FieldWire::Fixed(32)),
            field::FieldSchema::new("nonce", field::FieldWire::Fixed(8)),
            field::FieldSchema::new("address", field::FieldWire::Address),
            field::FieldSchema::optional("custom_message", field::FieldWire::Fixed(32)),
        ],
    };
}

impl field::FieldWireShape for DiamondMintData {
    const WIRE: field::FieldWire = field::FieldWire::Struct("DiamondMintData");
}

impl field::WireElementName for DiamondMintData {
    const NAME: &'static str = "DiamondMintData";
}

impl Default for DiamondMintData {
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

impl DiamondMintData {
    fn has_custom_message(&self) -> bool {
        self.number.uint() > wire_rules().custom_message_after
    }
}

impl Encode for DiamondMintData {
    fn size(&self) -> usize {
        self.diamond.size()
            + self.number.size()
            + self.prev_hash.size()
            + self.nonce.size()
            + self.address.size()
            + if self.has_custom_message() {
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
        if self.has_custom_message() {
            self.custom_message.encode_to(out);
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiamondMint {
    pub kind: Uint2,
    pub d: DiamondMintData,
}

impl DiamondMint {
    pub const KIND: u16 = 4;

    pub fn with(diamond: DiamondName, number: DiamondNumber) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            d: DiamondMintData {
                diamond,
                number,
                ..Default::default()
            },
        }
    }
}

impl base::ActionSchemaProvider for DiamondMint {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: "diamond_mint",
        audit_class: base::AuditClass::Full,
        blob: false,
        has_code: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("d", base::FieldWire::Struct("DiamondMintData")),
        ],
    };
}

impl Encode for DiamondMint {
    fn size(&self) -> usize {
        self.kind.size() + self.d.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.d.encode_to(out);
    }
}

base::impl_action_facts! {
    DiamondMint {
        name: "diamond_mint",
        scope: ActScope::TOP_ONLY,
        min_tx_type: 2,
        extra9: |this: &DiamondMint| {
            this.d.number.uint() > wire_rules().burn_90_percent_after
        },
        req_sign: |_: &DiamondMint| vec![],
        as_transfer_like: none,
        description: |this: &DiamondMint| format!("Mint diamond <{}> number {}", this.d.diamond.to_readable(), this.d.number.uint()),
    }
}

pub fn create_diamond_mint(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != DiamondMint::KIND {
        return sys::normalf!("DiamondMint codec got kind {}", kind.uint());
    }
    let diamond: DiamondName = r.read()?;
    let number: DiamondNumber = r.read()?;
    let prev_hash: Hash = r.read()?;
    let nonce: Fixed8 = r.read()?;
    let address: Address = r.read()?;
    let custom_message = if number.uint() > wire_rules().custom_message_after {
        r.read()?
    } else {
        Hash::default()
    };
    Ok((
        Arc::new(DiamondMint {
            kind,
            d: DiamondMintData {
                diamond,
                number,
                prev_hash,
                nonce,
                address,
                custom_message,
            },
        }),
        r.used(),
    ))
}

fn parse_diamond_mint_json(json: &str) -> Ret<DiamondMint> {
    // Field lists are short; `Vec` with a linear duplicate scan keeps the
    // hash-table machinery out of the wasm graph.
    let mut seen: Vec<&str> = Vec::new();
    let mut declared_kind = Uint2::from(DiamondMint::KIND);
    let mut data_json = None;
    let mut data = DiamondMintData::default();
    let mut flat_fields = Vec::new();

    for (key, value) in json_split_object(json)? {
        if seen.contains(&key) {
            return sys::normalf!("DiamondMint JSON field {} is duplicated", key);
        }
        seen.push(key);
        match key {
            "kind" => declared_kind = json_decode_value(value)?,
            "d" => data_json = Some(value),
            "diamond" | "number" | "prev_hash" | "nonce" | "address" | "custom_message" => {
                flat_fields.push((key, value));
            }
            _ => {}
        }
    }
    if declared_kind.uint() != DiamondMint::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            DiamondMint::KIND,
            declared_kind.uint()
        );
    }

    let fields = match data_json {
        Some(nested) => json_split_object(nested)?,
        None => flat_fields,
    };
    let mut data_seen: Vec<&str> = Vec::new();
    for (key, value) in fields {
        if data_seen.contains(&key) {
            return sys::normalf!("DiamondMint data field {} is duplicated", key);
        }
        data_seen.push(key);
        match key {
            "diamond" => data.diamond.from_json(value)?,
            "number" => data.number.from_json(value)?,
            "prev_hash" => data.prev_hash.from_json(value)?,
            "nonce" => data.nonce.from_json(value)?,
            "address" => data.address.from_json(value)?,
            "custom_message" => data.custom_message.from_json(value)?,
            _ => {}
        }
    }
    if !data_seen.contains(&"diamond") || !data_seen.contains(&"number") {
        return sys::errf!("DiamondMint JSON requires diamond and number");
    }
    Ok(DiamondMint {
        kind: declared_kind,
        d: data,
    })
}

impl base::ActionJsonCodec for DiamondMint {
    fn decode_json(json: &str) -> Ret<Self> {
        parse_diamond_mint_json(json)
    }
}

impl field::FromJSON for DiamondMint {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = parse_diamond_mint_json(json)?;
        Ok(())
    }
}

/// JSON creator for the consensus-specific diamond mint payload.
/// Accepts both `{"kind":4,"d":{...}}` and the historical flat form, keeping the API contract stable.
pub fn decode_diamond_mint_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    if kind != DiamondMint::KIND {
        return sys::normalf!("DiamondMint JSON codec got kind {}", kind);
    }
    Ok(Arc::new(parse_diamond_mint_json(json)?))
}
