//! Hand-written JSON encode/decode layer (replaces serde/serde_json).
//!
//! The body layer of the binary transport: results travel as JSON strings
//! (W2-wrapped), and complex objects in requests (Review/Policy/...) are also
//! transmitted as JSON strings. This layer only moves structure — fields with
//! real semantics such as amounts/addresses/hex stay strings (design A), with
//! parsing left to Rust's consensus layer. The TS side reads results with
//! native `JSON.parse`, zero hand-written parsing.

use sys::{Ret, errf};

use crate::audit::{ActionDesc, PayloadDesc, TransferDesc};
use crate::build::BuiltTransaction;
use crate::inspect::{
    HeightRangeDesc, InspectContext, Review, SignatureEntry, TransactionJson,
};
use crate::policy::{Policy, PolicyDecision};
use crate::profile::{
    AbiVersion, Capabilities, CodecProfile, FeatureItem, LimitsProfile,
    ProtocolParamsProfile,
};
use crate::attach::{
    AttachResult, SignatureProof, SignatureReport, SigningRequest, VerifyResult,
};
use crate::error::SdkError;
use crate::message::{MessagePrepareParams, MessageVerifyResult};
use crate::account::{AddressFromPublicKeyResult, VerifyAddressResult};
use crate::amount::ParsedAmount;

// ================================ serialization helpers ================================

/// JSON string escaping (for `"`, `\`, and control characters).
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Quoted string `"..."`.
pub(crate) fn q(s: &str) -> String {
    format!("\"{}\"", esc(s))
}

/// Object `{"k":v,...}` (empty-string entries are skipped, used for Option fields).
pub(crate) fn obj(parts: Vec<String>) -> String {
    let filtered: Vec<&str> = parts.iter().filter(|p| !p.is_empty()).map(|s| s.as_str()).collect();
    format!("{{{}}}", filtered.join(","))
}

/// Array `[...]`.
pub(crate) fn arr(parts: Vec<String>) -> String {
    format!("[{}]", parts.join(","))
}

/// Key-value pair `"k":v`.
pub(crate) fn kv(key: &str, value: String) -> String {
    format!("\"{}\":{}", esc(key), value)
}

/// Option field: None returns an empty string (filtered out by `obj`; matches
/// serde's `skip_serializing_if`).
pub(crate) fn kv_opt(key: &str, value: Option<String>) -> String {
    value.map(|v| kv(key, v)).unwrap_or_default()
}

// ================================ parsing helpers ================================

pub(crate) fn parse_str(value: &str) -> Ret<String> {
    field::json_expect_quoted_decoded(value)
}

pub(crate) fn parse_unquoted(value: &str) -> Ret<&str> {
    field::json_expect_unquoted(value)
}

pub(crate) fn parse_u64(value: &str) -> Ret<u64> {
    parse_unquoted(value)?
        .parse()
        .map_err(|_| sys::Error::fault(format!("json number invalid: {}", value)))
}

pub(crate) fn parse_u32(value: &str) -> Ret<u32> {
    parse_unquoted(value)?
        .parse()
        .map_err(|_| sys::Error::fault(format!("json number invalid: {}", value)))
}

pub(crate) fn parse_u16(value: &str) -> Ret<u16> {
    parse_unquoted(value)?
        .parse()
        .map_err(|_| sys::Error::fault(format!("json number invalid: {}", value)))
}

pub(crate) fn parse_usize(value: &str) -> Ret<usize> {
    parse_unquoted(value)?
        .parse()
        .map_err(|_| sys::Error::fault(format!("json number invalid: {}", value)))
}

pub(crate) fn parse_bool(value: &str) -> Ret<bool> {
    match parse_unquoted(value)? {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => errf!("json bool invalid: {}", value),
    }
}

pub(crate) fn parse_str_array(value: &str) -> Ret<Vec<String>> {
    let items = field::json_split_array(value)?;
    items.iter().map(|item| parse_str(item)).collect()
}

pub(crate) fn parse_u16_array(value: &str) -> Ret<Vec<u16>> {
    let items = field::json_split_array(value)?;
    items.iter().map(|item| parse_u16(item)).collect()
}

