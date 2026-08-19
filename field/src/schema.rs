//! Field wire shape — binary layout description of action/struct fields.
//!
//! `FieldWire` is the authoritative "field type -> wire shape" enum; every field
//! type participating in a schema implements `FieldWireShape` to provide its own
//! shape (co-located with the type definition). The `ActionCodec` derive
//! generates `ACTION_SCHEMA` via `<FieldType as FieldWireShape>::WIRE`;
//! `codec-schema-gen` collects and validates these, then generates the
//! TypeScript codec.
//!
//! This module is pure static data and never executes; native/fullnode and the
//! SDK share the same definitions.

/// Wire shape of a field (authoritative description of its binary layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldWire {
    /// Fixed-size big-endian unsigned integer: 1/2/4/5 bytes.
    U1,
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
    /// 8-byte big-endian.
    Satoshi,
    /// Fold64 variable-length compression.
    Fold64,
    /// 4-byte big-endian (`Timestamp(u32)`).
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
    /// `W2 count + 65n`.
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
    /// `U1 count + recursive actions` (dynamic dispatch, `ActionListW1`, e.g.
    /// `AstSelect.actions`).
    ActionListW1,
    /// 1 byte.
    U8,
}

/// Schema description of a single action/struct field.
///
/// `optional` marks a field whose presence on the design-A transport is
/// explicit (W2 length prefix; length 0 = absent) and whose native encoding is
/// omitted when absent. The native wire shape itself never carries a presence
/// marker: the owning codec decides presence (e.g. `DiamondMintData`
/// `custom_message` exists only above a consensus threshold), so the schema
/// flag exists to keep the transport and the friendly surface faithful to that
/// conditionality. It is a wire-fidelity fact, never a business rule.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// `#ty::WIRE`; field types that do not implement this trait fail to compile
/// (no silent fallback allowed).
pub trait FieldWireShape {
    const WIRE: FieldWire;
}

/// List element name: generic wire shape for `ListW1<T>`/`ListW2<T>`
/// (`ListW1(T::NAME)`). Lists with dedicated shapes (`DiamondNameList`/
/// `AssetAmtW1`, etc.) get concrete `FieldWireShape` impls; only element types
/// needing the `ListW1/ListW2(name)` shape implement this trait.
pub trait WireElementName {
    const NAME: &'static str;
}

impl<T: WireElementName> FieldWireShape for ListW1<T> {
    const WIRE: FieldWire = FieldWire::ListW1(T::NAME);
}
impl<T: WireElementName> FieldWireShape for ListW2<T> {
    const WIRE: FieldWire = FieldWire::ListW2(T::NAME);
}

/// Complete wire schema of an action (or a nested struct).
///
/// `audit_class` (one of `full`/`structured`/`branching`/`opaque`) and `blob`
/// are static review facts declared at the action definition site (the derive
/// requires the class; `blob` is opt-in). They ride the schema capture so the
/// SDK's audit surface is the definition surface, with no separate hand-written
/// grading table; they are not wire shapes, but they are part of the codec
/// identity (hashed into `schema_set_hash`), so a grading change rotates the
/// SDK profile like any other codec change.
#[derive(Debug, Clone)]
pub struct ActionSchema {
    pub kind: u16,
    pub name: &'static str,
    pub fields: &'static [FieldSchema],
    pub audit_class: &'static str,
    pub blob: bool,
}

/// Wire schema of a nested struct (not an action, no kind).
#[derive(Debug, Clone)]
pub struct StructSchema {
    pub name: &'static str,
    pub fields: &'static [FieldSchema],
}

/// Type-level schema provider: implemented automatically by the `ActionCodec`
/// derive, explicitly by hand-written codecs.
pub trait ActionSchemaProvider {
    const ACTION_SCHEMA: ActionSchema;
}

/// Schema provider for nested structs (hand-written impls, e.g.
/// `ContractSto`/`CodeStuff`).
pub trait StructSchemaProvider {
    const STRUCT_SCHEMA: StructSchema;
}

/// Build a nested-struct schema list from the provider types (single
/// registration point for `codec-schema-gen`; new structs are appended here
/// instead of writing the `<T as StructSchemaProvider>::STRUCT_SCHEMA` noise).
#[macro_export]
macro_rules! collect_struct_schemas {
    ($($ty:ty),+ $(,)?) => {
        vec![ $(<$ty as $crate::StructSchemaProvider>::STRUCT_SCHEMA),+ ]
    };
}

