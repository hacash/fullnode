//! `tx.build`: declarative construction of unsigned Type-1/2/3 bodies from a
//! kind-keyed action spec (Unified SDK 2.0, doc 14 §4.6/§5).
//!
//! The SDK never decides which actions are "meaningful": every action kind the
//! codec schema registry knows can be built. Friendly kinds are converted to
//! the wire shape purely from the `ACTION_SPECS` table (the same table the JS
//! adapter is generated from), then every kind — friendly or raw — is encoded
//! by the generic schema-driven path and constructed through the protocol's
//! own action decoder. There are no hand-written per-variant constructors, so
//! the friendly surface and the wire surface cannot drift apart. Kinds
//! without a transaction action codec (e.g. VM host opcodes) fail there with
//! the protocol registry's error; that boundary is the chain's, not the SDK's.

use base::{BinaryCodecs, TxCreateRequest};
use field::{Address, Amount};

use crate::actionspec::{
    ACTION_SPECS, ActionSpecDef, FieldDef, FriendlyGroup, JsConv, RustConv, friendly_groups,
};
use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::decode_tx;
use crate::schema::{SCHEMA_BUILT_TRANSACTION, SCHEMA_TRANSACTION_SPEC};
use crate::spec_codec::WireValue;

/// Friendly field types of the typed `ActionSpec` variants.
macro_rules! friendly_field_ty {
    (str) => { String };
    (opt_str) => { Option<String> };
    (num64) => { u64 };
    (num8) => { u8 };
    (opt_num8) => { Option<u8> };
    (str_list) => { Vec<String> };
    (num_list) => { Vec<u32> };
}

/// JSON key-value rendering per friendly field kind (canonical form, matched
/// by the golden vectors). The JSON field name is `stringify!($field)`, i.e.
/// exactly the enum field name.
macro_rules! friendly_spec_kv {
    ($field:ident, str) => {
        crate::json::kv(stringify!($field), crate::json::q($field))
    };
    ($field:ident, opt_str) => {
        crate::json::kv_opt(
            stringify!($field),
            $field.as_ref().map(|s| crate::json::q(s)),
        )
    };
    ($field:ident, num64) => {
        crate::json::kv(stringify!($field), $field.to_string())
    };
    ($field:ident, num8) => {
        crate::json::kv(stringify!($field), $field.to_string())
    };
    ($field:ident, opt_num8) => {
        crate::json::kv_opt(stringify!($field), $field.map(|v| v.to_string()))
    };
    ($field:ident, str_list) => {
        crate::json::kv(
            stringify!($field),
            crate::json::arr($field.iter().map(|s| crate::json::q(s)).collect()),
        )
    };
    ($field:ident, num_list) => {
        crate::json::kv(
            stringify!($field),
            crate::json::arr($field.iter().map(|v| v.to_string()).collect()),
        )
    };
}

/// One friendly field value, untyped for the table-driven wire build. The
/// variant structure (`friendly_spec!`) and the table (`actionspec.rs`) both
/// come from Rust, so the name/value pairs can be matched by name only.
#[derive(Debug, Clone)]
pub enum FriendlyValue {
    Str(String),
    OptStr(Option<String>),
    Num(u64),
    Num8(u8),
    OptNum8(Option<u8>),
    StrList(Vec<String>),
    NumList(Vec<u32>),
}

impl FriendlyValue {
    fn is_none(&self) -> bool {
        matches!(
            self,
            FriendlyValue::OptStr(None) | FriendlyValue::OptNum8(None)
        )
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            FriendlyValue::Str(s) => Some(s),
            FriendlyValue::OptStr(Some(s)) => Some(s),
            _ => None,
        }
    }

    fn as_num(&self) -> Option<u64> {
        match self {
            FriendlyValue::Num(n) => Some(*n),
            FriendlyValue::Num8(n) => Some(*n as u64),
            FriendlyValue::OptNum8(Some(n)) => Some(*n as u64),
            FriendlyValue::Str(s) => s.parse().ok(),
            FriendlyValue::OptStr(Some(s)) => s.parse().ok(),
            _ => None,
        }
    }

    fn as_str_list(&self) -> Option<&[String]> {
        match self {
            FriendlyValue::StrList(items) => Some(items),
            _ => None,
        }
    }

    fn as_num_list(&self) -> Option<&[u32]> {
        match self {
            FriendlyValue::NumList(items) => Some(items),
            _ => None,
        }
    }
}

