//! Strict protocol amount conversion (Unified SDK 2.0, doc 14 §4.7).
//!
//! `parse_protocol` accepts only canonical machine forms (decimal or
//! `digits:unit`); currency prefixes ("ㄜ", "HAC "), locale separators and
//! floats are UI-adapter concerns and never reach the canonical parser.

use field::Amount;
use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedAmount {
    pub value: String,
    pub unit: u8,
    pub is_negative: bool,
}

/// `amount.parse_protocol`: validate and canonicalize one amount string.
pub fn parse_protocol(value: &str) -> Result<ParsedAmount, SdkError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SdkError::new(SdkErrorCode::ParseFailed, "amount is empty"));
    }
    let body = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if body.contains(',') {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "thousand separators are not accepted by the machine parser",
        ));
    }
    if !body
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b':' || byte == b'.')
    {
        return Err(SdkError::with_detail(
            SdkErrorCode::ParseFailed,
            "amount contains unsupported characters; use canonical digits only",
            serde_json::json!({ "actual": value }),
        ));
    }
    let amount = Amount::from(trimmed).map_err(|error| SdkError::from(error))?;
    Ok(ParsedAmount {
        value: amount.to_fin_string(),
        unit: amount.unit(),
        is_negative: amount.is_negative(),
    })
}

/// `amount.format_protocol`: exact decimal string of the amount at the given
/// unit. No float is ever involved, so the result is safe for comparison and
/// arithmetic, not just display (the historical `hac_to_unit` returned a
/// JS float; JS callers that need a number do `Number(value)`). Unit 0
/// returns the canonical `digits:unit` form.
pub fn format_protocol(value: &str, unit: u8) -> Result<String, SdkError> {
    if unit > field::UNIT_MEI {
        return Err(SdkError::with_detail(
            SdkErrorCode::ParseFailed,
            format!("unit {unit} out of range, max {}", field::UNIT_MEI),
            serde_json::json!({ "expected": field::UNIT_MEI }),
        ));
    }
    let amount = Amount::from(value).map_err(|error| SdkError::from(error))?;
    Ok(amount.to_unit_string(&unit.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_forms_parse() {
        let parsed = parse_protocol("12:244").unwrap();
        assert_eq!(parsed.value, "12:244");
        assert_eq!(parsed.unit, 244);
        let decimal = parse_protocol("0.0012").unwrap();
        assert_eq!(decimal.value, "12:244");
    }

    #[test]
    fn prefixed_and_locale_forms_are_rejected() {
        for bad in ["ㄜ12:244", "HAC 12:244", "12,000", "12,000:244", "12.0.1"] {
            assert!(
                parse_protocol(bad).is_err(),
                "must reject non-canonical amount {bad:?}"
            );
        }
    }

    #[test]
    fn formats_to_decimal() {
        assert_eq!(
            format_protocol("12:244", field::UNIT_MEI).unwrap(),
            "0.0012"
        );
        assert!(format_protocol("12:244", field::UNIT_MEI + 1).is_err());
    }
}