pub(crate) fn parse_u32_array(value: &str) -> Ret<Vec<u32>> {
    let items = field::json_split_array(value)?;
    items.iter().map(|item| parse_u32(item)).collect()
}

pub(crate) fn parse_u64_array(value: &str) -> Ret<Vec<u64>> {
    let items = field::json_split_array(value)?;
    items.iter().map(|item| parse_u64(item)).collect()
}

/// Extract one field from an object JSON (missing → None; `null` → None).
/// Duplicate keys error out (consistent with serde's duplicate-field
/// behavior, avoiding the ambiguity of taking the first).
pub(crate) fn field_of<'a>(json: &'a str, key: &str) -> Ret<Option<&'a str>> {
    let mut found = None;
    for (k, v) in field::json_split_object(json)? {
        if k == key {
            if found.is_some() {
                return errf!("json field {} is duplicated", key);
            }
            found = Some(v);
        }
    }
    match found {
        None => Ok(None),
        Some("null") => Ok(None),
        Some(v) => Ok(Some(v)),
    }
}

/// Reject unknown fields (same behavior as the legacy serde `deny_unknown_fields`).
pub(crate) fn reject_unknown_keys(json: &str, known: &[&str]) -> Ret<()> {
    for (k, _) in field::json_split_object(json)? {
        if !known.contains(&k) {
            return errf!("json unknown field {}", k);
        }
    }
    Ok(())
}

/// Required string field.
pub(crate) fn req_str(json: &str, key: &str) -> Ret<String> {
    field_of(json, key)?
        .ok_or_else(|| sys::Error::fault(format!("json missing required field {}", key)))
        .and_then(parse_str)
}

/// Required numeric/bool field.
pub(crate) fn req_raw<'a>(json: &'a str, key: &str) -> Ret<&'a str> {
    field_of(json, key)?
        .ok_or_else(|| sys::Error::fault(format!("json missing required field {}", key)))
        .and_then(parse_unquoted)
}

/// Optional string field.
pub(crate) fn opt_str(json: &str, key: &str) -> Ret<Option<String>> {
    match field_of(json, key)? {
        Some(v) => parse_str(v).map(Some),
        None => Ok(None),
    }
}

/// Required numeric field. Same semantics as the hand-written
/// `req_raw(...).and_then(parse_u64)` pattern: missing or quoted values are
/// rejected with the identical error messages.
pub(crate) fn req_num<T: std::str::FromStr>(json: &str, key: &str) -> Ret<T> {
    let value = req_raw(json, key)?;
    value
        .parse()
        .map_err(|_| sys::Error::fault(format!("json number invalid: {}", value)))
}

/// Optional numeric field.
pub(crate) fn opt_num<T: std::str::FromStr>(json: &str, key: &str) -> Ret<Option<T>> {
    match field_of(json, key)? {
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|_| sys::Error::fault(format!("json number invalid for {}: {}", key, v))),
        None => Ok(None),
    }
}

// ================================ simple-object JSON derive ================================
// Types whose JSON shape is a plain field list (no nested objects, no
// conditionals, no enums) get both directions generated here; the rest
// (Review/ActionDesc/PayloadDesc/Policy/CodecProfile/...) keep hand-written
// impls below because their layout carries structure, not just fields. The
// generated output is byte-identical to the hand-written code it replaces.

macro_rules! sdk_json_kv {
    ($self:ident, $field:ident, str) => {
        crate::json::kv(stringify!($field), crate::json::q(&$self.$field))
    };
    ($self:ident, $field:ident, opt_str) => {
        crate::json::kv_opt(stringify!($field), $self.$field.as_ref().map(|s| crate::json::q(s)))
    };
    ($self:ident, $field:ident, num) => {
        crate::json::kv(stringify!($field), $self.$field.to_string())
    };
    ($self:ident, $field:ident, opt_num) => {
        crate::json::kv_opt(stringify!($field), $self.$field.map(|v| v.to_string()))
    };
    ($self:ident, $field:ident, bool) => {
        crate::json::kv(stringify!($field), $self.$field.to_string())
    };
    ($self:ident, $field:ident, str_array) => {
        crate::json::kv(
            stringify!($field),
            crate::json::arr($self.$field.iter().map(|s| crate::json::q(s)).collect()),
        )
    };
}