macro_rules! friendly_value {
    ($f:ident, str) => {
        FriendlyValue::Str($f.clone())
    };
    ($f:ident, opt_str) => {
        FriendlyValue::OptStr($f.clone())
    };
    ($f:ident, num64) => {
        FriendlyValue::Num(*$f)
    };
    ($f:ident, num8) => {
        FriendlyValue::Num8(*$f)
    };
    ($f:ident, opt_num8) => {
        FriendlyValue::OptNum8(*$f)
    };
    ($f:ident, str_list) => {
        FriendlyValue::StrList($f.clone())
    };
    ($f:ident, num_list) => {
        FriendlyValue::NumList($f.clone())
    };
}

/// Declares the typed `ActionSpec` surface once: the enum variant structure,
/// its canonical JSON (`to_json_string`) and the friendly-field extraction
/// (`friendly_fields`, consumed by the table-driven wire build) all come from
/// this list, so the friendly field names can never drift apart. `RawAction`
/// (the generic wire-shaped fallback, see the module docs) is appended by the
/// macro.
macro_rules! friendly_spec {
    ($(($variant:ident { $($field:ident : $kind:ident),+ $(,)? })),+ $(,)?) => {
        /// One action in a build spec. The `kind` tag is the stable data contract.
        ///
        /// Transfer actions carry an optional `from` address: when absent the
        /// action transfers from the transaction main address (`*ToTrs`); when
        /// present the action is a `*FromToTrs` transfer out of that address,
        /// which then becomes a required signer. The SDK never rewrites an
        /// explicit `from` (even one equal to `main`) — the wire form is the
        /// caller's choice, the chain decides acceptance.
        #[derive(Debug, Clone)]
        pub enum ActionSpec {
            $($variant { $($field: friendly_field_ty!($kind)),+ }),+,
            /// Generic wire-shaped action (design A): any kind the codec schema
            /// registry knows, with its wire field names and values. This is
            /// how the full protocol action surface stays reachable without
            /// hand-written constructors: the build path re-encodes the fields
            /// per the schema and constructs the native action through the
            /// protocol's own action decoder.
            RawAction {
                kind: String,
                fields: Vec<(String, WireValue)>,
            },
        }

        impl ActionSpec {
            /// Canonical JSON of one action spec (friendly field names; `from`
            /// omitted when absent). Used by the golden-vector tests and the
            /// dispatcher output.
            pub fn to_json_string(&self) -> String {
                use crate::json::{kv, obj, q};
                match self {
                    $(
                        ActionSpec::$variant { $($field,)+ } => obj(vec![
                            $(friendly_spec_kv!($field, $kind)),+
                        ]),
                    )+
                    ActionSpec::RawAction { kind, fields } => {
                        let mut parts = vec![kv("kind", q(kind))];
                        parts.extend(fields.iter().map(|(name, value)| kv(name, wire_value_json(value))));
                        obj(parts)
                    }
                }
            }

            /// Variant name (the friendly group key in `ACTION_SPECS`).
            fn variant_name(&self) -> &'static str {
                match self {
                    $(ActionSpec::$variant { .. } => stringify!($variant),)+
                    ActionSpec::RawAction { .. } => "RawAction",
                }
            }

            /// Friendly (name, value) pairs for the table-driven wire build.
            fn friendly_fields(&self) -> Vec<(&'static str, FriendlyValue)> {
                match self {
                    $(
                        ActionSpec::$variant { $($field,)+ } => vec![
                            $((stringify!($field), friendly_value!($field, $kind))),+
                        ],
                    )+
                    ActionSpec::RawAction { .. } => Vec::new(),
                }
            }
        }
    };
}

friendly_spec! {
    (HacTransfer { from: opt_str, to: str, amount: str }),
    (SatTransfer { from: opt_str, to: str, satoshi: num64 }),
    (HacdTransfer { from: opt_str, to: str, names: str_list }),
    (AssetTransfer { from: opt_str, to: str, serial: num64, amount: str }),
    (HeightScope { start: num64, end: num64 }),
    (ChainAllow { chains: num_list }),
    (ReqSignList { signers: str_list }),
    (TxMessage { data: str }),
    (TxBlob { data: str }),
    (InscPush { diamonds: str_list, protocol_cost: opt_str, engraved_type: opt_num8, engraved_content: str }),
    (InscClean { diamonds: str_list, protocol_cost: opt_str }),
    (InscEdit { diamond: str, index: num8, protocol_cost: opt_str, engraved_type: opt_num8, engraved_content: str }),
    (InscMove { from_diamond: str, to_diamond: str, index: num8, protocol_cost: opt_str }),
    (InscDrop { diamond: str, index: num8, protocol_cost: opt_str }),
    (ChannelOpen { channel_id: str, left_address: str, left_amount: str, right_address: str, right_amount: str }),
    (ChannelClose { channel_id: str }),
    (AssetCreate { ticket: str, name: str, serial: str, supply: str, decimal: str, issuer: str, protocol_cost: str }),
    (DiamondMint { diamond: str, number: str, prev_hash: str, nonce: str, address: str, custom_message: opt_str }),
}

