//! Envelope JSON decoding of a transaction spec. Action objects are kept as
//! raw protocol JSON and handed to `JsonCodecs::decode_action_json`.

use field::{AddrOrList, FromJSON};

use crate::build::{ActionSpec, TransactionSpec};
use crate::error::SdkError;
use crate::jsonparse::{
    find, object_pairs, optional_number, optional_string, parse_failed, reject_unknown, required,
    required_number, required_string,
};

/// Decode the public JSON TransactionSpec: top-level envelope is SDK-owned,
/// each `actions[i]` is a protocol action object (numeric `kind`).
pub fn decode_transaction_spec_json(json: &str) -> Result<TransactionSpec, SdkError> {
    let pairs = object_pairs(json, "transaction spec")?;
    reject_unknown(
        &pairs,
        &[
            "schema",
            "tx_type",
            "main",
            "fee",
            "timestamp",
            "gas_max",
            "addrlist",
            "actions",
        ],
        "transaction spec",
    )?;
    let schema = optional_string(&pairs, "schema", "JSON")?;
    let tx_type = required_number(&pairs, "tx_type", "JSON")?;
    let main = required_string(&pairs, "main", "JSON")?;
    let fee = required_string(&pairs, "fee", "JSON")?;
    let timestamp = optional_number(&pairs, "timestamp", "JSON")?;
    let gas_max = optional_number(&pairs, "gas_max", "JSON")?;
    let addrlist = match find(&pairs, "addrlist") {
        Some(raw) => {
            let mut value = AddrOrList::default();
            value
                .from_json(raw)
                .map_err(|e| parse_failed(format!("transaction spec addrlist invalid: {e}")))?;
            Some(value)
        }
        None => None,
    };
    let actions_raw = required(&pairs, "actions", "JSON")?;
    let action_items = field::json_split_array(actions_raw)
        .map_err(|e| parse_failed(format!("transaction spec actions is not an array: {e}")))?;
    let mut actions = Vec::with_capacity(action_items.len());
    for (i, raw) in action_items.iter().enumerate() {
        let spec = ActionSpec::new((*raw).to_owned())
            .map_err(|e| parse_failed(format!("actions[{i}]: {}", e.message)))?;
        actions.push(spec);
    }
    Ok(TransactionSpec {
        schema,
        tx_type,
        main,
        fee,
        timestamp,
        gas_max,
        addrlist,
        actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_spec_builds_a_hac_transfer() {
        let json = r#"{
            "tx_type": 2,
            "main": "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
            "fee": "1:244",
            "timestamp": 1755223764,
            "actions": [
                {
                    "kind": 1,
                    "to": "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
                    "hacash": "12:244"
                }
            ]
        }"#;
        let spec = decode_transaction_spec_json(json).unwrap();
        assert_eq!(spec.actions[0].kind, 1);
        let action_json = &spec.actions[0].json;
        assert!(
            action_json.contains("\"kind\": 1") || action_json.contains("\"kind\":1"),
            "{action_json}"
        );
        let built = crate::build::build_transaction(&spec).unwrap();
        assert_eq!(built.tx_type, 2);
    }

    #[test]
    fn json_spec_rejects_non_object_action_and_missing_kind() {
        let envelope = |actions: &str| {
            format!(
                r#"{{
                    "tx_type": 2,
                    "main": "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
                    "fee": "1:244",
                    "actions": {actions}
                }}"#
            )
        };
        let array = decode_transaction_spec_json(&envelope("[1]")).unwrap_err();
        assert!(array.message.contains("actions[0]"), "{}", array.message);
        let missing = decode_transaction_spec_json(&envelope(r#"[{"to":"x"}]"#)).unwrap_err();
        assert!(
            missing.message.contains("actions[0]")
                && missing.message.contains("missing numeric kind"),
            "{}",
            missing.message
        );
        // A quoted "1" is not a registered action name: the gate resolves the
        // same name space as the registry dispatch (numbers must be unquoted).
        let quoted = decode_transaction_spec_json(&envelope(r#"[{"kind":"1"}]"#)).unwrap_err();
        assert!(
            quoted.message.contains("unknown action kind name"),
            "{}",
            quoted.message
        );
    }
}
