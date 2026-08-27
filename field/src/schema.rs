//! Field wire shape — authoritative `FieldWire` enum mapping each field type to its
//! binary layout, consumed by the SDK codegen. Pure static data, never executes.

/// Wire shape of a field (authoritative description of its binary layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldWire {
    /// Fixed-size big-endian unsigned integer: 2/4/5 bytes (`U8` is the 1-byte case).
    U2,
    U4,
    U5,
    /// Arbitrary fixed-size byte block (Fixed2/Fixed3/Fixed4/Fixed16/Fixed10...).
    Fixed(u8),
    /// `unit u8 + dist i8 + big-endian mantissa (<=127B)`.
    Amount,
    /// Amount that preserves its original wire bytes.
    WireAmount,
    /// `version u8 + hash 20`.
    Address,
    /// `first < 240` is an address (21B), otherwise a 1-byte pointer marker.
    AddrOrPtr,
    /// A single address (21B) or a `W1 count + 21n` list.
    AddrOrList,
    /// `W1 length prefix + data`.
    BytesW1,
    /// `W2 length prefix + data`.
    BytesW2,
    /// `W4 length prefix + data`.
    BytesW4,
    /// 8-byte big-endian.
    Satoshi,
    /// Fold64 variable-length compression.
    Fold64,
    /// 5-byte big-endian (`Timestamp(Uint5)`).
    Timestamp,
    /// 7 bytes.
    DiamondName,
    /// 3 bytes (`DiamondNumber`).
    DiamondNumber,
    /// `W1 count + 7n`.
    DiamondNameList,
    /// `serial Fold64 + amount Fold64`.
    AssetAmt,
    /// `W1 count + AssetAmt`.
    AssetAmtW1,
    /// `W1 count + Uint4`.
    ChainIDList,
    /// `W1 count + 21n` (`ListW1<ContractAddress>`).
    ContractAddrListW1,
    /// `W2 count + 97n` (`ListW2<Sign>` of `{publickey, signature}` objects).
    SignW2,
    /// `1-byte count + elements`; the element name resolves to a nested struct
    /// (`StructSchema`) or a built-in leaf (`builtin_leaf_wire`).
    ListW1(&'static str),
    /// `2-byte count + elements`.
    ListW2(&'static str),
    /// Nested named struct (fields described by that struct's schema).
    Struct(&'static str),
    /// `U2 count + recursive actions` (dynamic dispatch, `ActionListW2`).
    ActionList,
    /// `Uint1` count + recursive actions (dynamic dispatch, `ActionListW1`, e.g.
    /// `AstSelect.actions`).
    ActionListW1,
    /// 1-byte unsigned integer (`Uint1`). JS tag `u8` (bit width, not byte count).
    U8,
    /// 1-byte 0/1 boolean. JSON is `true`/`false`; binary is still byte 0/1.
    Bool,
}

impl FieldWire {
    /// Canonical tag used by generated JavaScript metadata.
    pub fn js_wire_tag(self) -> String {
        match self {
            Self::U8 => "u8".to_owned(),
            Self::U2 => "u16".to_owned(),
            Self::U4 => "u32".to_owned(),
            Self::U5 => "u40".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Fixed(n) => format!("fixed:{n}"),
            Self::Amount => "amount".to_owned(),
            Self::WireAmount => "wire_amount".to_owned(),
            Self::Address => "address".to_owned(),
            Self::AddrOrPtr => "addr_or_ptr".to_owned(),
            Self::AddrOrList => "addr_or_list".to_owned(),
            Self::BytesW1 => "bytes_w1".to_owned(),
            Self::BytesW2 => "bytes_w2".to_owned(),
            Self::BytesW4 => "bytes_w4".to_owned(),
            Self::Satoshi => "satoshi".to_owned(),
            Self::Fold64 => "fold64".to_owned(),
            Self::Timestamp => "timestamp".to_owned(),
            Self::DiamondName => "diamond_name".to_owned(),
            Self::DiamondNumber => "diamond_number".to_owned(),
            Self::DiamondNameList => "diamond_name_list".to_owned(),
            Self::AssetAmt => "asset_amt".to_owned(),
            Self::AssetAmtW1 => "asset_amt_w1".to_owned(),
            Self::ChainIDList => "chain_id_list".to_owned(),
            Self::ContractAddrListW1 => "contract_addr_list_w1".to_owned(),
            Self::SignW2 => "sign_w2".to_owned(),
            Self::ListW1(name) => format!("list_w1:{name}"),
            Self::ListW2(name) => format!("list_w2:{name}"),
            Self::Struct(name) => format!("struct:{name}"),
            Self::ActionList => "action_list".to_owned(),
            Self::ActionListW1 => "action_list_w1".to_owned(),
        }
    }

    /// Generated JavaScript handler for non-parameterized wire tags.
    pub const fn js_handler(self) -> Option<&'static str> {
        Some(match self {
            Self::U8 => "raw_u8",
            Self::U2 => "raw_u16",
            Self::U4 => "raw_u32",
            Self::Bool => "bool",
            Self::U5
            | Self::Amount
            | Self::WireAmount
            | Self::Address
            | Self::AddrOrPtr
            | Self::AddrOrList
            | Self::Satoshi
            | Self::Fold64
            | Self::Timestamp
            | Self::DiamondNumber => "decimal_str",
            Self::BytesW1
            | Self::BytesW2
            | Self::BytesW4
            | Self::DiamondName
            | Self::AssetAmtW1 => "hex_w2",
            Self::AssetAmt => "asset_amt",
            Self::DiamondNameList | Self::ChainIDList | Self::ContractAddrListW1 => "hex_list",
            Self::ActionList => "action_list",
            Self::ActionListW1 => "action_list_w1",
            Self::Fixed(_) | Self::ListW1(_) | Self::ListW2(_) | Self::Struct(_) | Self::SignW2 => {
                return None;
            }
        })
    }
}

/// Schema description of a single action/struct field. `optional` marks explicit
/// transport presence (W2 prefix, 0 = absent); native wire shape carries no presence marker — the owning codec decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSchema {
    pub name: &'static str,
    pub wire: FieldWire,
    pub optional: bool,
}