macro_rules! sdk_json_expr {
    ($json:ident, $field:ident, str) => {
        crate::json::req_str($json, stringify!($field))?
    };
    ($json:ident, $field:ident, opt_str) => {
        crate::json::opt_str($json, stringify!($field))?
    };
    ($json:ident, $field:ident, num) => {
        crate::json::req_num($json, stringify!($field))?
    };
    ($json:ident, $field:ident, opt_num) => {
        crate::json::opt_num($json, stringify!($field))?
    };
    ($json:ident, $field:ident, bool) => {
        crate::json::req_raw($json, stringify!($field)).and_then(crate::json::parse_bool)?
    };
    ($json:ident, $field:ident, str_array) => {
        match crate::json::field_of($json, stringify!($field))? {
            Some(v) => crate::json::parse_str_array(v)?,
            None => vec![],
        }
    };
}

/// Generate `to_json_string`/`from_json` for a simple object type (modes:
/// `to`, `from`, `both`). Field kinds: `str`/`opt_str`/`num`/`opt_num`/
/// `bool`/`str_array`.
macro_rules! impl_sdk_json {
    ($ty:ty { $($field:ident : $kind:ident),+ $(,)? } both) => {
        impl $ty {
            pub(crate) fn to_json_string(&self) -> String {
                crate::json::obj(vec![
                    $(sdk_json_kv!(self, $field, $kind)),+
                ])
            }

            pub(crate) fn from_json(json: &str) -> sys::Ret<Self> {
                $(let $field = sdk_json_expr!(json, $field, $kind);)+
                Ok(Self { $($field),+ })
            }
        }
    };
    ($ty:ty { $($field:ident : $kind:ident),+ $(,)? } to) => {
        impl $ty {
            pub(crate) fn to_json_string(&self) -> String {
                crate::json::obj(vec![
                    $(sdk_json_kv!(self, $field, $kind)),+
                ])
            }
        }
    };
    ($ty:ty { $($field:ident : $kind:ident),+ $(,)? } from) => {
        impl $ty {
            pub(crate) fn from_json(json: &str) -> sys::Ret<Self> {
                $(let $field = sdk_json_expr!(json, $field, $kind);)+
                Ok(Self { $($field),+ })
            }
        }
    };
}

// ================================ basic results ================================

impl_sdk_json! {
    VerifyAddressResult { ok: bool, error: opt_str, address: opt_str } to
}
impl_sdk_json! {
    AddressFromPublicKeyResult { address: str, version: num } to
}
impl_sdk_json! {
    ParsedAmount { value: str, unit: num, is_negative: bool } to
}
impl_sdk_json! {
    MessageVerifyResult { ok: bool, address: opt_str, error: opt_str } to
}

impl SdkError {
    /// JSON representation of the error object (reused in the envelope's err branch).
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(crate::schema::SCHEMA_ERROR)),
            kv("code", q(&self.code.to_string())),
            kv("message", q(&self.message)),
            kv_opt("detail", self.detail.clone()),
        ])
    }
}

// ================================ profile ================================

impl_sdk_json! {
    ProtocolParamsProfile {
        ast_tree_depth_max: num,
        max_type3_signers: num,
        fee_purity_floor: num,
        diamond_form_flag: num,
    } to
}
impl_sdk_json! {
    LimitsProfile { max_tx_size: num, tx_actions_max: num, hacd_wire_max: num } to
}

impl CodecProfile {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("sdk_version", q(&self.sdk_version)),
            kv("fullnode_commit", q(&self.fullnode_commit)),
            kv("protocol_params", self.protocol_params.to_json_string()),
            kv("limits", self.limits.to_json_string()),
            kv(
                "registered_kinds",
                arr(self.registered_kinds.iter().map(|k| k.to_string()).collect()),
            ),
            kv("schema_hash", q(&self.schema_hash)),
            kv("profile_hash", q(&self.profile_hash)),
        ])
    }
}

