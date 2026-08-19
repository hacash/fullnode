//! TransactionSpec binary payload decoding (symmetric to
//! `encodeTransactionSpec` in `sdk/js/generated/codec.ts`, layout in §4).
//!
//! Layout (v1):
//! ```text
//! u8 tx_type | W2 main | W2 fee | u64 timestamp | u8 gas_max | u16 action_count
//! per action: u16 kind + fields encoded per that action's schema (design A:
//! amount/address/hex and other "semantic" fields as W2 strings/hex, numbers
//! u1/u2/u4 as raw big-endian, u5+ as decimal strings, lists with a u16 count)
//! ```
//! Decoding yields `Vec<(field name, WireValue)>` and then maps to
//! `build::ActionSpec`.

use sys::{Ret, errf};

use base::FieldWire;
use field::Encode;

use crate::build::{ActionSpec, TransactionSpec};
use crate::error::{SdkError, SdkErrorCode};

/// Generic decoded value (the stringized form of design A). Exposed as part of
/// the raw action surface (`ActionSpec::RawAction`): any action kind the codec
/// schema registry knows can travel through the SDK as wire-shaped fields.
#[derive(Debug, Clone, PartialEq)]
pub enum WireValue {
    Num(u64),
    Str(String),
    Hex(Vec<u8>),
    List(Vec<WireValue>),
    Struct(Vec<(String, WireValue)>),
}

impl WireValue {
    fn as_num(&self) -> Ret<u64> {
        match self {
            WireValue::Num(n) => Ok(*n),
            _ => errf!("expected numeric wire value, got {self:?}"),
        }
    }

    fn as_str(&self) -> Ret<&str> {
        match self {
            WireValue::Str(s) => Ok(s),
            _ => errf!("expected string wire value, got {self:?}"),
        }
    }

    fn as_hex(&self) -> Ret<&[u8]> {
        match self {
            WireValue::Hex(b) => Ok(b),
            _ => errf!("expected hex wire value, got {self:?}"),
        }
    }

    fn as_list(&self) -> Ret<&[WireValue]> {
        match self {
            WireValue::List(items) => Ok(items),
            _ => errf!("expected list wire value, got {self:?}"),
        }
    }

    fn as_struct(&self) -> Ret<&[(String, WireValue)]> {
        match self {
            WireValue::Struct(items) => Ok(items),
            _ => errf!("expected struct wire value, got {self:?}"),
        }
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Ret<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return sys::errf!("payload truncated at {}", self.pos);
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Ret<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Ret<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Ret<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Ret<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// W2 length-prefixed string (utf8).
    fn w2_str(&mut self) -> Ret<String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| sys::Error::normal("payload string is not utf8"))
    }

    /// W2 length prefix + raw bytes (the TS side's `pushHexW2` already
    /// decodes hex to bytes).
    fn w2_bytes(&mut self) -> Ret<Vec<u8>> {
        let len = self.u16()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    fn done(&self) -> bool {
        self.pos == self.buf.len()
    }
}