impl FieldSchema {
    pub const fn new(name: &'static str, wire: FieldWire) -> Self {
        Self {
            name,
            wire,
            optional: false,
        }
    }

    /// Optional field (explicit transport presence, see the struct doc).
    pub const fn optional(name: &'static str, wire: FieldWire) -> Self {
        Self {
            name,
            wire,
            optional: true,
        }
    }

    pub const fn with_optional(name: &'static str, wire: FieldWire, optional: bool) -> Self {
        Self {
            name,
            wire,
            optional,
        }
    }
}

/// Field type -> wire shape. The `ActionCodec` derive builds schemas from
/// `#ty::WIRE`; types without an impl fail to compile (no silent fallback).
pub trait FieldWireShape {
    const WIRE: FieldWire;
}

/// List element name: generic wire shape for `ListW1<T>`/`ListW2<T>` (`ListW1(T::NAME)`).
/// Only element types needing the generic `ListW1/ListW2(name)` shape implement this trait.
pub trait WireElementName {
    const NAME: &'static str;
}

impl<T: WireElementName> FieldWireShape for ListW1<T> {
    const WIRE: FieldWire = FieldWire::ListW1(T::NAME);
}
impl<T: WireElementName> FieldWireShape for ListW2<T> {
    const WIRE: FieldWire = FieldWire::ListW2(T::NAME);
}

/// Complete wire schema of an action (or nested struct). `audit_class`, `blob`
/// and `has_code` are static review facts from the definition site, hashed into
/// `schema_set_hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditClass {
    Full,
    Structured,
    Branching,
    Opaque,
}

impl AuditClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Structured => "structured",
            Self::Branching => "branching",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActionSchema {
    pub kind: u16,
    pub name: &'static str,
    pub fields: &'static [FieldSchema],
    pub audit_class: AuditClass,
    pub blob: bool,
    /// The action carries executable VM code (`contract_main_call` /
    /// `contract_deploy` / `contract_update` / `p2sh`); the SDK surfaces its
    /// metadata in `ActionDesc.code` and feeds `vm.code` for decompilation.
    pub has_code: bool,
}

/// Wire schema of a nested struct (not an action, no kind).
#[derive(Debug, Clone, Copy)]
pub struct StructSchema {
    pub name: &'static str,
    pub fields: &'static [FieldSchema],
}

/// Type-level schema provider: implemented automatically by the `ActionCodec`
/// derive, explicitly by hand-written codecs.
pub trait ActionSchemaProvider {
    const ACTION_SCHEMA: ActionSchema;
}

/// Schema provider for nested structs (`FieldCodec` derive, plus handwritten
/// composites such as `FuncArgvTypes`).
pub trait StructSchemaProvider {
    const STRUCT_SCHEMA: StructSchema;
}

// ================================ Built-in type impls ================================
// Each field type's wire shape lives with its definition; lists use the generic `ListW1/ListW2<T: WireElementName>` shape, `Fixed<N>` via const generics.

use crate::types::*;

