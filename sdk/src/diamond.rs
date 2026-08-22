//! `diamond.lookup`: offline diamond identity checks (Unified SDK 2.0, doc 14 §4.7).
//! The name→serial mapping lives in chain state (mining order), so this operation
//! validates what IS derivable offline: name charset/format and serial range.
//! Deposit/withdrawal flows use it to reject malformed user input before a query.

use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::schema::SCHEMA_DIAMOND_LOOKUP;

/// `diamond.lookup` response (`hacash.sdk/diamond-lookup@1`). Exactly one of
/// `name` / `serial` is requested; the response echoes the validated branch.
#[derive(Debug, Clone, PartialEq)]
pub struct DiamondLookup {
    pub schema: String,
    pub valid: bool,
    pub name: Option<String>,
    pub serial: Option<String>,
    pub error: Option<String>,
}

/// Diamond serials are assigned on-chain as u32 (`field::DiamondNumber`); the
/// range check here is a plausibility gate, not a chain query.
fn serial_valid(serial: u64) -> bool {
    (1..=u32::MAX as u64).contains(&serial)
}

/// `diamond.lookup`: validate a diamond name (charset/format via the wire
/// type's own rules) or a serial number.
pub fn lookup(name: Option<&str>, serial: Option<&str>) -> Result<DiamondLookup, SdkError> {
    match (name.map(str::trim), serial.map(str::trim)) {
        (Some(name), None) => {
            let canonical = name.to_owned();
            let result = match field::DiamondName::from_readable(name) {
                Ok(diamond) => DiamondLookup {
                    schema: SCHEMA_DIAMOND_LOOKUP.to_owned(),
                    valid: true,
                    name: Some(diamond.to_readable()),
                    serial: None,
                    error: None,
                },
                Err(error) => DiamondLookup {
                    schema: SCHEMA_DIAMOND_LOOKUP.to_owned(),
                    valid: false,
                    name: Some(canonical),
                    serial: None,
                    error: Some(error.to_string()),
                },
            };
            Ok(result)
        }
        (None, Some(serial)) => {
            let value: u64 = serial.parse().map_err(|_| {
                SdkError::new(
                    SdkErrorCode::ParseFailed,
                    format!("serial {serial:?} is not a decimal number"),
                )
            })?;
            let valid = serial_valid(value);
            Ok(DiamondLookup {
                schema: SCHEMA_DIAMOND_LOOKUP.to_owned(),
                valid,
                name: None,
                serial: Some(value.to_string()),
                error: if valid {
                    None
                } else {
                    Some("diamond serial must be in 1..=4294967295".to_owned())
                },
            })
        }
        (Some(_), Some(_)) => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "provide exactly one of `name` or `serial`",
        )),
        (None, None) => Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "provide exactly one of `name` or `serial`",
        )),
    }
}

impl SdkJsonTo for DiamondLookup {
    fn to_json_string(&self) -> String {
        use crate::json::{kv, kv_opt, obj, q};
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("valid", if self.valid { "true".to_owned() } else { "false".to_owned() }),
            kv_opt("name", self.name.as_deref().map(q)),
            kv_opt("serial", self.serial.as_deref().map(q)),
            kv_opt("error", self.error.as_deref().map(q)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_a_name() {
        // 6 chars from the frozen `WTYUIAHXVMEKBSZN` alphabet.
        let result = lookup(Some("WTYUIA"), None).unwrap();
        assert!(result.valid);
        assert_eq!(result.name.as_deref(), Some("WTYUIA"));
        assert!(result.error.is_none());
    }

    #[test]
    fn rejects_an_invalid_name_as_a_fact() {
        let result = lookup(Some("ABC"), None).unwrap();
        assert!(!result.valid);
        assert!(result.error.is_some());
    }

    #[test]
    fn validates_a_serial() {
        let result = lookup(None, Some("5")).unwrap();
        assert!(result.valid);
        assert_eq!(result.serial.as_deref(), Some("5"));
    }

    #[test]
    fn rejects_zero_serial_as_a_fact() {
        let result = lookup(None, Some("0")).unwrap();
        assert!(!result.valid);
        assert!(result.error.is_some());
    }

    #[test]
    fn rejects_non_decimal_serial() {
        let error = lookup(None, Some("abc")).unwrap_err();
        assert_eq!(error.code, "parse_failed");
    }

    #[test]
    fn requires_exactly_one_input() {
        assert_eq!(
            lookup(Some("ABC"), Some("5")).unwrap_err().code,
            "parse_failed"
        );
        assert_eq!(lookup(None, None).unwrap_err().code, "parse_failed");
    }
}
