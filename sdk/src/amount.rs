//! Protocol amount conversion (Unified SDK 2.0, doc 14 §4.7).
//!
//! Parse and format are thin wrappers around `field::Amount` — the same
//! functions the chain codecs use. The SDK does not re-implement charset,
//! grouping, or unit-range rules; a form `Amount::from` accepts is accepted
//! here, including comma grouping (`12,000:244`). Currency prefixes ("ㄜ",
//! "HAC ") fail because `Amount::from` rejects them.

use field::Amount;

use crate::error::SdkError;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAmount {
    pub value: String,
    pub unit: u8,
    pub is_negative: bool,
}

/// `amount.parse_protocol`: canonicalize one amount string via `Amount::from`.
pub fn parse_protocol(value: &str) -> Result<ParsedAmount, SdkError> {
    let amount = Amount::from(value).map_err(SdkError::from)?;
    Ok(ParsedAmount {
        value: amount.to_fin_string(),
        unit: amount.unit(),
        is_negative: amount.is_negative(),
    })
}

/// `amount.format_protocol`: exact decimal string of the amount at the given
/// unit, via `Amount::to_unit_string`. No float is ever involved, so the
/// result is safe for comparison and arithmetic, not just display (the
/// historical `hac_to_unit` returned a JS float; JS callers that need a
/// number do `Number(value)`). Unit 0 returns the canonical `digits:unit`
/// form — the same fallback `to_unit_string` uses for an unparseable unit.
pub fn format_protocol(value: &str, unit: u8) -> Result<String, SdkError> {
    let amount = Amount::from(value).map_err(SdkError::from)?;
    Ok(amount.to_unit_string(&unit.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::Amount;

    #[test]
    fn canonical_forms_parse() {
        let parsed = parse_protocol("12:244").unwrap();
        assert_eq!(parsed.value, "12:244");
        assert_eq!(parsed.unit, 244);
        let decimal = parse_protocol("0.0012").unwrap();
        assert_eq!(decimal.value, "12:244");
    }

    #[test]
    fn prefixed_and_malformed_forms_are_rejected() {
        for bad in ["ㄜ12:244", "HAC 12:244", "12.0.1", ""] {
            assert!(parse_protocol(bad).is_err(), "must reject amount {bad:?}");
            assert!(
                Amount::from(bad).is_err(),
                "Amount::from must also reject {bad:?}"
            );
        }
    }

    #[test]
    fn grouping_forms_follow_amount_from() {
        for value in ["12,000", "12,000:244"] {
            let parsed = parse_protocol(value).unwrap();
            let amount = Amount::from(value).unwrap();
            assert_eq!(parsed.value, amount.to_fin_string());
            assert_eq!(parsed.unit, amount.unit());
            assert_eq!(parsed.is_negative, amount.is_negative());
        }
    }

    #[test]
    fn formats_to_decimal() {
        assert_eq!(
            format_protocol("12:244", field::UNIT_MEI).unwrap(),
            "0.0012"
        );
        let amount = Amount::from("12:244").unwrap();
        let unit = field::UNIT_MEI + 1;
        assert_eq!(
            format_protocol("12:244", unit).unwrap(),
            amount.to_unit_string(&unit.to_string())
        );
    }
}
