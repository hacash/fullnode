//! JSON view of SDK domain types for the wasm boundary (hand-written `field::json_*`
//! engine; serde_json test-oracle only). Amounts/addresses/hex and numeric fields
//! are strings on the boundary so JS `JSON.parse` cannot lose precision.

use std::collections::HashSet;

use sys::errf;

use crate::account::{AddressFromPublicKeyResult, VerifyAddressResult};
use crate::amount::ParsedAmount;
use crate::attach::{AttachResult, SignatureProof, SignatureReport, SigningRequest, VerifyResult};
use crate::audit::{ActionDesc, PayloadDesc, TransferDesc};
use crate::build::BuiltTransaction;
use crate::inspect::{HeightRangeDesc, InspectContext, Review, SignatureEntry, TransactionJson};
use crate::message::{MessagePrepareParams, MessageVerifyResult};
use crate::policy::{Policy, PolicyDecision};
use crate::profile::{
    AbiVersion, Capabilities, CodecProfile, FeatureItem, LimitsProfile, ProtocolParamsProfile,
};

// ================================ error-detail JSON builders ================================
// Retained only for `SdkError.detail` and native-side tests, not the boundary serializer.

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
    let filtered: Vec<&str> = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|s| s.as_str())
        .collect();
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

/// Option field: None returns an empty string (filtered out by `obj`).
pub(crate) fn kv_opt(key: &str, value: Option<String>) -> String {
    value.map(|v| kv(key, v)).unwrap_or_default()
}

/// Quoted decimal string of a numeric value (boundary convention: numbers
/// travel as strings so JS never loses precision).
fn qnum(v: impl std::fmt::Display) -> String {
    format!("\"{v}\"")
}

// ================================ JSON serialization traits ================================

/// Canonical JSON view of an SDK boundary object: emits every declared field
/// in declaration order (skip None), the deterministic input of binding hashes.
pub(crate) trait SdkJsonTo {
    fn to_json_string(&self) -> String;
}

/// JSON parsing side of a boundary object: parses with the hand-written engine,
/// rejecting missing required fields, duplicated keys and unknown fields.
pub(crate) trait SdkJsonFrom: Sized {
    fn from_json_str(json: &str) -> sys::Ret<Self>;
}

// ================================ parse-side helpers ================================

/// Split a JSON object for one boundary type, rejecting duplicated or unknown
/// keys; the allowed list comes from the same layout declaration as the reader.
fn sdk_json_pairs<'a>(json: &'a str, allowed: &[&str]) -> sys::Ret<Vec<(&'a str, &'a str)>> {
    let pairs = field::json_split_object(json)
        .map_err(|e| sys::Error::fault(format!("json object parse failed: {e}")))?;
    let mut seen = HashSet::new();
    for (key, _) in &pairs {
        if !seen.insert(*key) {
            return errf!("json field {key} is duplicated");
        }
        if !allowed.iter().any(|name| *name == *key) {
            return errf!("json field {key} is unknown");
        }
    }
    Ok(pairs)
}

fn jfind<'a>(pairs: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

fn ensure_allowed_fields(pairs: &[(&str, &str)], allowed: &[&str]) -> sys::Ret<()> {
    for (key, _) in pairs {
        if !allowed.iter().any(|name| *name == *key) {
            return errf!("json field {key} is unknown");
        }
    }
    Ok(())
}

fn jneed<'a>(pairs: &'a [(&'a str, &'a str)], name: &str) -> sys::Ret<&'a str> {
    jfind(pairs, name).ok_or_else(|| sys::Error::fault(format!("json field {name} missing")))
}

fn jstr(pairs: &[(&str, &str)], name: &str) -> sys::Ret<String> {
    field::json_expect_quoted_decoded(jneed(pairs, name)?)
        .map_err(|e| sys::Error::fault(format!("json field {name} is not a string: {e}")))
}