/// Decode one field value according to `FieldWire`.
fn decode_wire(r: &mut Reader, wire: &FieldWire) -> Ret<WireValue> {
    match wire {
        FieldWire::U1 | FieldWire::U8 => Ok(WireValue::Num(r.u8()? as u64)),
        FieldWire::U2 => Ok(WireValue::Num(r.u16()? as u64)),
        FieldWire::U4 => Ok(WireValue::Num(r.u32()? as u64)),
        FieldWire::U5 => Ok(WireValue::Num(r.w2_str()?.parse().map_err(|_| sys::Error::fault("bad u5"))?)),
        FieldWire::Fixed(n) => {
            let bytes = r.take(*n as usize)?;
            Ok(WireValue::Hex(bytes.to_vec()))
        }
        FieldWire::Amount
        | FieldWire::WireAmount
        | FieldWire::Address
        | FieldWire::AddrOrPtr
        | FieldWire::AddrOrList
        | FieldWire::Satoshi
        | FieldWire::Fold64
        | FieldWire::Timestamp
        | FieldWire::DiamondNumber => Ok(WireValue::Str(r.w2_str()?)),
        FieldWire::BytesW1
        | FieldWire::BytesW2
        | FieldWire::DiamondName
        | FieldWire::SignW2
        | FieldWire::AssetAmtW1 => Ok(WireValue::Hex(r.w2_bytes()?)),
        FieldWire::DiamondNameList
        | FieldWire::ChainIDList
        | FieldWire::ContractAddrListW1 => {
            let count = r.u16()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(WireValue::Hex(r.w2_bytes()?));
            }
            Ok(WireValue::List(items))
        }
        FieldWire::AssetAmt => {
            // Design A: serial/amount are decimal strings (Fold64 packed
            // encoding stays in Rust)
            let serial = r.w2_str()?;
            let amount = r.w2_str()?;
            Ok(WireValue::Struct(vec![
                ("serial".to_owned(), WireValue::Str(serial)),
                ("amount".to_owned(), WireValue::Str(amount)),
            ]))
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let count = r.u16()? as usize;
            let elem_wire = element_wire(wire);
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(decode_wire(r, &elem_wire)?);
            }
            Ok(WireValue::List(items))
        }
        FieldWire::Struct(name) => Ok(WireValue::Struct(decode_struct_fields(r, name)?)),
        FieldWire::ActionList => {
            let count = r.u16()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(decode_action_value(r)?);
            }
            Ok(WireValue::List(items))
        }
        FieldWire::ActionListW1 => {
            // `AstSelect.actions` (ActionListW1) uses a 1-byte count
            let count = r.u8()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(decode_action_value(r)?);
            }
            Ok(WireValue::List(items))
        }
    }
}

/// Wire shape of list elements: resolved by name to a struct schema or
/// built-in leaf.
pub(crate) fn element_wire(wire: &FieldWire) -> FieldWire {
    let name = match wire {
        FieldWire::ListW1(name) | FieldWire::ListW2(name) => name,
        _ => unreachable!("element_wire called on non-list"),
    };
    // Registered action/struct names → nested reference; otherwise look up
    // the built-in leaf.
    if action_schema_registry().iter().any(|(n, _)| *n == *name)
        || struct_schema_registry().iter().any(|(n, _)| *n == *name)
    {
        return FieldWire::Struct(name);
    }
    base::builtin_leaf_wire(name).unwrap_or(FieldWire::Struct(name))
}

/// Decode one struct (in field-schema order, skipping kind). Optional fields
/// (`FieldSchema::optional`) travel as a W2 length prefix on the transport
/// (length 0 = absent), so a missing optional field consumes two bytes and is
/// never ambiguous with the following data.
fn decode_struct_fields(r: &mut Reader, name: &str) -> Ret<Vec<(String, WireValue)>> {
    let fields = struct_fields_of(name).ok_or_else(|| sys::Error::fault(format!("unknown struct schema {}", name)))?;
    if fields.is_empty() {
        // Placeholder empty schemas (e.g. TexCell/FuncArgvTypes) cannot be
        // decoded: error out rather than silently consuming zero bytes
        return errf!("struct schema {} has no fields (not yet supported)", name);
    }
    decode_schema_fields(r, fields)
}

/// Decode one field-sequence (struct members or top-level action fields,
/// skipping `kind`), honoring optional-field presence (W2 length prefix).
fn decode_schema_fields(
    r: &mut Reader,
    fields: &[base::FieldSchema],
) -> Ret<Vec<(String, WireValue)>> {
    let mut out = Vec::with_capacity(fields.len());
    for field in fields {
        if field.name == "kind" {
            continue;
        }
        if field.optional {
            let len = r.u16()? as usize;
            if len == 0 {
                continue;
            }
            let inner = r.take(len)?;
            let mut sub = Reader::new(inner);
            let value = decode_wire(&mut sub, &field.wire)?;
            out.push((field.name.to_owned(), value));
        } else {
            let value = decode_wire(r, &field.wire)?;
            out.push((field.name.to_owned(), value));
        }
    }
    Ok(out)
}