#[derive(Debug, Clone)]
pub struct TransactionSpec {
        pub schema: Option<String>,
    pub tx_type: u8,
    pub main: String,
    pub fee: String,
        pub timestamp: Option<u64>,
        pub gas_max: Option<u8>,
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTransaction {
    pub schema: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
    pub hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub body: String,
}

pub fn build_transaction(spec: &TransactionSpec) -> Result<BuiltTransaction, SdkError> {
    if let Some(schema) = &spec.schema {
        if schema != SCHEMA_TRANSACTION_SPEC {
            return Err(SdkError::new(
                SdkErrorCode::UnsupportedSchema,
                format!("unsupported spec schema {schema:?}"),
            ));
        }
    }
    let main = Address::from_readable(&spec.main).map_err(|error| SdkError::from(error))?;
    let fee = Amount::from(&spec.fee).map_err(|error| SdkError::from(error))?;
    let fee_fin = fee.to_fin_string();
    let timestamp = spec.timestamp.unwrap_or_else(crate::now_secs);

    let mut actions = Vec::with_capacity(spec.actions.len());
    for action in &spec.actions {
        actions.push(build_action(action)?);
    }

    // The body is built by the protocol's own standard-tx constructor: the
    // type list and the gas rule (only type 3 carries a gas budget) live in
    // `protocol::tx_std`, not here.
    let body = protocol::tx_std::encode_standard_tx(
        TxCreateRequest::new(spec.tx_type, main, fee, timestamp)
            .with_gas_max(spec.gas_max.unwrap_or(0)),
        &actions,
        &[],
    )
    .map_err(SdkError::from)?;

    // Round-trip invariant: encode(decode(body)) == body must hold.
    let body_hex = hex::encode(&body);
    let decoded = decode_tx(&body)?;
    let re_encoded = decoded.encode();
    if re_encoded != body {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "built body failed the encode(decode(body)) == body round-trip",
        ));
    }
    let unsigned_body_hash = crate::audit::unsigned_body_hash(&body_hex)?;
    Ok(BuiltTransaction {
        schema: SCHEMA_BUILT_TRANSACTION.to_owned(),
        tx_type: spec.tx_type,
        timestamp,
        main: main.to_readable(),
        fee: fee_fin,
        hash: hex::encode(decoded.hash().0),
        hash_with_fee: hex::encode(decoded.hash_with_fee().0),
        unsigned_body_hash,
        body: body_hex,
    })
}

/// Build one action. Typed variants go through the table-driven
/// `friendly_to_wire`; `RawAction` goes through the generic wire-shaped path;
/// both end in the same schema-driven encode + protocol decode.
fn build_action(spec: &ActionSpec) -> Result<base::ActionRef, SdkError> {
    match spec {
        ActionSpec::RawAction { kind, fields } => build_raw(kind, fields),
        typed => {
            let (kind, fields) = friendly_to_wire(typed)?;
            build_raw(&kind, &fields)
        }
    }
}

/// Generic wire build: design-A wire fields → native action payload bytes →
/// the protocol's own action decoder. This is the single construction path
/// for every action kind the codec schema registry knows.
fn build_raw(kind: &str, fields: &[(String, WireValue)]) -> Result<base::ActionRef, SdkError> {
    let mut buf = Vec::new();
    crate::spec_codec::encode_action(&mut buf, kind, fields).map_err(SdkError::from)?;
    let codecs = crate::codec::standard_codecs().map_err(SdkError::from)?;
    let (action, used) = codecs.decode_action(&buf).map_err(|error| {
        // Two distinct failures surface here: kinds without a transaction
        // action codec in the protocol registry, and payloads the codec
        // rejects (e.g. a diamond_mint above the custom-message threshold
        // without its `custom_message` field). Both are the chain's boundary.
        SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("action {kind:?} has no usable transaction action codec ({error})"),
        )
    })?;
    // The protocol decoder may consume less than the payload when a native
    // codec conditionally trims design-A fields (`diamond_mint` drops
    // `custom_message` below the consensus threshold). Verify the consumed
    // part is the canonical native form instead of failing the build.
    let canonical = action.encode();
    if canonical.len() > buf.len() || &buf[..canonical.len()] != canonical.as_slice() {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!(
                "action {kind:?} decoded {used} of {} bytes and the consumed form does not re-encode",
                buf.len()
            ),
        ));
    }
    Ok(action)
}