// ================================ Built-in type impls ================================
// Each field type's wire shape lives in the same crate as its definition; lists
// uniformly use the generic `ListW1/ListW2<T: WireElementName>` shape
// (`ListW1("DiamondName")` etc., wire-equivalent to the dedicated variants such
// as `DiamondNameList`), and `Fixed<N>` is covered by const generics (`Hash =
// Fixed<32>`, `ChannelId = Fixed16` and other aliases get a shape automatically).

use crate::types::*;

impl FieldWireShape for Uint1 {
    const WIRE: FieldWire = FieldWire::U1;
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
    // `Uint8` is an 8-byte big-endian integer (`fixed_uint!(Uint8, u64, 8)`,
    // aliased as `Satoshi`), not 1 byte; `FieldWire::U8` (1 byte) corresponds
    // only to `Uint1`. The legacy wire_expr wrongly mapped `Uint8` to the
    // 1-byte `U8`; fixed here.
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
impl FieldWireShape for Fold64 {
    const WIRE: FieldWire = FieldWire::Fold64;
}
impl FieldWireShape for DiamondName {
    const WIRE: FieldWire = FieldWire::DiamondName;
}
impl FieldWireShape for DiamondNumber {
    const WIRE: FieldWire = FieldWire::DiamondNumber;
}
impl FieldWireShape for AssetAmt {
    const WIRE: FieldWire = FieldWire::AssetAmt;
}
impl FieldWireShape for Sign {
    const WIRE: FieldWire = FieldWire::Fixed(97);
}
impl FieldWireShape for AddrHac {
    const WIRE: FieldWire = FieldWire::Struct("AddrHac");
}
impl FieldWireShape for AssetSmelt {
    const WIRE: FieldWire = FieldWire::Struct("AssetSmelt");
}

impl StructSchemaProvider for AddrHac {
    const STRUCT_SCHEMA: StructSchema = StructSchema {
        name: "AddrHac",
        fields: &[
            FieldSchema::new("address", FieldWire::Address),
            FieldSchema::new("amount", FieldWire::Amount),
        ],
    };
}

impl StructSchemaProvider for AssetSmelt {
    const STRUCT_SCHEMA: StructSchema = StructSchema {
        name: "AssetSmelt",
        fields: &[
            FieldSchema::new("serial", FieldWire::Fold64),
            FieldSchema::new("supply", FieldWire::Fold64),
            FieldSchema::new("decimal", FieldWire::U1),
            FieldSchema::new("issuer", FieldWire::Address),
            FieldSchema::new("ticket", FieldWire::BytesW1),
            FieldSchema::new("name", FieldWire::BytesW1),
        ],
    };
}

// ---- List element names (the `ListW1/ListW2<T>` generic shape resolves element wires here) ----

impl WireElementName for DiamondName {
    const NAME: &'static str = "DiamondName";
}
impl WireElementName for AssetAmt {
    const NAME: &'static str = "AssetAmt";
}
impl WireElementName for Uint4 {
    const NAME: &'static str = "Uint4";
}
impl WireElementName for Sign {
    const NAME: &'static str = "Sign";
}

// ================================ Derivation macro ================================
// `wire_struct_schema!` generates the full wire-schema impl set for a plain
// named-field struct whose binary layout is exactly its fields in declaration
// order: `StructSchemaProvider` (built from each field type's `FieldWireShape`,
// same mechanism as the `ActionCodec` derive), `FieldWireShape` (own `Struct`
// name) and `WireElementName`. Used by `vm::contract_codec_struct!` and for
// standalone structs with hand-written codecs (`CodeStuff`). Composite types
// whose wire shape is not a plain field sequence (`FuncArgvTypes`) keep
// hand-written impls.
//
// A field may be followed by the `optional` marker (e.g.
// `custom_message: Hash optional`): the `StructSchemaProvider` then carries
// `FieldSchema::optional = true` (see `FieldSchema` for the transport
// semantics). The marker is validated by `wire_struct_field_schema!` — only
// the literal `optional` is accepted, so a typo fails to compile instead of
// silently changing the wire contract.

#[macro_export]
macro_rules! wire_struct_schema {
    ($name:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        impl $crate::schema::StructSchemaProvider for $name {
            const STRUCT_SCHEMA: $crate::schema::StructSchema = $crate::schema::StructSchema {
                name: stringify!($name),
                fields: &[
                    $($crate::schema::FieldSchema::new(
                        stringify!($field),
                        <$ty as $crate::schema::FieldWireShape>::WIRE,
                    )),+
                ],
            };
        }
        impl $crate::schema::FieldWireShape for $name {
            const WIRE: $crate::schema::FieldWire =
                $crate::schema::FieldWire::Struct(stringify!($name));
        }
        impl $crate::schema::WireElementName for $name {
            const NAME: &'static str = stringify!($name);
        }
    };
}