/// Wire shape of one top-level action field (used by the table-driven build
/// direction to convert friendly values per the schema).
pub(crate) fn schema_wire_of(action_name: &str, field: &str) -> Option<FieldWire> {
    action_schema_registry()
        .iter()
        .find(|(n, _)| *n == action_name)
        .and_then(|(_, schema)| {
            schema
                .fields
                .iter()
                .find(|f| f.name == field)
                .map(|f| f.wire.clone())
        })
}

/// Wire shape of one member of a nested struct reference (used by the
/// table-driven build direction). The `asset` field is the dedicated
/// `AssetAmt` wire variant whose serial/amount members are intrinsic.
pub(crate) fn struct_member_wire(action_name: &str, struct_field: &str, member: &str) -> Option<FieldWire> {
    let action = action_schema_registry()
        .iter()
        .find(|(n, _)| *n == action_name)
        .map(|(_, s)| *s)?;
    let field = action.fields.iter().find(|f| f.name == struct_field)?;
    match &field.wire {
        FieldWire::Struct(name) => struct_fields_of(name)?
            .iter()
            .find(|f| f.name == member)
            .map(|f| f.wire.clone()),
        FieldWire::AssetAmt => match member {
            "serial" | "amount" => Some(FieldWire::Fold64),
            _ => None,
        },
        _ => None,
    }
}

/// Decode one action: u16 kind + field sequence. The decoded struct keeps the
/// kind as a `"kind"` name entry (first element) so nested action lists can be
/// re-encoded by the generic constructor without losing the dispatch tag.
fn decode_action_value(r: &mut Reader) -> Ret<WireValue> {
    let kind = r.u16()?;
    let schema = action_schema_of(kind).ok_or_else(|| sys::Error::fault(format!("unknown action kind {}", kind)))?;
    let mut fields = Vec::with_capacity(schema.fields.len());
    fields.push(("kind".to_owned(), WireValue::Str(schema.name.to_owned())));
    fields.extend(decode_schema_fields(r, schema.fields)?);
    Ok(WireValue::Struct(fields))
}

// ---- schema lookup (action/struct registry inside the sdk crate) ----

fn action_schema_of(kind: u16) -> Option<&'static base::ActionSchema> {
    action_schema_registry()
        .iter()
        .map(|(_, s)| *s)
        .find(|s| s.kind == kind)
}

fn struct_fields_of(name: &str) -> Option<&'static [base::FieldSchema]> {
    if let Some(schema) = action_schema_registry()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
    {
        return Some(schema.fields);
    }
    struct_schema_registry()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| s.fields)
}

/// (registered name, schema) lazy registry: schemas are captured during
/// `standard_codecs()` registration assembly (same registration macro as
/// `codec-schema-gen`, naturally the same source as the runtime registry; new
/// actions need no registration here).
fn action_schema_registry() -> &'static Vec<(&'static str, &'static base::ActionSchema)> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Vec<(&'static str, &'static base::ActionSchema)>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| {
        let codecs = crate::codec::standard_codecs().expect("standard codecs assembly");
        codecs
            .action_schemas()
            .iter()
            .map(|s| (s.name, s))
            .collect()
    })
}

fn collect_struct_schemas() -> Vec<(&'static str, &'static base::StructSchema)> {
    let leaked: &'static [base::StructSchema] =
        Box::leak(chain_codec::struct_schemas().into_boxed_slice());
    leaked.iter().map(|s| (s.name, s)).collect()
}

fn struct_schema_registry() -> &'static Vec<(&'static str, &'static base::StructSchema)> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Vec<(&'static str, &'static base::StructSchema)>> =
        OnceLock::new();
    REGISTRY.get_or_init(collect_struct_schemas)
}

// ================================ top-level decoding ================================

/// Decode a TransactionSpec binary payload (inverse of §4 codec.ts
/// `encodeTransactionSpec`).
pub fn decode_transaction_spec_binary(buf: &[u8]) -> Result<TransactionSpec, SdkError> {
    Ok(decode_transaction_spec_parts(buf)?.1)
}