// ================================ table-driven friendly → wire ================================
//
// The friendly→wire conversion is derived from `ACTION_SPECS` (the same table
// the JS adapter is generated from) plus the wire schemas: kind selection via
// the shared group analysis, field names/defaults from the table entries,
// value conversion per the wire shape. No per-variant constructor code.

fn find_value<'a>(
    values: &'a [(&'static str, FriendlyValue)],
    name: &str,
) -> Option<&'a FriendlyValue> {
    values.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
}

/// Select the wire kind for a friendly variant, mirroring the JS adapter's
/// selection (shared `friendly_groups` analysis) plus the from-only rebuild
/// path. An explicit `from` always selects the `*FromToTrs` form — the SDK
/// never rewrites it, even when it equals `main` (that choice is the
/// caller's; both wire forms are chain-valid).
fn select_wire_kind(
    group: &FriendlyGroup<'static>,
    values: &[(&'static str, FriendlyValue)],
) -> Result<&'static str, SdkError> {
    let from = find_value(values, "from").and_then(FriendlyValue::as_str);
    let to = find_value(values, "to").and_then(FriendlyValue::as_str);
    let names_len = find_value(values, "names")
        .and_then(FriendlyValue::as_str_list)
        .map_or(0, |names| names.len());
    let missing = |what: &str| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            format!(
                "friendly group {} has no {} wire form",
                group.friendly, what
            ),
        )
    };
    if let Some(from) = from {
        if to == Some("") {
            // A decoded from-only form (`transfer_hac_from`, `to` empty)
            // rebuilds as the same wire kind instead of an invalid from_to.
            if let Some(kind) = group.from_only_kind {
                return Ok(kind);
            }
        }
        return group
            .from_to_kind
            .map(Ok)
            .unwrap_or_else(|| Err(missing("from_to")));
    }
    if names_len == 1 {
        if let Some(single) = group.single_entry {
            return Ok(single.kind);
        }
    }
    if let Some(kind) = group.to_kind {
        return Ok(kind);
    }
    // Fixed groups without a `to` field (height_scope etc.): the single entry.
    ACTION_SPECS
        .iter()
        .find(|def| crate::actionspec::friendly_of(def.kind) == Some(group.friendly))
        .map(|def| def.kind)
        .ok_or_else(|| missing("wire"))
}

/// Friendly typed variant → (wire kind, design-A wire fields).
fn friendly_to_wire(spec: &ActionSpec) -> Result<(String, Vec<(String, WireValue)>), SdkError> {
    let variant = spec.variant_name();
    let values = spec.friendly_fields();
    let group = friendly_groups()
        .into_iter()
        .find(|group| {
            ACTION_SPECS.iter().any(|def| {
                crate::actionspec::friendly_of(def.kind) == Some(group.friendly)
                    && def.variant == variant
            })
        })
        .ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("no friendly group for variant {variant}"),
            )
        })?;
    let kind = select_wire_kind(&group, &values)?;
    let entry = ACTION_SPECS
        .iter()
        .find(|def| def.kind == kind)
        .expect("selected kind is in the table");
    let fields = wire_fields(entry, &values)?;
    Ok((kind.to_owned(), fields))
}