impl_sdk_json! {
    AbiVersion { major: num, minor: num } to
}
impl_sdk_json! {
    FeatureItem { id: str, version: num } to
}

impl Capabilities {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("package_version", q(&self.package_version)),
            kv("abi", self.abi.to_json_string()),
            kv("codec_profile_hash", q(&self.codec_profile_hash)),
            kv(
                "features",
                arr(self.features.iter().map(|f| f.to_json_string()).collect()),
            ),
        ])
    }
}

// ================================ audit ================================

impl PayloadDesc {
    pub(crate) fn to_json_string(&self) -> String {
        match self {
            PayloadDesc::Hac { amount } => obj(vec![
                kv("type", q("hac")),
                kv("amount", q(amount)),
            ]),
            PayloadDesc::Satoshi { atoms } => obj(vec![
                kv("type", q("satoshi")),
                kv("atoms", q(atoms)),
            ]),
            PayloadDesc::Hacd { count, names } => obj(vec![
                kv("type", q("hacd")),
                kv("count", count.to_string()),
                kv("names", arr(names.iter().map(|n| q(n)).collect())),
            ]),
            PayloadDesc::Asset { serial, atoms } => obj(vec![
                kv("type", q("asset")),
                kv("serial", q(serial)),
                kv("atoms", q(atoms)),
            ]),
        }
    }
}

impl TransferDesc {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv_opt("from", self.from.as_ref().map(|s| q(s))),
            kv("to", q(&self.to)),
            kv("payload", self.payload.to_json_string()),
        ])
    }
}

impl ActionDesc {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("index", self.index.to_string()),
            kv("path", q(&self.path)),
            kv("kind", self.kind.to_string()),
            kv_opt("name", self.name.as_ref().map(|s| q(s))),
            kv("scope", q(&self.scope)),
            kv("json", q(&self.json)),
            kv("raw", q(&self.raw)),
            kv("protocol_valid", self.protocol_valid.to_string()),
            kv("auditability", q(&self.auditability)),
            kv(
                "audit_notes",
                arr(self.audit_notes.iter().map(|n| q(n)).collect()),
            ),
            kv("blob", self.blob.to_string()),
            kv_opt(
                "transfer",
                self.transfer.as_ref().map(|t| t.to_json_string()),
            ),
            kv_opt(
                "children",
                self.children
                    .as_ref()
                    .map(|c| arr(c.iter().map(|a| a.to_json_string()).collect())),
            ),
        ])
    }
}

// ================================ inspect ================================

impl_sdk_json! {
    InspectContext { current_height: num, expected_chain_id: num } both
}

impl_sdk_json! {
    HeightRangeDesc { start: num, end: num } to
}