fn jopt_str(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<String>> {
    match jfind(pairs, name) {
        Some(raw) => field::json_expect_quoted_decoded(raw)
            .map(Some)
            .map_err(|e| sys::Error::fault(format!("json field {name} is not a string: {e}"))),
        None => Ok(None),
    }
}

/// Numeric value: the boundary convention is decimal strings, but bare
/// numbers are accepted too (hand-written JS callers).
fn jnum<T: std::str::FromStr>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<T> {
    let raw = jneed(pairs, name)?.trim();
    let text = if raw.starts_with('"') {
        field::json_expect_quoted_decoded(raw)
            .map_err(|e| sys::Error::fault(format!("json field {name} is not a number: {e}")))?
    } else {
        raw.to_owned()
    };
    text.parse()
        .map_err(|_| sys::Error::fault(format!("json field {name} is not a number")))
}

fn jopt_num<T: std::str::FromStr>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<T>> {
    match jfind(pairs, name) {
        Some(_) => jnum(pairs, name).map(Some),
        None => Ok(None),
    }
}

fn jbool(pairs: &[(&str, &str)], name: &str) -> sys::Ret<bool> {
    let raw = jneed(pairs, name)?.trim();
    let text = if raw.starts_with('"') {
        field::json_expect_quoted_decoded(raw)
            .map_err(|e| sys::Error::fault(format!("json field {name} is not a bool: {e}")))?
    } else {
        raw.to_owned()
    };
    match text.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => errf!("json field {name} is not a bool"),
    }
}