/// Payload → (per-action wire kinds, spec). The kinds let the build tests
/// lock the table-driven path: the native actions of the built body must
/// carry exactly the kinds the payload declared.
pub(crate) fn decode_transaction_spec_parts(
    buf: &[u8],
) -> Result<(Vec<u16>, TransactionSpec), SdkError> {
    let mut r = Reader::new(buf);
    let tx_type = r.u8().map_err(spec_err)?;
    let main = r.w2_str().map_err(spec_err)?;
    let fee = r.w2_str().map_err(spec_err)?;
    let timestamp = r.u64().map_err(spec_err)?;
    let gas_max = r.u8().map_err(spec_err)?;
    let count = r.u16().map_err(spec_err)? as usize;
    let mut actions = Vec::with_capacity(count);
    let mut kinds = Vec::with_capacity(count);
    for _ in 0..count {
        let (kind, action) = decode_action_spec(&mut r)?;
        kinds.push(kind);
        actions.push(action);
    }
    if !r.done() {
        return Err(spec_err(sys::Error::fault("trailing bytes in TransactionSpec payload")));
    }
    Ok((
        kinds,
        TransactionSpec {
            schema: Some(crate::schema::SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type,
            main,
            fee,
            timestamp: (timestamp != 0).then_some(timestamp as u64),
            gas_max: (gas_max != 0).then_some(gas_max),
            actions,
        },
    ))
}

fn spec_err(e: sys::Error) -> SdkError {
    SdkError::new(SdkErrorCode::ParseFailed, e.to_string())
}

/// Decode one action and map it to `ActionSpec`, returning the wire kind too.
fn decode_action_spec(r: &mut Reader) -> Result<(u16, ActionSpec), SdkError> {
    let kind = r.u16().map_err(spec_err)?;
    let schema = action_schema_of(kind).ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!("unknown action kind {}", kind),
        )
    })?;
    let fields = decode_schema_fields(r, schema.fields).map_err(spec_err)?;
    Ok((kind, crate::actionspec::map_action_spec(schema.name, fields)?))
}

pub(crate) fn field_str(fields: &[(String, WireValue)], name: &str) -> Result<String, SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Str(v))) => Ok(v.clone()),
        Some((_, WireValue::Hex(v))) => Ok(hex::encode(v)),
        Some((_, WireValue::Num(v))) => Ok(v.to_string()),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("action field {} missing or invalid", name),
        )),
    }
}

pub(crate) fn field_num<T: std::str::FromStr>(fields: &[(String, WireValue)], name: &str) -> Result<T, SdkError> {
    field_str(fields, name)?
        .parse()
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, format!("field {} invalid", name)))
}

pub(crate) fn field_opt<T: std::str::FromStr>(
    fields: &[(String, WireValue)],
    name: &str,
) -> Result<Option<T>, SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        // Zero is the wire default for optional fields, in both transport
        // forms (raw number or design-A string): absent and zero are the
        // same friendly value.
        Some((_, WireValue::Num(0))) => Ok(None),
        Some((_, WireValue::Str(s))) if s == "0" => Ok(None),
        Some((_, v)) => {
            let s = match v {
                WireValue::Num(n) => n.to_string(),
                WireValue::Str(s) => s.clone(),
                WireValue::Hex(b) => hex::encode(b),
                _ => return Err(SdkError::new(SdkErrorCode::ParseFailed, "bad field")),
            };
            s.parse::<T>()
                .map(Some)
                .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, format!("field {} invalid", name)))
        }
        None => Ok(None),
    }
}

pub(crate) fn field_num_list<T: std::str::FromStr>(
    fields: &[(String, WireValue)],
    name: &str,
) -> Result<Vec<T>, SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::List(items))) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = match item {
                    WireValue::Num(n) => n.to_string(),
                    WireValue::Str(s) => s.clone(),
                    WireValue::Hex(b) => hex::encode(b),
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!("field {} items invalid", name),
                        ))
                    }
                };
                out.push(s.parse().map_err(|_| {
                    SdkError::new(SdkErrorCode::ParseFailed, format!("field {} invalid", name))
                })?);
            }
            Ok(out)
        }
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("action field {} missing or invalid", name),
        )),
    }
}