impl Review {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("codec_profile_hash", q(&self.codec_profile_hash)),
            kv("tx_type", self.tx_type.to_string()),
            kv("timestamp", self.timestamp.to_string()),
            kv("main", q(&self.main)),
            kv("fee", q(&self.fee)),
            kv_opt("gas_max", self.gas_max.map(|v| v.to_string())),
            kv("tx_hash", q(&self.tx_hash)),
            kv("hash_with_fee", q(&self.hash_with_fee)),
            kv("unsigned_body_hash", q(&self.unsigned_body_hash)),
            kv("review_binding", q(&self.review_binding)),
            kv_opt("signer_address", self.signer_address.as_ref().map(|s| q(s))),
            kv_opt(
                "inspect_context",
                self.inspect_context.as_ref().map(|c| c.to_json_string()),
            ),
            kv("protocol_valid", self.protocol_valid.to_string()),
            kv("signability", q(&self.signability)),
            kv("auditability", q(&self.auditability)),
            kv(
                "requires_user_confirmation",
                self.requires_user_confirmation.to_string(),
            ),
            // Omitted when empty: reviews without limit violations keep their
            // canonical digest stable across this additive field.
            kv_opt(
                "limits_violations",
                (!self.limits_violations.is_empty()).then(|| {
                    arr(self.limits_violations.iter().map(|v| q(v)).collect())
                }),
            ),
            kv(
                "required_signers",
                arr(self.required_signers.iter().map(|s| q(s)).collect()),
            ),
            kv(
                "present_signers",
                arr(self.present_signers.iter().map(|s| q(s)).collect()),
            ),
            kv(
                "missing_signers",
                arr(self.missing_signers.iter().map(|s| q(s)).collect()),
            ),
            kv_opt(
                "chain_ids_allowed",
                self.chain_ids_allowed
                    .as_ref()
                    .map(|v| arr(v.iter().map(|n| n.to_string()).collect())),
            ),
            kv_opt(
                "valid_height_range",
                self.valid_height_range.as_ref().map(|h| h.to_json_string()),
            ),
            kv_opt("fee_purity", self.fee_purity.map(|v| v.to_string())),
            kv_opt("fee_purity_ok", self.fee_purity_ok.map(|v| v.to_string())),
            kv(
                "actions",
                arr(self.actions.iter().map(|a| a.to_json_string()).collect()),
            ),
            kv(
                "asset_serials",
                arr(self.asset_serials.iter().map(|s| s.to_string()).collect()),
            ),
        ])
    }

    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        let actions = match field_of(json, "actions")? {
            Some(v) => field::json_split_array(v)?
                .iter()
                .map(|item| ActionDesc::from_json(item))
                .collect::<Ret<Vec<_>>>()?,
            None => {
                return Err(sys::Error::fault("json missing required field actions"));
            }
        };
        Ok(Self {
            schema: req_str(json, "schema")?,
            codec_profile_hash: req_str(json, "codec_profile_hash")?,
            tx_type: req_raw(json, "tx_type").and_then(|v| v.parse::<u8>().map_err(|_| sys::Error::fault("bad tx_type")))?,
            timestamp: req_raw(json, "timestamp").and_then(parse_u64)?,
            main: req_str(json, "main")?,
            fee: req_str(json, "fee")?,
            gas_max: opt_num(json, "gas_max")?,
            tx_hash: req_str(json, "tx_hash")?,
            hash_with_fee: req_str(json, "hash_with_fee")?,
            unsigned_body_hash: req_str(json, "unsigned_body_hash")?,
            review_binding: req_str(json, "review_binding")?,
            signer_address: opt_str(json, "signer_address")?,
            inspect_context: match field_of(json, "inspect_context")? {
                Some(v) => Some(InspectContext::from_json(v)?),
                None => None,
            },
            protocol_valid: req_raw(json, "protocol_valid").and_then(parse_bool)?,
            signability: req_str(json, "signability")?,
            auditability: req_str(json, "auditability")?,
            requires_user_confirmation: req_raw(json, "requires_user_confirmation")
                .and_then(parse_bool)?,
            limits_violations: match field_of(json, "limits_violations")? {
                Some(v) => parse_str_array(v)?,
                None => vec![],
            },
            required_signers: match field_of(json, "required_signers")? {
                Some(v) => parse_str_array(v)?,
                None => vec![],
            },
            present_signers: match field_of(json, "present_signers")? {
                Some(v) => parse_str_array(v)?,
                None => vec![],
            },
            missing_signers: match field_of(json, "missing_signers")? {
                Some(v) => parse_str_array(v)?,
                None => vec![],
            },
            chain_ids_allowed: match field_of(json, "chain_ids_allowed")? {
                Some(v) => Some(parse_u32_array(v)?),
                None => None,
            },
            valid_height_range: match field_of(json, "valid_height_range")? {
                Some(v) => {
                    let start = field_of(v, "start")?.and_then(|x| x.parse().ok());
                    let end = field_of(v, "end")?.and_then(|x| x.parse().ok());
                    match (start, end) {
                        (Some(start), Some(end)) => Some(HeightRangeDesc { start, end }),
                        _ => None,
                    }
                }
                None => None,
            },
            fee_purity: opt_num(json, "fee_purity")?,
            fee_purity_ok: opt_num(json, "fee_purity_ok")?,
            actions,
            asset_serials: match field_of(json, "asset_serials")? {
                Some(v) => parse_u64_array(v)?,
                None => vec![],
            },
        })
    }
}

impl_sdk_json! {
    SignatureEntry { public_key: str, signature: str } both
}