fn jopt_bool(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<bool>> {
    match jfind(pairs, name) {
        Some(_) => jbool(pairs, name).map(Some),
        None => Ok(None),
    }
}

fn jarr_str(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Vec<String>> {
    let raw = jneed(pairs, name)?;
    let items = field::json_split_array(raw)
        .map_err(|e| sys::Error::fault(format!("json field {name} is not a string array: {e}")))?;
    items
        .iter()
        .map(|item| {
            field::json_expect_quoted_decoded(item).map_err(|e| {
                sys::Error::fault(format!("json field {name} item is not a string: {e}"))
            })
        })
        .collect()
}

fn jopt_arr_str(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<Vec<String>>> {
    match jfind(pairs, name) {
        Some(_) => jarr_str(pairs, name).map(Some),
        None => Ok(None),
    }
}

fn jarr_num<T: std::str::FromStr>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Vec<T>> {
    let raw = jneed(pairs, name)?;
    let items = field::json_split_array(raw)
        .map_err(|e| sys::Error::fault(format!("json field {name} is not an array: {e}")))?;
    items
        .iter()
        .map(|item| {
            let text = item.trim();
            let value = if text.starts_with('"') {
                field::json_expect_quoted_decoded(text).map_err(|e| {
                    sys::Error::fault(format!("json field {name} item is not a number: {e}"))
                })?
            } else {
                text.to_owned()
            };
            value
                .parse()
                .map_err(|_| sys::Error::fault(format!("json field {name} item is not a number")))
        })
        .collect()
}

fn jopt_arr_num<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    name: &str,
) -> sys::Ret<Option<Vec<T>>> {
    match jfind(pairs, name) {
        Some(_) => jarr_num(pairs, name).map(Some),
        None => Ok(None),
    }
}

fn jobj<T: SdkJsonFrom>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<T> {
    <T as SdkJsonFrom>::from_json_str(jneed(pairs, name)?)
}

fn jopt_obj<T: SdkJsonFrom>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<T>> {
    match jfind(pairs, name) {
        Some(raw) => <T as SdkJsonFrom>::from_json_str(raw).map(Some),
        None => Ok(None),
    }
}

fn jarr_obj<T: SdkJsonFrom>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Vec<T>> {
    let raw = jneed(pairs, name)?;
    let items = field::json_split_array(raw)
        .map_err(|e| sys::Error::fault(format!("json field {name} is not an array: {e}")))?;
    items
        .iter()
        .map(|item| <T as SdkJsonFrom>::from_json_str(item))
        .collect()
}

fn jopt_arr_obj<T: SdkJsonFrom>(pairs: &[(&str, &str)], name: &str) -> sys::Ret<Option<Vec<T>>> {
    match jfind(pairs, name) {
        Some(_) => jarr_obj(pairs, name).map(Some),
        None => Ok(None),
    }
}

// ================================ JSON derive macros ================================

macro_rules! sdk_json_kv {
    ($self:ident, $field:ident, str) => {
        kv(stringify!($field), field::json_escape(&$self.$field))
    };
    ($self:ident, $field:ident, opt_str) => {
        kv_opt(
            stringify!($field),
            $self.$field.as_deref().map(field::json_escape),
        )
    };
    ($self:ident, $field:ident, str_def) => {
        kv(stringify!($field), field::json_escape(&$self.$field))
    };
    ($self:ident, $field:ident, u64) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, opt_u64) => {
        kv_opt(stringify!($field), $self.$field.map(qnum))
    };
    ($self:ident, $field:ident, u32) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, opt_u32) => {
        kv_opt(stringify!($field), $self.$field.map(qnum))
    };
    ($self:ident, $field:ident, u8) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, opt_u8) => {
        kv_opt(stringify!($field), $self.$field.map(qnum))
    };
    ($self:ident, $field:ident, bool) => {
        kv(stringify!($field), $self.$field.to_string())
    };
    ($self:ident, $field:ident, opt_bool) => {
        kv_opt(stringify!($field), $self.$field.map(|v| v.to_string()))
    };
    ($self:ident, $field:ident, bool_def_true) => {
        kv(stringify!($field), $self.$field.to_string())
    };
    ($self:ident, $field:ident, bool_def_false) => {
        kv(stringify!($field), $self.$field.to_string())
    };
    ($self:ident, $field:ident, str_arr) => {
        kv(
            stringify!($field),
            arr($self.$field.iter().map(|s| field::json_escape(s)).collect()),
        )
    };
    ($self:ident, $field:ident, opt_str_arr) => {
        kv_opt(
            stringify!($field),
            $self
                .$field
                .as_ref()
                .map(|items| arr(items.iter().map(|s| field::json_escape(s)).collect())),
        )
    };
    ($self:ident, $field:ident, u64_arr) => {
        kv(
            stringify!($field),
            arr($self.$field.iter().map(|v| qnum(v)).collect()),
        )
    };
    ($self:ident, $field:ident, opt_u32_arr) => {
        kv_opt(
            stringify!($field),
            $self
                .$field
                .as_ref()
                .map(|items| arr(items.iter().map(|v| qnum(v)).collect())),
        )
    };
    ($self:ident, $field:ident, u16_arr) => {
        kv(
            stringify!($field),
            arr($self.$field.iter().map(|v| qnum(v)).collect()),
        )
    };
    ($self:ident, $field:ident, opt_u16_arr) => {
        kv_opt(
            stringify!($field),
            $self
                .$field
                .as_ref()
                .map(|items| arr(items.iter().map(|v| qnum(v)).collect())),
        )
    };
    ($self:ident, $field:ident, usize) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, usize_def) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, u16) => {
        kv(stringify!($field), qnum($self.$field))
    };
    ($self:ident, $field:ident, obj $t:ty) => {
        kv(
            stringify!($field),
            <$t as SdkJsonTo>::to_json_string(&$self.$field),
        )
    };
    ($self:ident, $field:ident, opt_obj $t:ty) => {
        kv_opt(
            stringify!($field),
            $self
                .$field
                .as_ref()
                .map(|v| <$t as SdkJsonTo>::to_json_string(v)),
        )
    };
    ($self:ident, $field:ident, obj_arr $t:ty) => {
        kv(
            stringify!($field),
            arr($self
                .$field
                .iter()
                .map(|x| <$t as SdkJsonTo>::to_json_string(x))
                .collect()),
        )
    };
    ($self:ident, $field:ident, opt_obj_arr $t:ty) => {
        kv_opt(
            stringify!($field),
            $self.$field.as_ref().map(|items| {
                arr(items
                    .iter()
                    .map(|x| <$t as SdkJsonTo>::to_json_string(x))
                    .collect())
            }),
        )
    };
}