pub(crate) fn field_str_list(fields: &[(String, WireValue)], name: &str) -> Result<Vec<String>, SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::List(items))) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(match item {
                    WireValue::Str(s) => s.clone(),
                    WireValue::Hex(b) => hex::encode(b),
                    WireValue::Num(n) => n.to_string(),
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!("field {} items invalid", name),
                        ))
                    }
                });
            }
            Ok(out)
        }
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("action field {} missing or invalid", name),
        )),
    }
}

/// DiamondName's wire form is 6 bytes of ASCII (`Fixed6`) — hex bytes to
/// readable name.
pub(crate) fn diamond_field_readable(
    fields: &[(String, WireValue)],
    name: &str,
) -> Result<String, SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Hex(bytes))) => String::from_utf8(bytes.clone()).map_err(|_| {
            SdkError::new(SdkErrorCode::ParseFailed, "diamond name is not ascii")
        }),
        Some((_, WireValue::Str(s))) => Ok(s.clone()),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("{} field missing", name),
        )),
    }
}

pub(crate) fn diamond_name_readable(fields: &[(String, WireValue)]) -> Result<String, SdkError> {
    diamond_field_readable(fields, "diamond")
}

pub(crate) fn diamond_names_readable(fields: &[(String, WireValue)]) -> Result<Vec<String>, SdkError> {
    match fields.iter().find(|(n, _)| n == "diamonds") {
        Some((_, WireValue::List(items))) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    WireValue::Hex(bytes) => out.push(String::from_utf8(bytes.clone()).map_err(
                        |_| SdkError::new(SdkErrorCode::ParseFailed, "diamond name is not ascii"),
                    )?),
                    WireValue::Str(s) => out.push(s.clone()),
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            "diamonds items must be hex",
                        ))
                    }
                }
            }
            Ok(out)
        }
        _ => Err(SdkError::new(SdkErrorCode::ParseFailed, "diamonds field missing")),
    }
}

/// The `AssetAmt` struct fields (`serial`/`amount`).
pub(crate) fn asset_fields<'a>(
    fields: &'a [(String, WireValue)],
) -> Result<&'a [(String, WireValue)], SdkError> {
    match fields.iter().find(|(n, _)| n == "asset") {
        Some((_, WireValue::Struct(items))) => Ok(items),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "asset field missing",
        )),
    }
}

/// Extract nested struct field values (`AddrHac`/`AssetSmelt`/
/// `DiamondMintData` etc.).
pub(crate) fn fields_struct<'a>(
    fields: &'a [(String, WireValue)],
    name: &str,
) -> Result<&'a [(String, WireValue)], SdkError> {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Struct(items))) => Ok(items),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("{} field missing or invalid", name),
        )),
    }
}

pub(crate) fn struct_field_str(items: &[(String, WireValue)], name: &str) -> Result<String, SdkError> {
    match items.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Str(s))) => Ok(s.clone()),
        Some((_, WireValue::Hex(b))) => Ok(hex::encode(b)),
        Some((_, WireValue::Num(n))) => Ok(n.to_string()),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("struct field {} missing or invalid", name),
        )),
    }
}

/// Optional struct member: `None` when the member is absent (the field is
/// declared `optional` in its struct schema and was omitted on the wire).
pub(crate) fn struct_field_opt_str(
    items: &[(String, WireValue)],
    name: &str,
) -> Result<Option<String>, SdkError> {
    match items.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Str(s))) => Ok(Some(s.clone())),
        Some((_, WireValue::Hex(b))) => Ok(Some(hex::encode(b))),
        Some((_, WireValue::Num(n))) => Ok(Some(n.to_string())),
        None => Ok(None),
        Some((_, _)) => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("struct field {} invalid", name),
        )),
    }
}

/// Convert a hex byte field inside a struct to a readable string
/// (`DiamondMintData.diamond`).
pub(crate) fn struct_field_readable(items: &[(String, WireValue)], name: &str) -> Result<String, SdkError> {
    match items.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Hex(bytes))) => String::from_utf8(bytes.clone()).map_err(|_| {
            SdkError::new(SdkErrorCode::ParseFailed, "diamond name is not ascii")
        }),
        Some((_, WireValue::Str(s))) => Ok(s.clone()),
        _ => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("struct field {} missing", name),
        )),
    }
}

