//! Protocol amount conversion (Unified SDK 2.0, doc 14 §4.7). Parse/format are
//! thin wrappers over `field::Amount`, so any form it accepts (incl. comma grouping) is accepted here.

use field::Amount;

use crate::error::SdkError;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAmount {
    pub value: String,
    pub unit: u8,
    pub is_negative: bool,
}

/// `amount.parse`: canonicalize one amount string via `Amount::from`.
pub fn parse(value: &str) -> Result<ParsedAmount, SdkError> {
    let amount = Amount::from(value).map_err(SdkError::from)?;
    Ok(ParsedAmount {
        value: amount.to_fin_string(),
        unit: amount.unit(),
        is_negative: amount.is_negative(),
    })
}

/// `amount.format`: exact decimal string at the given unit via
/// `Amount::to_unit_string` — no float, so it is safe for comparison/arithmetic.
pub fn format(value: &str, unit: u8) -> Result<String, SdkError> {
    let amount = Amount::from(value).map_err(SdkError::from)?;
    Ok(amount.to_unit_string(&unit.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::Amount;

    #[test]
    fn canonical_forms_parse() {
        let parsed = parse("12:244").unwrap();
        assert_eq!(parsed.value, "12:244");
        assert_eq!(parsed.unit, 244);
        let decimal = parse("0.0012").unwrap();
        assert_eq!(decimal.value, "12:244");
    }

    #[test]
    fn prefixed_and_malformed_forms_are_rejected() {
        for bad in ["ㄜ12:244", "HAC 12:244", "12.0.1", ""] {
            assert!(parse(bad).is_err(), "must reject amount {bad:?}");
            assert!(
                Amount::from(bad).is_err(),
                "Amount::from must also reject {bad:?}"
            );
        }
    }

    #[test]
    fn grouping_forms_follow_amount_from() {
        for value in ["12,000", "12,000:244"] {
            let parsed = parse(value).unwrap();
            let amount = Amount::from(value).unwrap();
            assert_eq!(parsed.value, amount.to_fin_string());
            assert_eq!(parsed.unit, amount.unit());
            assert_eq!(parsed.is_negative, amount.is_negative());
        }
    }

    #[test]
    fn formats_to_decimal() {
        assert_eq!(
            format("12:244", field::UNIT_MEI).unwrap(),
            "0.0012"
        );
        let amount = Amount::from("12:244").unwrap();
        let unit = field::UNIT_MEI + 1;
        assert_eq!(
            format("12:244", unit).unwrap(),
            amount.to_unit_string(&unit.to_string())
        );
    }
}