impl FieldWireShape for Uint1 {
    const WIRE: FieldWire = FieldWire::U8;
}
impl FieldWireShape for Uint2 {
    const WIRE: FieldWire = FieldWire::U2;
}
impl FieldWireShape for Uint4 {
    const WIRE: FieldWire = FieldWire::U4;
}
impl FieldWireShape for Uint5 {
    const WIRE: FieldWire = FieldWire::U5;
}
impl FieldWireShape for Uint8 {
    // `Uint8` is an 8-byte big-endian integer (`fixed_uint!(Uint8, u64, 8)`, aliased `Satoshi`),
    // not 1 byte; `FieldWire::U8` (1 byte) corresponds only to `Uint1` (legacy wire_expr had it wrong).
    const WIRE: FieldWire = FieldWire::Satoshi;
}

impl<const N: usize> FieldWireShape for Fixed<N> {
    const WIRE: FieldWire = FieldWire::Fixed(N as u8);
}

impl FieldWireShape for Amount {
    const WIRE: FieldWire = FieldWire::Amount;
}
impl FieldWireShape for WireAmount {
    const WIRE: FieldWire = FieldWire::WireAmount;
}
impl FieldWireShape for Address {
    const WIRE: FieldWire = FieldWire::Address;
}
impl FieldWireShape for AddrOrPtr {
    const WIRE: FieldWire = FieldWire::AddrOrPtr;
}
impl FieldWireShape for AddrOrList {
    const WIRE: FieldWire = FieldWire::AddrOrList;
}
impl FieldWireShape for BytesW1 {
    const WIRE: FieldWire = FieldWire::BytesW1;
}
impl FieldWireShape for BytesW2 {
    const WIRE: FieldWire = FieldWire::BytesW2;
}
impl FieldWireShape for BytesW4 {
    const WIRE: FieldWire = FieldWire::BytesW4;
}
impl FieldWireShape for Fold64 {
    const WIRE: FieldWire = FieldWire::Fold64;
}
impl FieldWireShape for Timestamp {
    const WIRE: FieldWire = FieldWire::Timestamp;
}
impl FieldWireShape for Bool {
    const WIRE: FieldWire = FieldWire::Bool;
}
impl FieldWireShape for DiamondName {
    const WIRE: FieldWire = FieldWire::DiamondName;
}
impl FieldWireShape for DiamondNumber {
    const WIRE: FieldWire = FieldWire::DiamondNumber;
}
impl FieldWireShape for SatoshiAuto {
    const WIRE: FieldWire = FieldWire::Fold64;
}
impl FieldWireShape for DiamondNumberAuto {
    const WIRE: FieldWire = FieldWire::Fold64;
}
// Diamond name lists are dedicated wrapper types (not generic `ListW1<T>`),
// keeping the generic `ListW1/ListW2("DiamondName")` wire shape.
impl FieldWireShape for DiamondNameListMax200 {
    const WIRE: FieldWire = FieldWire::ListW1("DiamondName");
}
impl FieldWireShape for DiamondNameListMax60000 {
    const WIRE: FieldWire = FieldWire::ListW2("DiamondName");
}
impl FieldWireShape for AssetAmt {
    const WIRE: FieldWire = FieldWire::AssetAmt;
}

// ---- List element names (the `ListW1/ListW2<T>` generic shape resolves element wires here) ----

impl WireElementName for AssetAmt {
    const NAME: &'static str = "AssetAmt";
}
impl WireElementName for Uint4 {
    const NAME: &'static str = "Uint4";
}
impl WireElementName for AddrOrPtr {
    const NAME: &'static str = "AddrOrPtr";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_wire_tags_use_bit_width() {
        assert_eq!(FieldWire::U8.js_wire_tag(), "u8");
        assert_eq!(FieldWire::U2.js_wire_tag(), "u16");
        assert_eq!(FieldWire::U4.js_wire_tag(), "u32");
        assert_eq!(FieldWire::U5.js_wire_tag(), "u40");
        assert_eq!(FieldWire::Bool.js_wire_tag(), "bool");
        assert_eq!(FieldWire::U8.js_handler(), Some("raw_u8"));
        assert_eq!(FieldWire::Bool.js_handler(), Some("bool"));
        // There is no `u1` tag: 1-byte unsigned integers are `u8`.
        for tag in [
            FieldWire::U8.js_wire_tag(),
            FieldWire::U2.js_wire_tag(),
            FieldWire::U4.js_wire_tag(),
            FieldWire::U5.js_wire_tag(),
            FieldWire::Bool.js_wire_tag(),
        ] {
            assert_ne!(tag, "u1");
        }
    }
}