/// Convert one entry's friendly fields into design-A wire fields, using the
/// table's field names/defaults and the wire schema for value conversion.
fn wire_fields(
    entry: &'static ActionSpecDef,
    values: &[(&'static str, FriendlyValue)],
) -> Result<Vec<(String, WireValue)>, SdkError> {
    let mut top: Vec<(String, WireValue)> = Vec::new();
    let mut structs: Vec<(String, Vec<(String, WireValue)>)> = Vec::new();
    for field in entry.fields {
        let value = find_value(values, field.friendly);
        match field.js {
            JsConv::StructField(struct_name, sub, _subconv) => {
                // The sub-conversion is driven by the member's wire shape
                // (same table entry on the JS side); defaults are handled by
                // the conversion below when the value is absent.
                let value = match value {
                    Some(v) if !v.is_none() => v.clone(),
                    // Optional struct members (schema `optional`: native
                    // presence is the owning codec's decision) stay absent
                    // when the friendly value is absent.
                    _ if matches!(field.rust, RustConv::StructOptStr(..)) => continue,
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!("friendly field {} missing", field.friendly),
                        ));
                    }
                };
                let wire = crate::spec_codec::struct_member_wire(entry.kind, struct_name, sub)
                    .ok_or_else(|| {
                        SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!(
                                "{} struct member {struct_name}.{sub} not found in the wire schemas",
                                entry.kind
                            ),
                        )
                    })?;
                let wv = convert_scalar(&wire, &value)?;
                push_struct_member(&mut structs, struct_name, sub, wv);
            }
            JsConv::Noop => {
                // The `from`/`to` placeholders: emit only when the wire
                // schema actually carries the field (the `none()` and
                // `empty()` placeholders are never wire fields) and the
                // friendly value is present.
                if matches!(field.rust, RustConv::ConstNone | RustConv::ConstEmpty) {
                    continue;
                }
                let Some(value) = value else { continue };
                if value.is_none() {
                    continue;
                }
                let wire = crate::spec_codec::schema_wire_of(entry.kind, field.friendly)
                    .ok_or_else(|| {
                        SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!(
                                "{} wire field {} not found in the action schema",
                                entry.kind, field.friendly
                            ),
                        )
                    })?;
                top.push((field.friendly.to_owned(), convert_scalar(&wire, value)?));
            }
            _ => {
                let wire_name = js_wire_name(field);
                let value = match (value, field.js) {
                    // Single-diamond form: the friendly `names` list collapses
                    // to its first element (the JS adapter's `hex_single`).
                    (Some(FriendlyValue::StrList(items)), JsConv::HexSingle(_)) => {
                        FriendlyValue::Str(items.first().cloned().ok_or_else(|| {
                            SdkError::new(
                                SdkErrorCode::ParseFailed,
                                format!("friendly field {} is empty", field.friendly),
                            )
                        })?)
                    }
                    (Some(v), _) if !v.is_none() => v.clone(),
                    (_, js) => default_value(js).ok_or_else(|| {
                        SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!("friendly field {} missing", field.friendly),
                        )
                    })?,
                };
                let wire =
                    crate::spec_codec::schema_wire_of(entry.kind, wire_name).ok_or_else(|| {
                        SdkError::new(
                            SdkErrorCode::ParseFailed,
                            format!(
                                "{} wire field {wire_name} not found in the action schema",
                                entry.kind
                            ),
                        )
                    })?;
                top.push((wire_name.to_owned(), convert_scalar(&wire, &value)?));
            }
        }
    }
    for (name, members) in structs {
        top.push((name, WireValue::Struct(members)));
    }
    Ok(top)
}

/// Wire field name of one table field (top-level JsConv forms).
fn js_wire_name(field: &FieldDef) -> &'static str {
    match field.js {
        JsConv::Rename(w)
        | JsConv::RenameDef(w, _)
        | JsConv::RenameDefNum(w, _)
        | JsConv::ToString(w, _)
        | JsConv::NumList(w)
        | JsConv::Hex(w)
        | JsConv::HexList(w)
        | JsConv::HexSingle(w)
        | JsConv::HexOrKeep(w, _)
        | JsConv::Strip0x(w, _) => w,
        JsConv::Noop | JsConv::StructField(..) => field.friendly,
    }
}

/// Default friendly value for an absent optional field, from the table's
/// JsConv defaults (the same defaults the JS adapter writes).
fn default_value(js: JsConv) -> Option<FriendlyValue> {
    match js {
        JsConv::RenameDef(_, d)
        | JsConv::ToString(_, d)
        | JsConv::HexOrKeep(_, d)
        | JsConv::Strip0x(_, d) => Some(FriendlyValue::Str(d.to_owned())),
        JsConv::RenameDefNum(_, d) => d.parse().ok().map(FriendlyValue::Num),
        JsConv::NumList(_) | JsConv::HexList(_) => Some(FriendlyValue::StrList(Vec::new())),
        JsConv::Noop
        | JsConv::Rename(_)
        | JsConv::Hex(_)
        | JsConv::HexSingle(_)
        | JsConv::StructField(..) => None,
    }
}