impl TransactionJson {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("tx_type", self.tx_type.to_string()),
            kv("timestamp", self.timestamp.to_string()),
            kv("main", q(&self.main)),
            kv("fee", q(&self.fee)),
            kv_opt("gas_max", self.gas_max.map(|v| v.to_string())),
            kv("tx_hash", q(&self.tx_hash)),
            kv("hash_with_fee", q(&self.hash_with_fee)),
            kv("unsigned_body_hash", q(&self.unsigned_body_hash)),
            kv(
                "actions",
                arr(self.actions.iter().map(|a| a.to_json_string()).collect()),
            ),
            kv(
                "signatures",
                arr(self.signatures.iter().map(|s| s.to_json_string()).collect()),
            ),
        ])
    }
}

impl ActionDesc {
    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        let transfer = match field_of(json, "transfer")? {
            Some(v) => Some(TransferDesc::from_json(v)?),
            None => None,
        };
        let children = match field_of(json, "children")? {
            Some(v) => Some(
                field::json_split_array(v)?
                    .iter()
                    .map(|item| ActionDesc::from_json(item))
                    .collect::<Ret<Vec<_>>>()?,
            ),
            None => None,
        };
        Ok(Self {
            schema: req_str(json, "schema")?,
            index: req_raw(json, "index").and_then(parse_usize)?,
            path: req_str(json, "path")?,
            kind: req_raw(json, "kind").and_then(parse_u16)?,
            name: opt_str(json, "name")?,
            scope: req_str(json, "scope")?,
            json: req_str(json, "json")?,
            raw: req_str(json, "raw")?,
            protocol_valid: req_raw(json, "protocol_valid").and_then(parse_bool)?,
            auditability: req_str(json, "auditability")?,
            audit_notes: match field_of(json, "audit_notes")? {
                Some(v) => parse_str_array(v)?,
                None => vec![],
            },
            // Optional on the wire (reviews produced before the fact was
            // added carry no `blob` field): absent means false.
            blob: match field_of(json, "blob")? {
                Some(v) => parse_bool(v)?,
                None => false,
            },
            transfer,
            children,
        })
    }
}

impl TransferDesc {
    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        let payload = match field_of(json, "payload")? {
            Some(v) => PayloadDesc::from_json(v)?,
            None => return sys::errf!("transfer desc missing payload"),
        };
        Ok(Self {
            schema: req_str(json, "schema")?,
            from: opt_str(json, "from")?,
            to: req_str(json, "to")?,
            payload,
        })
    }
}

impl PayloadDesc {
    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        let ty = req_str(json, "type")?;
        match ty.as_str() {
            "hac" => Ok(PayloadDesc::Hac {
                amount: req_str(json, "amount")?,
            }),
            "satoshi" => Ok(PayloadDesc::Satoshi {
                atoms: req_str(json, "atoms")?,
            }),
            "hacd" => Ok(PayloadDesc::Hacd {
                count: req_raw(json, "count").and_then(parse_u32)?,
                names: match field_of(json, "names")? {
                    Some(v) => parse_str_array(v)?,
                    None => vec![],
                },
            }),
            "asset" => Ok(PayloadDesc::Asset {
                serial: req_str(json, "serial")?,
                atoms: req_str(json, "atoms")?,
            }),
            _ => sys::errf!("unknown payload type {}", ty),
        }
    }
}

// ================================ attach ================================

impl SigningRequest {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("id", q(&self.id)),
            kv("purpose", q(&self.purpose)),
            kv("algorithm", q(&self.algorithm)),
            kv("signer_address", q(&self.signer_address)),
            kv("digest", q(&self.digest)),
            kv_opt("body_hash", self.body_hash.as_ref().map(|s| q(s))),
            kv_opt("review_binding", self.review_binding.as_ref().map(|s| q(s))),
            kv_opt(
                "policy_decision",
                self.policy_decision.as_ref().map(|p| p.to_json_string()),
            ),
            kv_opt("origin", self.origin.as_ref().map(|s| q(s))),
            kv_opt("expires_at", self.expires_at.map(|v| v.to_string())),
            kv("request_binding", q(&self.request_binding)),
        ])
    }

    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        Ok(Self {
            schema: req_str(json, "schema")?,
            id: req_str(json, "id")?,
            purpose: req_str(json, "purpose")?,
            algorithm: req_str(json, "algorithm")?,
            signer_address: req_str(json, "signer_address")?,
            digest: req_str(json, "digest")?,
            body_hash: opt_str(json, "body_hash")?,
            review_binding: opt_str(json, "review_binding")?,
            policy_decision: match field_of(json, "policy_decision")? {
                Some(v) => Some(PolicyDecision::from_json(v)?),
                None => None,
            },
            origin: opt_str(json, "origin")?,
            expires_at: opt_num(json, "expires_at")?,
            request_binding: req_str(json, "request_binding")?,
        })
    }
}