// ================================ generic native construction ================================
//
// Turns design-A wire values (strings/hex/lists, the shape the generated JS
// encoder produced) into *native* action payload bytes, then hands them to the
// protocol's own action decoder. This is how any action kind the codec schema
// registry knows can be built through the raw path — no per-action
// constructor, no hand-written capability list. Kinds without a transaction
// action codec (e.g. VM host opcodes) fail at `decode_action` with the
// registry's error; that boundary is the chain's, not the SDK's.

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Encode one field value to its *native* wire layout (the inverse of the
/// design-A transport format; design-A strings are parsed into the native
/// field types here, exactly like `build_action` does for the typed kinds).
pub(crate) fn encode_wire(out: &mut Vec<u8>, wire: &FieldWire, value: &WireValue) -> Ret<()> {
    use field::{
        AddrOrList, AddrOrPtr, Address, Amount, BytesW1, BytesW2, DiamondName, DiamondNumber,
        Fold64, Satoshi, Sign, Timestamp, Uint1, Uint2, Uint4, Uint5, WireAmount,
    };
    match wire {
        FieldWire::U1 | FieldWire::U8 => {
            let n = value.as_num()?;
            if n > 0xff {
                return errf!("u8 value {n} out of range");
            }
            Uint1::from(n as u8).encode_to(out);
        }
        FieldWire::U2 => {
            let n = value.as_num()?;
            if n > 0xffff {
                return errf!("u2 value {n} out of range");
            }
            Uint2::from(n as u16).encode_to(out);
        }
        FieldWire::U4 => {
            let n = value.as_num()?;
            if n > 0xffff_ffff {
                return errf!("u4 value {n} out of range");
            }
            Uint4::from(n as u32).encode_to(out);
        }
        FieldWire::U5 => {
            Uint5::from(value.as_num()?).encode_to(out);
        }
        FieldWire::Fixed(n) => {
            let bytes = value.as_hex()?;
            if bytes.len() != *n as usize {
                return errf!("fixed({n}) field must be {n} bytes, got {}", bytes.len());
            }
            out.extend_from_slice(bytes);
        }
        FieldWire::Amount => {
            Amount::from(value.as_str()?)?.encode_to(out);
        }
        FieldWire::WireAmount => {
            WireAmount::from(Amount::from(value.as_str()?)?).encode_to(out);
        }
        FieldWire::Address => {
            Address::from_readable(value.as_str()?)?.encode_to(out);
        }
        FieldWire::AddrOrPtr => {
            let address = Address::from_readable(value.as_str()?)?;
            AddrOrPtr::Addr(address).encode_to(out);
        }
        FieldWire::AddrOrList => {
            let address = Address::from_readable(value.as_str()?)?;
            AddrOrList::Single(address).encode_to(out);
        }
        FieldWire::Satoshi => {
            let n: u64 = value.as_str()?.parse().map_err(|_| sys::Error::normal("satoshi not a decimal string"))?;
            Satoshi::from(n).encode_to(out);
        }
        FieldWire::Fold64 => {
            let n: u64 = value.as_str()?.parse().map_err(|_| sys::Error::normal("fold64 not a decimal string"))?;
            Fold64::from(n)?.encode_to(out);
        }
        FieldWire::Timestamp => {
            let n: u64 = value.as_str()?.parse().map_err(|_| sys::Error::normal("timestamp not a decimal string"))?;
            Timestamp::from_checked(n)?.encode_to(out);
        }
        FieldWire::DiamondNumber => {
            let n: u32 = value.as_str()?.parse().map_err(|_| sys::Error::normal("diamond number not a decimal string"))?;
            DiamondNumber::from(n).encode_to(out);
        }
        FieldWire::DiamondName => {
            let bytes = value.as_hex()?;
            if bytes.len() != DiamondName::SIZE {
                return errf!("diamond name must be {} bytes, got {}", DiamondName::SIZE, bytes.len());
            }
            DiamondName::from(bytes.try_into().expect("diamond name size checked")).encode_to(out);
        }
        FieldWire::BytesW1 => {
            BytesW1::from(value.as_hex()?.to_vec())?.encode_to(out);
        }
        FieldWire::BytesW2 => {
            BytesW2::from(value.as_hex()?.to_vec())?.encode_to(out);
        }
        FieldWire::SignW2 => {
            // Design-A carries a single W2 hex blob; treat it as the native
            // `Sign` bytes (no current schema uses this tag).
            let bytes = value.as_hex()?;
            if bytes.len() != Sign::SIZE {
                return errf!("sign must be {} bytes, got {}", Sign::SIZE, bytes.len());
            }
            Sign {
                publickey: bytes[..Sign::PUBLICKEY_SIZE].try_into().expect("sign split"),
                signature: bytes[Sign::PUBLICKEY_SIZE..].try_into().expect("sign split"),
            }
            .encode_to(out);
        }
        FieldWire::AssetAmtW1 => {
            // Design-A carries a single W2 hex blob; treat it as the native
            // `AssetAmtW1` bytes (no current schema uses this tag).
            out.extend_from_slice(value.as_hex()?);
        }
        FieldWire::DiamondNameList => {
            let items = value.as_list()?;
            Uint1::from(items.len() as u8).encode_to(out);
            for item in items {
                let bytes = item.as_hex()?;
                if bytes.len() != DiamondName::SIZE {
                    return errf!("diamond name must be {} bytes, got {}", DiamondName::SIZE, bytes.len());
                }
                DiamondName::from(bytes.try_into().expect("diamond name size checked"))
                    .encode_to(out);
            }
        }
        FieldWire::ChainIDList => {
            let items = value.as_list()?;
            Uint1::from(items.len() as u8).encode_to(out);
            for item in items {
                let bytes = item.as_hex()?;
                if bytes.len() != 4 {
                    return errf!("chain id must be 4 bytes, got {}", bytes.len());
                }
                Uint4::from(u32::from_be_bytes(bytes.try_into().expect("chain id size checked")))
                    .encode_to(out);
            }
        }
        FieldWire::ContractAddrListW1 => {
            let items = value.as_list()?;
            Uint1::from(items.len() as u8).encode_to(out);
            for item in items {
                Address::from_readable(item.as_str()?)?.encode_to(out);
            }
        }
        FieldWire::AssetAmt => {
            let items = value.as_struct()?;
            let serial: u64 = struct_str_of(items, "serial")?
                .parse()
                .map_err(|_| sys::Error::normal("asset serial not a decimal string"))?;
            let amount: u64 = struct_str_of(items, "amount")?
                .parse()
                .map_err(|_| sys::Error::normal("asset amount not a decimal string"))?;
            Fold64::from(serial)?.encode_to(out);
            Fold64::from(amount)?.encode_to(out);
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let items = value.as_list()?;
            match wire {
                FieldWire::ListW1(_) => Uint1::from(items.len() as u8).encode_to(out),
                FieldWire::ListW2(_) => Uint2::from(items.len() as u16).encode_to(out),
                _ => unreachable!("list arms"),
            }
            let elem_wire = element_wire(wire);
            for item in items {
                encode_wire(out, &elem_wire, item)?;
            }
        }
        FieldWire::Struct(name) => {
            encode_struct_fields(out, name, value.as_struct()?)?;
        }
        FieldWire::ActionList => {
            let items = value.as_list()?;
            Uint2::from(items.len() as u16).encode_to(out);
            for item in items {
                encode_nested_action(out, item)?;
            }
        }
        FieldWire::ActionListW1 => {
            let items = value.as_list()?;
            if items.len() > 0xff {
                return errf!("action list exceeds 255 items: {}", items.len());
            }
            Uint1::from(items.len() as u8).encode_to(out);
            for item in items {
                encode_nested_action(out, item)?;
            }
        }
    }
    Ok(())
}

/// Encode a nested action (a `WireValue::Struct` whose `"kind"` entry carries
/// the action name).
fn encode_nested_action(out: &mut Vec<u8>, value: &WireValue) -> Ret<()> {
    let items = value.as_struct()?;
    let kind = items
        .iter()
        .find(|(n, _)| n == "kind")
        .and_then(|(_, v)| match v {
            WireValue::Str(s) => Some(s.as_str()),
            _ => None,
        })
        .ok_or_else(|| sys::Error::fault("nested action missing kind"))?;
    encode_action(out, kind, items)
}

/// Encode one action: u16 kind + fields per that action's schema (unknown
/// fields rejected, mirroring the generated JS encoder). `fields` never
/// includes a `kind` entry.
pub(crate) fn encode_action(
    out: &mut Vec<u8>,
    kind: &str,
    fields: &[(String, WireValue)],
) -> Ret<()> {
    let schema = action_schema_registry()
        .iter()
        .find(|(n, _)| *n == kind)
        .map(|(_, s)| *s)
        .ok_or_else(|| sys::Error::fault(format!("no action schema for {kind}")))?;
    push_u16(out, schema.kind);
    encode_struct_fields(out, kind, fields)
}

fn struct_str_of<'a>(items: &'a [(String, WireValue)], name: &str) -> Ret<&'a str> {
    match items.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Str(s))) => Ok(s),
        _ => errf!("struct field {name} missing or not a string"),
    }
}