/// Friendly value → design-A wire value, per the wire shape. This is the
/// inverse of `spec_codec::decode_wire`: strings/hex/lists of the same
/// transport form, so the built payload and the decoded payload agree.
fn convert_scalar(wire: &base::FieldWire, value: &FriendlyValue) -> Result<WireValue, SdkError> {
    use base::FieldWire;
    match wire {
        FieldWire::U1 | FieldWire::U2 | FieldWire::U4 | FieldWire::U5 | FieldWire::U8 => {
            Ok(WireValue::Num(value.as_num().ok_or_else(|| {
                SdkError::new(SdkErrorCode::ParseFailed, "numeric friendly field invalid")
            })?))
        }
        FieldWire::Fixed(_) => Ok(WireValue::Hex(decode_hex_strict(friendly_str(value)?)?)),
        FieldWire::Amount
        | FieldWire::WireAmount
        | FieldWire::Address
        | FieldWire::AddrOrPtr
        | FieldWire::AddrOrList
        | FieldWire::Satoshi
        | FieldWire::Fold64
        | FieldWire::Timestamp
        | FieldWire::DiamondNumber => Ok(WireValue::Str(friendly_to_string(value).ok_or_else(
            || SdkError::new(SdkErrorCode::ParseFailed, "friendly field is not a string"),
        )?)),
        FieldWire::BytesW1 | FieldWire::BytesW2 => {
            Ok(WireValue::Hex(hex_or_utf8(friendly_str(value)?)?))
        }
        FieldWire::DiamondName => Ok(WireValue::Hex(friendly_str(value)?.as_bytes().to_vec())),
        FieldWire::DiamondNameList => Ok(WireValue::List(
            friendly_str_list(value)?
                .iter()
                .map(|name| WireValue::Hex(name.as_bytes().to_vec()))
                .collect(),
        )),
        FieldWire::ChainIDList => Ok(WireValue::List(
            friendly_num_list(value)?
                .iter()
                .map(|id| WireValue::Hex(id.to_be_bytes().to_vec()))
                .collect(),
        )),
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let elem = crate::spec_codec::element_wire(wire);
            let items: Result<Vec<WireValue>, SdkError> = match &elem {
                // Numeric element wires (e.g. `ListW1<Uint4>` chain ids)
                // carry raw numbers in the design-A transport.
                FieldWire::U1 | FieldWire::U2 | FieldWire::U4 | FieldWire::U5 | FieldWire::U8 => {
                    friendly_num_list(value)?
                        .iter()
                        .map(|item| convert_scalar(&elem, &FriendlyValue::Num(*item as u64)))
                        .collect()
                }
                _ => friendly_str_list(value)?
                    .iter()
                    .map(|item| convert_scalar(&elem, &FriendlyValue::Str(item.clone())))
                    .collect(),
            };
            Ok(WireValue::List(items?))
        }
        other => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("unsupported wire shape {other:?} in the friendly build path"),
        )),
    }
}

fn friendly_str(value: &FriendlyValue) -> Result<&str, SdkError> {
    value
        .as_str()
        .ok_or_else(|| SdkError::new(SdkErrorCode::ParseFailed, "friendly field is not a string"))
}

/// String form of a friendly value: strings pass through, numeric values
/// render as decimal (the design-A wire carries them as strings).
fn friendly_to_string(value: &FriendlyValue) -> Option<String> {
    match value {
        FriendlyValue::Str(s) => Some(s.clone()),
        FriendlyValue::OptStr(Some(s)) => Some(s.clone()),
        FriendlyValue::Num(n) => Some(n.to_string()),
        FriendlyValue::Num8(n) => Some(n.to_string()),
        _ => None,
    }
}

fn friendly_str_list(value: &FriendlyValue) -> Result<&[String], SdkError> {
    value.as_str_list().ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            "friendly field is not a string list",
        )
    })
}

fn friendly_num_list(value: &FriendlyValue) -> Result<&[u32], SdkError> {
    value.as_num_list().ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            "friendly field is not a numeric list",
        )
    })
}

/// Strict hex decode with an optional `0x` prefix (fixed-length wire fields).
fn decode_hex_strict(raw: &str) -> Result<Vec<u8>, SdkError> {
    hex::decode(raw.trim_start_matches("0x").trim_start_matches("0X")).map_err(|_| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("field must be hex, got {raw:?}"),
        )
    })
}

/// Hex-or-text decode: `0x`-prefixed or valid hex strings decode as hex
/// (the wire form the JS adapter produces), anything else is UTF-8 text.
/// This makes the friendly↔wire round-trip lossless for the hex-carrying
/// fields (inscription content, hashes, asset metadata) while direct Rust
/// callers can still pass plain text.
fn hex_or_utf8(raw: &str) -> Result<Vec<u8>, SdkError> {
    let clean = raw.trim_start_matches("0x").trim_start_matches("0X");
    if clean.len() % 2 == 0 && clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return hex::decode(clean).map_err(|_| {
            SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("field hex invalid: {raw:?}"),
            )
        });
    }
    Ok(raw.as_bytes().to_vec())
}