impl_sdk_json! {
    SignatureProof {
        schema: str,
        request_id: str,
        request_binding: str,
        public_key: str,
        signature: str,
        algorithm: str,
    } both
}

impl_sdk_json! {
    AttachResult { schema: str, body: str, complete: bool, missing_signers: str_array } to
}

impl_sdk_json! {
    VerifyResult { schema: str, ok: bool, errors: str_array } to
}

impl_sdk_json! {
    SignatureReport {
        schema: str,
        required: str_array,
        present: str_array,
        valid: str_array,
        missing: str_array,
        invalid: str_array,
    } to
}

// ================================ build ================================

impl_sdk_json! {
    BuiltTransaction {
        schema: str,
        tx_type: num,
        timestamp: num,
        main: str,
        fee: str,
        hash: str,
        hash_with_fee: str,
        unsigned_body_hash: str,
        body: str,
    } to
}

// ================================ policy ================================

impl Policy {
    pub(crate) fn to_json_string(&self) -> String {
        obj(vec![
            kv_opt("schema", self.schema.as_ref().map(|s| q(s))),
            kv_opt(
                "deny_kinds",
                self.deny_kinds
                    .as_ref()
                    .map(|v| arr(v.iter().map(|k| k.to_string()).collect())),
            ),
            kv_opt("deny_blob", self.deny_blob.map(|v| v.to_string())),
            kv_opt(
                "max_diamond_names",
                self.max_diamond_names.map(|v| v.to_string()),
            ),
            kv_opt(
                "confirm_auditability",
                self.confirm_auditability
                    .as_ref()
                    .map(|v| arr(v.iter().map(|s| q(s)).collect())),
            ),
        ])
    }

    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        // Policies are authorization data: a mistyped field must error out,
        // not silently become an empty policy that allows everything.
        reject_unknown_keys(
            json,
            &[
                "schema",
                "deny_kinds",
                "deny_blob",
                "max_diamond_names",
                "confirm_auditability",
            ],
        )?;
        Ok(Self {
            schema: opt_str(json, "schema")?,
            deny_kinds: match field_of(json, "deny_kinds")? {
                Some(v) => Some(parse_u16_array(v)?),
                None => None,
            },
            deny_blob: opt_num(json, "deny_blob")?,
            max_diamond_names: opt_num(json, "max_diamond_names")?,
            confirm_auditability: match field_of(json, "confirm_auditability")? {
                Some(v) => Some(parse_str_array(v)?),
                None => None,
            },
        })
    }
}

impl_sdk_json! {
    PolicyDecision {
        schema: str,
        policy_id: str,
        policy_hash: str,
        review_binding: str,
        decision: str,
        findings: str_array,
        policy_binding: str,
    } both
}

// ================================ message ================================

impl_sdk_json! {
    MessagePrepareParams { digest: str, signer_address: str, origin: opt_str, expires_at: opt_num } from
}