macro_rules! sdk_json_expr {
    ($pairs:ident, $field:ident, str) => {
        jstr(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_str) => {
        jopt_str(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, str_def) => {
        jopt_str(&$pairs, stringify!($field))?.unwrap_or_default()
    };
    ($pairs:ident, $field:ident, u64) => {
        jnum(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_u64) => {
        jopt_num(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, u32) => {
        jnum(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_u32) => {
        jopt_num(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, u8) => {
        jnum(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_u8) => {
        jopt_num(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, bool) => {
        jbool(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_bool) => {
        jopt_bool(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, bool_def_true) => {
        jopt_bool(&$pairs, stringify!($field))?.unwrap_or(true)
    };
    ($pairs:ident, $field:ident, bool_def_false) => {
        jopt_bool(&$pairs, stringify!($field))?.unwrap_or(false)
    };
    ($pairs:ident, $field:ident, str_arr) => {
        jopt_arr_str(&$pairs, stringify!($field))?.unwrap_or_default()
    };
    ($pairs:ident, $field:ident, opt_str_arr) => {
        jopt_arr_str(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, u64_arr) => {
        jopt_arr_num(&$pairs, stringify!($field))?.unwrap_or_default()
    };
    ($pairs:ident, $field:ident, opt_u32_arr) => {
        jopt_arr_num(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, u16_arr) => {
        jopt_arr_num(&$pairs, stringify!($field))?.unwrap_or_default()
    };
    ($pairs:ident, $field:ident, opt_u16_arr) => {
        jopt_arr_num(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, usize) => {
        jnum(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, usize_def) => {
        jopt_num(&$pairs, stringify!($field))?.unwrap_or_default()
    };
    ($pairs:ident, $field:ident, u16) => {
        jnum(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, obj $t:ty) => {
        jobj::<$t>(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_obj $t:ty) => {
        jopt_obj::<$t>(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, obj_arr $t:ty) => {
        jarr_obj::<$t>(&$pairs, stringify!($field))?
    };
    ($pairs:ident, $field:ident, opt_obj_arr $t:ty) => {
        jopt_arr_obj::<$t>(&$pairs, stringify!($field))?
    };
}

/// Generate `to_json_string`/`from_json_str` for a boundary view type
/// (modes: `to`, `from`, `both`).
macro_rules! impl_sdk_json {
    ($ty:ty { $($field:ident : $kind:ident $($arg:ty)?),+ $(,)? } both) => {
        impl SdkJsonTo for $ty {
            fn to_json_string(&self) -> String {
                obj(vec![$(sdk_json_kv!(self, $field, $kind $($arg)?)),+])
            }
        }
        impl SdkJsonFrom for $ty {
            fn from_json_str(json: &str) -> sys::Ret<Self> {
                let pairs = sdk_json_pairs(json, &[$(stringify!($field)),+])?;
                $(let $field = sdk_json_expr!(pairs, $field, $kind $($arg)?);)+
                Ok(Self { $($field),+ })
            }
        }
    };
    ($ty:ty { $($field:ident : $kind:ident $($arg:ty)?),+ $(,)? } to) => {
        impl SdkJsonTo for $ty {
            fn to_json_string(&self) -> String {
                obj(vec![$(sdk_json_kv!(self, $field, $kind $($arg)?)),+])
            }
        }
    };
    ($ty:ty { $($field:ident : $kind:ident $($arg:ty)?),+ $(,)? } from) => {
        impl SdkJsonFrom for $ty {
            fn from_json_str(json: &str) -> sys::Ret<Self> {
                let pairs = sdk_json_pairs(json, &[$(stringify!($field)),+])?;
                $(let $field = sdk_json_expr!(pairs, $field, $kind $($arg)?);)+
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
    AddressFromPublicKeyResult { address: str, version: u8 } to
}
impl_sdk_json! {
    ParsedAmount { value: str, unit: u8, is_negative: bool } to
}
impl_sdk_json! {
    MessageVerifyResult { ok: bool, address: opt_str, error: opt_str } to
}

// ================================ profile ================================

impl ProtocolParamsProfile {
    pub(crate) fn to_json_string(&self) -> String {
        // Reductions travel flattened (activation, next) alternating.
        let flat: Vec<String> = self
            .fee_purity_reductions
            .iter()
            .flat_map(|(activation, next)| [qnum(activation), qnum(next)])
            .collect();
        obj(vec![
            kv("ast_tree_depth_max", qnum(self.ast_tree_depth_max as u64)),
            kv("max_type3_signers", qnum(self.max_type3_signers as u64)),
            kv("fee_purity_floor", qnum(self.fee_purity_floor)),
            kv("diamond_form_flag", qnum(self.diamond_form_flag)),
            kv("fee_purity_reductions", arr(flat)),
        ])
    }
}

impl SdkJsonTo for ProtocolParamsProfile {
    fn to_json_string(&self) -> String {
        // Flattened (activation, next) pairs; see the inherent method.
        ProtocolParamsProfile::to_json_string(self)
    }
}

impl_sdk_json! {
    LimitsProfile { max_tx_size: usize, tx_actions_max: usize, hacd_wire_max: usize } to
}

impl_sdk_json! {
    CodecProfile {
        schema: str,
        sdk_version: str,
        fullnode_commit: str,
        params_version: u32,
        protocol_params: obj ProtocolParamsProfile,
        limits: obj LimitsProfile,
        registered_kinds: u16_arr,
        registered_tx_types: u16_arr,
        schema_hash: str,
        registry_hash: str,
        profile_hash: str,
    } to
}

impl_sdk_json! {
    AbiVersion { major: u32, minor: u32 } to
}
impl_sdk_json! {
    FeatureItem { id: str, version: u32 } to
}

impl_sdk_json! {
    Capabilities {
        schema: str,
        package_version: str,
        abi: obj AbiVersion,
        codec_profile_hash: str,
        features: obj_arr FeatureItem,
    } to
}

/// The `system.sdk_version` response body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SdkVersion {
    pub schema: String,
    pub package_version: String,
    pub abi: AbiVersion,
}

impl_sdk_json! {
    SdkVersion { schema: str, package_version: str, abi: obj AbiVersion } to
}

/// The `amount.format_protocol` response body (`{value}`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AmountFormatResult {
    pub value: String,
}

impl_sdk_json! {
    AmountFormatResult { value: str } to
}

// ================================ audit ================================

impl PayloadDesc {
    pub(crate) fn to_json_string(&self) -> String {
        match self {
            PayloadDesc::Hac { amount } => obj(vec![kv("type", q("hac")), kv("amount", q(amount))]),
            PayloadDesc::Satoshi { atoms } => {
                obj(vec![kv("type", q("satoshi")), kv("atoms", q(atoms))])
            }
            PayloadDesc::Hacd { count, names } => obj(vec![
                kv("type", q("hacd")),
                kv("count", qnum(count)),
                kv("names", arr(names.iter().map(|n| q(n)).collect())),
            ]),
            PayloadDesc::Asset { serial, atoms } => obj(vec![
                kv("type", q("asset")),
                kv("serial", q(serial)),
                kv("atoms", q(atoms)),
            ]),
        }
    }

    pub(crate) fn from_json_str(json: &str) -> sys::Ret<Self> {
        let pairs = sdk_json_pairs(
            json,
            &["type", "amount", "atoms", "count", "names", "serial"],
        )?;
        match jstr(&pairs, "type")?.as_str() {
            "hac" => {
                ensure_allowed_fields(&pairs, &["type", "amount"])?;
                Ok(PayloadDesc::Hac {
                    amount: jstr(&pairs, "amount")?,
                })
            }
            "satoshi" => {
                ensure_allowed_fields(&pairs, &["type", "atoms"])?;
                Ok(PayloadDesc::Satoshi {
                    atoms: jstr(&pairs, "atoms")?,
                })
            }
            "hacd" => {
                ensure_allowed_fields(&pairs, &["type", "count", "names"])?;
                Ok(PayloadDesc::Hacd {
                    count: jnum(&pairs, "count")?,
                    names: jopt_arr_str(&pairs, "names")?.unwrap_or_default(),
                })
            }
            "asset" => {
                ensure_allowed_fields(&pairs, &["type", "serial", "atoms"])?;
                Ok(PayloadDesc::Asset {
                    serial: jstr(&pairs, "serial")?,
                    atoms: jstr(&pairs, "atoms")?,
                })
            }
            other => errf!("unknown payload type {}", other),
        }
    }
}

impl SdkJsonTo for PayloadDesc {
    fn to_json_string(&self) -> String {
        PayloadDesc::to_json_string(self)
    }
}

impl SdkJsonFrom for PayloadDesc {
    fn from_json_str(json: &str) -> sys::Ret<Self> {
        PayloadDesc::from_json_str(json)
    }
}

impl_sdk_json! {
    TransferDesc {
        schema: str_def,
        from: opt_str,
        to: str,
        payload: obj PayloadDesc,
    } both
}

impl_sdk_json! {
    ActionDesc {
        schema: str_def,
        index: usize_def,
        path: str_def,
        kind: u16,
        name: opt_str,
        scope: str_def,
        raw: str,
        protocol_valid: bool_def_true,
        auditability: str_def,
        audit_notes: str_arr,
        blob: bool_def_false,
        transfer: opt_obj TransferDesc,
        children: opt_obj_arr ActionDesc,
    } both
}

// ================================ inspect ================================

impl_sdk_json! {
    InspectContext {
        current_height: u64,
        expected_chain_id: u32,
        consensus_flags: opt_u64,
    } both
}

impl_sdk_json! {
    HeightRangeDesc { start: u64, end: u64 } both
}

impl_sdk_json! {
    Review {
        schema: str,
        codec_profile_hash: str,
        tx_type: u8,
        timestamp: u64,
        main: str,
        fee: str,
        gas_max: opt_u8,
        tx_hash: str,
        hash_with_fee: str,
        unsigned_body_hash: str,
        review_binding: str,
        signer_address: opt_str,
        inspect_context: opt_obj InspectContext,
        expired_height: opt_bool,
        wrong_chain: opt_bool,
        protocol_valid: bool,
        signability: str,
        auditability: str,
        requires_user_confirmation: bool,
        limits_violations: str_arr,
        topology_violations: str_arr,
        guard_violations: str_arr,
        schedule_violations: str_arr,
        required_signers: str_arr,
        present_signers: str_arr,
        valid_signers: str_arr,
        missing_signers: str_arr,
        invalid_signers: str_arr,
        signature_errors: str_arr,
        chain_ids_allowed: opt_u32_arr,
        valid_height_range: opt_obj HeightRangeDesc,
        fee_purity: opt_u64,
        fee_purity_ok: opt_bool,
        actions: obj_arr ActionDesc,
        asset_serials: u64_arr,
    } both
}

impl_sdk_json! {
    SignatureEntry { public_key: str, signature: str } both
}

impl_sdk_json! {
    TransactionJson {
        schema: str,
        tx_type: u8,
        timestamp: u64,
        main: str,
        fee: str,
        gas_max: opt_u8,
        tx_hash: str,
        hash_with_fee: str,
        unsigned_body_hash: str,
        actions: obj_arr ActionDesc,
        signatures: obj_arr SignatureEntry,
    } both
}

// ================================ attach ================================

impl_sdk_json! {
    SigningRequest {
        schema: str,
        id: str,
        purpose: str,
        algorithm: str,
        signer_address: str,
        digest: str,
        body_hash: opt_str,
        review_binding: opt_str,
        policy_decision: opt_obj PolicyDecision,
        origin: opt_str,
        expires_at: opt_u64,
        request_binding: str,
    } both
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
    AttachResult {
        schema: str,
        body: str,
        complete: bool,
        present_signers: str_arr,
        valid_signers: str_arr,
        missing_signers: str_arr,
        invalid_signers: str_arr,
        signature_errors: str_arr,
    } to
}

impl_sdk_json! {
    VerifyResult { schema: str, ok: bool, errors: str_arr } to
}

impl_sdk_json! {
    SignatureReport {
        schema: str,
        required: str_arr,
        present: str_arr,
        valid: str_arr,
        missing: str_arr,
        invalid: str_arr,
    } to
}

// ================================ build ================================

impl_sdk_json! {
    BuiltTransaction {
        schema: str,
        tx_type: u8,
        timestamp: u64,
        main: str,
        fee: str,
        hash: str,
        hash_with_fee: str,
        unsigned_body_hash: str,
        body: str,
    } to
}

// ================================ policy ================================

impl_sdk_json! {
    Policy {
        schema: opt_str,
        deny_kinds: opt_u16_arr,
        deny_blob: opt_bool,
        max_diamond_names: opt_u32,
        confirm_auditability: opt_str_arr,
    } both
}

impl_sdk_json! {
    PolicyDecision {
        schema: str,
        policy_id: str,
        policy_hash: str,
        review_binding: str,
        decision: str,
        findings: str_arr,
        policy_binding: str,
    } both
}

// ================================ message ================================

impl_sdk_json! {
    MessagePrepareParams { digest: str, signer_address: str, origin: opt_str, expires_at: opt_u64 } from
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_desc_json_round_trips() {
        let cases = [
            PayloadDesc::Hac {
                amount: "1".to_owned(),
            },
            PayloadDesc::Satoshi {
                atoms: "2".to_owned(),
            },
            PayloadDesc::Hacd {
                count: 3,
                names: vec!["AAABBB".to_owned()],
            },
            PayloadDesc::Asset {
                serial: "4".to_owned(),
                atoms: "5".to_owned(),
            },
        ];
        for payload in cases {
            let json = payload.to_json_string();
            let pairs = field::json_split_object(&json).expect("payload json parses");
            let names: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
            assert_eq!(names[0], "type", "type tag must be the first field");
            let decoded = PayloadDesc::from_json_str(&json).expect("payload json round-trips");
            assert_eq!(decoded, payload, "payload json round-trip");
        }
    }

    /// JSON round-trip of the macro-generated simple types (locks the field
    /// set so a future refactor cannot silently drop or rename a field).
    #[test]
    fn simple_types_json_round_trip() {
        let verify = crate::account::VerifyAddressResult {
            ok: true,
            error: None,
            address: Some("1abc".to_owned()),
        };
        let json = verify.to_json_string();
        let pairs = field::json_split_object(&json).unwrap();
        assert_eq!(jbool(&pairs, "ok").unwrap(), true);
        assert_eq!(jstr(&pairs, "address").unwrap(), "1abc");

        let ctx = InspectContext {
            current_height: 123_456,
            expected_chain_id: 2,
            consensus_flags: None,
        };
        let decoded = InspectContext::from_json_str(&ctx.to_json_string()).unwrap();
        assert_eq!(decoded, ctx);
        // Numeric fields travel as decimal strings on the boundary.
        assert!(ctx
            .to_json_string()
            .contains("\"current_height\":\"123456\""));

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
        let json = built.to_json_string();
        let pairs = field::json_split_object(&json).unwrap();
        assert_eq!(jnum::<u8>(&pairs, "tx_type").unwrap(), 2);
        assert_eq!(jnum::<u64>(&pairs, "timestamp").unwrap(), 3);
        assert_eq!(jstr(&pairs, "main").unwrap(), "m");

        let report = crate::attach::SignatureReport {
            schema: "s".to_owned(),
            required: vec!["a".to_owned()],
            present: vec![],
            valid: vec!["a".to_owned()],
            missing: vec![],
            invalid: vec![],
        };
        let json = report.to_json_string();
        let pairs = field::json_split_object(&json).unwrap();
        assert_eq!(jarr_str(&pairs, "required").unwrap(), vec!["a"]);
        assert_eq!(jarr_str(&pairs, "present").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn request_objects_round_trip() {
        let ctx = InspectContext {
            current_height: 9,
            expected_chain_id: 1,
            consensus_flags: None,
        };
        let decoded = InspectContext::from_json_str(&ctx.to_json_string()).unwrap();
        assert_eq!(decoded.current_height, 9);
        assert_eq!(decoded.expected_chain_id, 1);

        let proof = crate::attach::SignatureProof {
            schema: "s".to_owned(),
            request_id: "r".to_owned(),
            request_binding: "rb".to_owned(),
            public_key: "pk".to_owned(),
            signature: "sig".to_owned(),
            algorithm: "alg".to_owned(),
        };
        let decoded =
            crate::attach::SignatureProof::from_json_str(&proof.to_json_string()).unwrap();
        assert_eq!(decoded.request_id, "r");
        assert_eq!(decoded.public_key, "pk");

        let decision = crate::policy::PolicyDecision {
            schema: "s".to_owned(),
            policy_id: "p".to_owned(),
            policy_hash: "ph".to_owned(),
            review_binding: "rb".to_owned(),
            decision: "allow".to_owned(),
            findings: vec!["f1".to_owned()],
            policy_binding: "pb".to_owned(),
        };
        let decoded =
            crate::policy::PolicyDecision::from_json_str(&decision.to_json_string()).unwrap();
        assert_eq!(decoded.findings, vec!["f1".to_owned()]);

        let params = crate::message::MessagePrepareParams {
            digest: "d".to_owned(),
            signer_address: "sa".to_owned(),
            origin: Some("o".to_owned()),
            expires_at: Some(5),
        };
        // Bare numbers accepted on read (hand-written JS callers).
        let decoded = crate::message::MessagePrepareParams::from_json_str(
            r#"{"digest":"d","signer_address":"sa","origin":"o","expires_at":5}"#,
        )
        .unwrap();
        assert_eq!(decoded, params);

        // Missing required field is rejected.
        assert!(InspectContext::from_json_str("{}").is_err());
        // Duplicated keys are rejected.
        assert!(InspectContext::from_json_str(
            r#"{"current_height":"1","current_height":"2","expected_chain_id":"0"}"#
        )
        .is_err());
    }

    #[test]
    fn complex_json_rejects_unknown_fields() {
        let error = crate::attach::SignatureProof::from_json_str(
            r#"{"schema":"s","request_id":"r","request_binding":"b","public_key":"p","signature":"s","algorithm":"a","typo":1}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown"));

        let error =
            PayloadDesc::from_json_str(r#"{"type":"hac","amount":"1","atoms":"2"}"#).unwrap_err();
        assert!(error.to_string().contains("unknown"));
    }

    #[test]
    fn narrow_integer_fields_reject_truncating_values() {
        let error = Policy::from_json_str(r#"{"deny_kinds":["65536"]}"#).unwrap_err();
        assert!(error.to_string().contains("not a number"), "{error}");
    }
}
