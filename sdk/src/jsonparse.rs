//! Shared hand-written JSON parsing primitives for the wasm boundary (no serde).
//! One implementation serves request parsing (`service`), transaction-spec
//! parsing (`spec_codec`) and boundary-type parsing (`json`, via error mapping).
//! Field-level helpers take a `context` label so each caller keeps its own
//! wording (`request field …` / `JSON field …`).

use crate::error::{SdkError, SdkErrorCode};

pub(crate) fn parse_failed(msg: impl Into<String>) -> SdkError {
    SdkError::new(SdkErrorCode::ParseFailed, msg)
}

/// Split a JSON object, rejecting duplicated keys. Object field lists are
/// short, so duplicate detection is a linear scan over a `Vec`, not a hash set.
pub(crate) fn object_pairs<'a>(
    raw: &'a str,
    context: &str,
) -> Result<Vec<(&'a str, &'a str)>, SdkError> {
    field::json_object_pairs(raw, context).map_err(|e| parse_failed(e.as_str().to_owned()))
}

/// Reject keys outside the allowed list.
pub(crate) fn reject_unknown(
    pairs: &[(&str, &str)],
    allowed: &[&str],
    context: &str,
) -> Result<(), SdkError> {
    field::json_reject_unknown(pairs, allowed, context).map_err(|e| {
        if e.code() == Some(field::JSON_UNKNOWN_FIELD) {
            SdkError::new(SdkErrorCode::UnknownField, e.as_str().to_owned())
        } else {
            parse_failed(e.as_str().to_owned())
        }
    })
}

pub(crate) fn find<'a>(pairs: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

/// Required raw field value.
pub(crate) fn required<'a>(
    pairs: &'a [(&'a str, &'a str)],
    name: &str,
    context: &str,
) -> Result<&'a str, SdkError> {
    find(pairs, name).ok_or_else(|| parse_failed(format!("{context} field {name} missing")))
}

/// Quoted string value.
pub(crate) fn string_value(raw: &str, name: &str, context: &str) -> Result<String, SdkError> {
    field::json_expect_quoted_decoded(raw)
        .map_err(|e| parse_failed(format!("{context} field {name} is not a string: {e}")))
}

/// Numeric value: decimal strings are the boundary convention, bare numbers
/// are accepted too (hand-written JS callers).
pub(crate) fn number_value<T: std::str::FromStr>(
    raw: &str,
    name: &str,
    context: &str,
) -> Result<T, SdkError> {
    let trimmed = raw.trim();
    let text = if trimmed.starts_with('"') {
        string_value(trimmed, name, context)?
    } else {
        trimmed.to_owned()
    };
    text.parse()
        .map_err(|_| parse_failed(format!("{context} field {name} is not a number")))
}

// ---- field combinators over an object pair list ----

pub(crate) fn required_string(
    pairs: &[(&str, &str)],
    name: &str,
    context: &str,
) -> Result<String, SdkError> {
    string_value(required(pairs, name, context)?, name, context)
}

pub(crate) fn optional_string(
    pairs: &[(&str, &str)],
    name: &str,
    context: &str,
) -> Result<Option<String>, SdkError> {
    find(pairs, name)
        .map(|raw| string_value(raw, name, context))
        .transpose()
}

pub(crate) fn required_number<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    name: &str,
    context: &str,
) -> Result<T, SdkError> {
    number_value(required(pairs, name, context)?, name, context)
}

pub(crate) fn optional_number<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    name: &str,
    context: &str,
) -> Result<Option<T>, SdkError> {
    find(pairs, name)
        .map(|raw| number_value(raw, name, context))
        .transpose()
}