impl TransactionJson {
    pub(crate) fn from_json(json: &str) -> Ret<Self> {
        let actions = match field_of(json, "actions")? {
            Some(v) => field::json_split_array(v)?
                .iter()
                .map(|item| ActionDesc::from_json(item))
                .collect::<Ret<Vec<_>>>()?,
            None => {
                return Err(sys::Error::fault("json missing required field actions"));
            }
        };
        let signatures = match field_of(json, "signatures")? {
            Some(v) => field::json_split_array(v)?
                .iter()
                .map(|item| SignatureEntry::from_json(item))
                .collect::<Ret<Vec<_>>>()?,
            None => {
                return Err(sys::Error::fault("json missing required field signatures"));
            }
        };
        Ok(Self {
            schema: req_str(json, "schema")?,
            tx_type: req_raw(json, "tx_type").and_then(|v| v.parse::<u8>().map_err(|_| sys::Error::fault("bad tx_type")))?,
            timestamp: req_raw(json, "timestamp").and_then(parse_u64)?,
            main: req_str(json, "main")?,
            fee: req_str(json, "fee")?,
            gas_max: opt_num(json, "gas_max")?,
            tx_hash: req_str(json, "tx_hash")?,
            hash_with_fee: req_str(json, "hash_with_fee")?,
            unsigned_body_hash: req_str(json, "unsigned_body_hash")?,
            actions,
            signatures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-exact JSON shapes for the macro-generated simple types (locks the
    /// derive output so a future refactor cannot silently reorder or requote).
    #[test]
    fn simple_types_json_shape_is_frozen() {
        let verify = crate::account::VerifyAddressResult {
            ok: true,
            error: None,
            address: Some("1abc".to_owned()),
        };
        assert_eq!(
            verify.to_json_string(),
            r#"{"ok":true,"address":"1abc"}"#
        );

        let ctx = InspectContext {
            current_height: 123_456,
            expected_chain_id: 2,
        };
        assert_eq!(
            ctx.to_json_string(),
            r#"{"current_height":123456,"expected_chain_id":2}"#
        );

        let built = crate::build::BuiltTransaction {
            schema: "s".to_owned(),
            tx_type: 2,
            timestamp: 3,
            main: "m".to_owned(),
            fee: "f".to_owned(),
            hash: "h".to_owned(),
            hash_with_fee: "hf".to_owned(),
            unsigned_body_hash: "ub".to_owned(),
            body: "b".to_owned(),
        };
        assert_eq!(
            built.to_json_string(),
            r#"{"schema":"s","tx_type":2,"timestamp":3,"main":"m","fee":"f","hash":"h","hash_with_fee":"hf","unsigned_body_hash":"ub","body":"b"}"#
        );

        let report = crate::attach::SignatureReport {
            schema: "s".to_owned(),
            required: vec!["a".to_owned()],
            present: vec![],
            valid: vec!["a".to_owned()],
            missing: vec![],
            invalid: vec![],
        };
        assert_eq!(
            report.to_json_string(),
            r#"{"schema":"s","required":["a"],"present":[],"valid":["a"],"missing":[],"invalid":[]}"#
        );
    }

    #[test]
    fn simple_types_from_json_matches_handwritten_semantics() {
        let ctx = InspectContext::from_json(r#"{"current_height":9,"expected_chain_id":1}"#).unwrap();
        assert_eq!(ctx.current_height, 9);
        assert_eq!(ctx.expected_chain_id, 1);

        let proof = SignatureProof::from_json(
            r#"{"schema":"s","request_id":"r","request_binding":"rb","public_key":"pk","signature":"sig","algorithm":"alg"}"#,
        )
        .unwrap();
        assert_eq!(proof.request_id, "r");
        assert_eq!(proof.public_key, "pk");

        let decision = crate::policy::PolicyDecision::from_json(
            r#"{"schema":"s","policy_id":"p","policy_hash":"ph","review_binding":"rb","decision":"allow","findings":["f1"],"policy_binding":"pb"}"#,
        )
        .unwrap();
        assert_eq!(decision.findings, vec!["f1".to_owned()]);

        let params = crate::message::MessagePrepareParams::from_json(
            r#"{"digest":"d","signer_address":"sa","origin":"o","expires_at":5}"#,
        )
        .unwrap();
        assert_eq!(params.expires_at, Some(5));

        // Missing required field and invalid number are rejected like before.
        assert!(InspectContext::from_json(r#"{"expected_chain_id":1}"#).is_err());
        assert!(InspectContext::from_json(r#"{"current_height":"x","expected_chain_id":1}"#).is_err());
    }
}