fn push_struct_member(
    structs: &mut Vec<(String, Vec<(String, WireValue)>)>,
    name: &str,
    member: &str,
    value: WireValue,
) {
    if let Some((_, members)) = structs.iter_mut().find(|(n, _)| n == name) {
        members.push((member.to_owned(), value));
    } else {
        structs.push((name.to_owned(), vec![(member.to_owned(), value)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn sample_spec() -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 2,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![
                ActionSpec::HacTransfer {
                    from: None,
                    to: MAIN.to_owned(),
                    amount: "12:244".to_owned(),
                },
                ActionSpec::HeightScope {
                    start: 1_000_000,
                    end: 0,
                },
            ],
        }
    }

    #[test]
    fn builds_type2_and_round_trips() {
        let built = build_transaction(&sample_spec()).unwrap();
        assert_eq!(built.tx_type, 2);
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(hex::encode(decoded.encode()), built.body);
        assert_eq!(decoded.action_count(), 2);
    }

    #[test]
    fn delegates_unknown_tx_type_to_protocol_constructor() {
        let mut spec = sample_spec();
        spec.tx_type = 0;
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(
            error
                .message
                .contains("unsupported standard user transaction type 0")
        );
    }

    #[test]
    fn rejects_non_type3_with_gas_max() {
        let mut spec = sample_spec();
        spec.tx_type = 2;
        spec.gas_max = Some(10);
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
    }

    #[test]
    fn inscription_content_hex_and_text_forms() {
        // Hex-carrying content fields round-trip losslessly (the wire form
        // the JS adapter produces) and plain text stays UTF-8.
        let hex = hex_or_utf8("68656c6c6f").unwrap();
        assert_eq!(hex, b"hello");
        assert_eq!(hex_or_utf8("0x00ff80").unwrap(), &[0x00, 0xff, 0x80]);
        assert_eq!(hex_or_utf8("plain text").unwrap(), b"plain text");
    }

    #[test]
    fn explicit_from_builds_from_to_transfer_and_becomes_signer() {
        let other = sys::Account::create_by("654321").unwrap();
        let mut spec = sample_spec();
        spec.actions[0] = ActionSpec::HacTransfer {
            from: Some(other.readable().to_owned()),
            to: MAIN.to_owned(),
            amount: "12:244".to_owned(),
        };
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::HacFromToTrs::KIND
        );
        // The explicit from address becomes a required signer.
        let required = decoded.req_sign().unwrap();
        let other_address = field::Address::from_readable(other.readable()).unwrap();
        assert!(required.contains(&other_address));
    }

    #[test]
    fn explicit_from_equal_to_main_builds_the_from_to_form() {
        let mut spec = sample_spec();
        spec.actions[0] = ActionSpec::HacTransfer {
            from: Some(MAIN.to_owned()),
            to: MAIN.to_owned(),
            amount: "12:244".to_owned(),
        };
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::HacFromToTrs::KIND,
            "an explicit from equal to main must keep the from_to wire form (the SDK never rewrites it)"
        );
    }

    fn raw_spec(kind: &str, fields: Vec<(String, WireValue)>) -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![ActionSpec::RawAction {
                kind: kind.to_owned(),
                fields,
            }],
        }
    }

    /// Every action the codec schema registry knows builds through the generic
    /// raw path; the protocol's own decoder constructs the native action and
    /// the encode(decode(body)) == body invariant holds. Representatives cover
    /// the different wire shapes: amounts/addresses/lists (balance_floor),
    /// fixed/bytes (contract_main_call) and nested action lists (ast_select).
    #[test]
    fn raw_actions_build_through_the_protocol_codec() {
        let cases: Vec<(&str, Vec<(String, WireValue)>)> = vec![
            (
                "balance_floor",
                vec![
                    ("addr".to_owned(), WireValue::Str(MAIN.to_owned())),
                    ("hacash".to_owned(), WireValue::Str("12:244".to_owned())),
                    ("satoshi".to_owned(), WireValue::Str("100".to_owned())),
                    ("diamond".to_owned(), WireValue::Str("5".to_owned())),
                    (
                        "assets".to_owned(),
                        WireValue::List(vec![WireValue::Struct(vec![
                            ("serial".to_owned(), WireValue::Str("7".to_owned())),
                            ("amount".to_owned(), WireValue::Str("100".to_owned())),
                        ])]),
                    ),
                ],
            ),
            (
                "contract_main_call",
                vec![
                    ("marks".to_owned(), WireValue::Hex(vec![0, 0, 0])),
                    ("codeconf".to_owned(), WireValue::Num(1)),
                    ("codes".to_owned(), WireValue::Hex(vec![0x01, 0x02, 0x03])),
                ],
            ),
            (
                "ast_select",
                vec![
                    ("exe_min".to_owned(), WireValue::Num(1)),
                    ("exe_max".to_owned(), WireValue::Num(1)),
                    (
                        "actions".to_owned(),
                        WireValue::List(vec![WireValue::Struct(vec![
                            (
                                "kind".to_owned(),
                                WireValue::Str("transfer_hac_to".to_owned()),
                            ),
                            ("to".to_owned(), WireValue::Str(MAIN.to_owned())),
                            ("hacash".to_owned(), WireValue::Str("12:244".to_owned())),
                        ])]),
                    ),
                ],
            ),
        ];
        let expected_kinds = [
            protocol::action_std::BalanceFloor::KIND,
            vm::action::ContractMainCall::KIND,
            protocol::action_std::AstSelect::KIND,
        ];
        for ((kind, fields), expected) in cases.into_iter().zip(expected_kinds) {
            let built = build_transaction(&raw_spec(kind, fields)).expect(kind);
            let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
            assert_eq!(
                hex::encode(decoded.encode()),
                built.body,
                "{kind}: raw build must round-trip the tx body"
            );
            assert_eq!(
                decoded.actions()[0].kind(),
                expected,
                "{kind}: the protocol's own decoder must construct the native action"
            );
        }
    }

    /// Every registered action kind — including the VM host opcodes — builds
    /// through the raw path: they have a transaction action codec, so the SDK
    /// exposes them. Whether the chain accepts one in a body is decided by the
    /// chain's scope validation (`ActScope::CALL_ONLY` for host opcodes), not
    /// by the SDK.
    #[test]
    fn raw_host_opcode_kind_builds_and_round_trips() {
        let built = build_transaction(&raw_spec("block_height", vec![])).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            hex::encode(decoded.encode()),
            built.body,
            "host opcode body must round-trip (scope validation is the chain's, not the SDK's)"
        );
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::EnvHeight::KIND
        );
    }

    /// Every golden payload (the same vectors that lock the decode direction)
    /// must rebuild through the table-driven typed path: the native action
    /// kinds of the built body equal the kinds the payload declared. This
    /// locks the encode direction of the friendly↔wire mapping against the
    /// hand-generated payloads.
    #[test]
    fn golden_vectors_rebuild_through_the_typed_path() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden.json");
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let mut vectors = 0usize;
        for (_, value) in field::json_split_object(&json).expect("golden object") {
            if !value.starts_with('[') {
                continue;
            }
            for vector in field::json_split_array(value).expect("vectors") {
                let mut payload_hex = String::new();
                for (key, v) in field::json_split_object(vector).expect("vector") {
                    if key == "payload" {
                        payload_hex = field::json_expect_quoted_decoded(v)
                            .expect("payload")
                            .to_owned();
                    }
                }
                let bytes = hex::decode(&payload_hex).expect("payload hex");
                let (kinds, spec) = crate::spec_codec::decode_transaction_spec_parts(&bytes)
                    .expect("payload decodes");
                let built = build_transaction(&spec)
                    .unwrap_or_else(|e| panic!("vector {vectors}: rebuild failed: {e}"));
                let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
                let native: Vec<u16> = decoded.actions().iter().map(|a| a.kind()).collect();
                assert_eq!(
                    native, kinds,
                    "vector {vectors}: built body action kinds must equal the payload kinds"
                );
                vectors += 1;
            }
        }
        assert!(
            vectors >= 20,
            "expected a full golden vector set, got {vectors}"
        );
    }
}

/// JSON rendering of a wire value (design A): numbers as decimal strings, hex
/// as hex strings, lists/structs recursively.
fn wire_value_json(value: &WireValue) -> String {
    use crate::json::{arr, obj, q};
    match value {
        WireValue::Num(n) => n.to_string(),
        WireValue::Str(s) => q(s),
        WireValue::Hex(b) => q(&hex::encode(b)),
        WireValue::List(items) => arr(items.iter().map(wire_value_json).collect()),
        WireValue::Struct(items) => obj(
            items
                .iter()
                .map(|(name, value)| crate::json::kv(name, wire_value_json(value)))
                .collect(),
        ),
    }
}