fn encode_struct_fields(out: &mut Vec<u8>, name: &str, items: &[(String, WireValue)]) -> Ret<()> {
    let fields = struct_fields_of(name)
        .ok_or_else(|| sys::Error::fault(format!("unknown struct schema {name}")))?;
    if fields.is_empty() {
        // Placeholder empty schemas (e.g. TexCell/FuncArgvTypes) can't be
        // encoded: error out rather than silently writing zero bytes.
        return errf!("struct schema {name} has no fields (not yet supported)");
    }
    for (field_name, _) in items {
        if field_name == "kind" {
            continue;
        }
        if !fields.iter().any(|f| f.name == *field_name) {
            return errf!("struct {name} has unknown field {field_name}");
        }
    }
    for field in fields {
        if field.name == "kind" {
            continue;
        }
        match items.iter().find(|(n, _)| n == field.name) {
            Some((_, value)) => encode_wire(out, &field.wire, value)?,
            // Optional fields are absent in the native canonical form (the
            // owning codec decides native presence); writing nothing here
            // keeps encode(decode(body)) == body for the trimmed form.
            None if field.optional => {}
            None => {
                return errf!("struct {name} missing field {}", field.name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod golden_tests {
    use super::*;

    /// Parses a flat JSON object into sorted (key, value) pairs so key order
    /// never affects the comparison.
    fn sorted_pairs(json: &str) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = field::json_split_object(json)
            .expect("object")
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        pairs.sort();
        pairs
    }

    /// Golden vectors lock the §4 payload decode + friendly mapping to fixed
    /// bytes: any change to the payload layout, action schema field order or
    /// the friendly mapping fails here. Regenerate via `sdk_codegen` after a
    /// deliberate change.
    #[test]
    fn golden_vectors_decode_to_the_committed_friendly_shape() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden.json");
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let mut vectors = 0usize;
        for (_, value) in field::json_split_object(&json).expect("golden object") {
            if !value.starts_with('[') {
                continue;
            }
            for vector in field::json_split_array(value).expect("vectors") {
                let mut payload_hex = String::new();
                let mut decoded = String::new();
                for (key, v) in field::json_split_object(vector).expect("vector") {
                    match key {
                        "payload" => {
                            payload_hex = field::json_expect_quoted_decoded(v)
                                .expect("payload")
                                .to_owned()
                        }
                        "decoded" => decoded = v.to_owned(),
                        _ => {}
                    }
                }
                let bytes = hex::decode(&payload_hex).expect("payload hex");
                let spec = decode_transaction_spec_binary(&bytes).expect("payload decodes");
                let actions: Vec<String> = spec
                    .actions
                    .iter()
                    .map(|a| a.to_json_string())
                    .collect();
                let actual = format!("{{\"actions\":[{}]}}", actions.join(","));
                assert_eq!(
                    sorted_pairs(&actual),
                    sorted_pairs(&decoded),
                    "golden vector {vectors} decode drifted from golden.json"
                );
                vectors += 1;
            }
        }
        assert!(vectors >= 20, "expected a full golden vector set, got {vectors}");
    }
}
