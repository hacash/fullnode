use field::{DiamondName, JSONFormater, json_escape};
use sys::ToHex;

use super::request::json_string;

fn json_member(key: &str, value: impl AsRef<str>) -> String {
    let mut s = json_escape(key);
    s.push(':');
    s.push_str(value.as_ref());
    s
}

fn json_object(members: impl IntoIterator<Item = String>) -> String {
    let mut s = String::from("{");
    let mut first = true;
    for member in members {
        if member.is_empty() {
            continue;
        }
        if !first {
            s.push(',');
        }
        first = false;
        s.push_str(&member);
    }
    s.push('}');
    s
}

fn json_array(items: impl IntoIterator<Item = String>) -> String {
    let mut s = String::from("[");
    let mut first = true;
    for item in items {
        if !first {
            s.push(',');
        }
        first = false;
        s.push_str(&item);
    }
    s.push(']');
    s
}

pub(crate) fn diamond_names_readable(names: &[u8]) -> String {
    names
        .chunks_exact(DiamondName::SIZE)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

pub(crate) fn action_desc_array_json(
    tx: &dyn base::TransactionSign,
    unit: &str,
    description: bool,
) -> String {
    // Protocol JSON is emitted as-is. Description, when requested, is a sibling
    // field on a wrapper object — never injected into the action.
    json_array(tx.actions().iter().map(|act| {
        let protocol = act.to_json_fmt(&JSONFormater::new_unit(unit));
        if description {
            json_object([
                json_member("action", protocol),
                json_member("description", json_escape(&act.description())),
            ])
        } else {
            protocol
        }
    }))
}

pub(crate) fn tx_signature_report_json(tx: &dyn base::TransactionSign) -> Option<String> {
    let report = protocol::tx_std::signature_report(tx).ok()?;
    Some(json_array(report.required.iter().map(|addr| {
        json_object([
            json_member("address", json_string(&addr.to_readable())),
            json_member(
                "complete",
                if report.valid.contains(addr) {
                    "true"
                } else {
                    "false"
                },
            ),
        ])
    })))
}

pub(crate) fn transaction_fields_json(
    tx: &dyn base::TransactionSign,
    block: Option<&dyn base::Block>,
    last_height: u64,
    unit: &str,
    body: bool,
    action: bool,
    signature: bool,
    description: bool,
    pending: bool,
) -> String {
    let fee_str = tx.fee().to_unit_string(unit);
    let main_addr = tx.main().to_readable();
    let mut fields = vec![
        json_member("hash", json_string(&tx.hash().as_ref().to_hex())),
        json_member(
            "hash_with_fee",
            json_string(&tx.hash_with_fee().as_ref().to_hex()),
        ),
        json_member("type", tx.ty().to_string()),
        json_member("timestamp", tx.timestamp().value().to_string()),
        json_member("fee", json_string(&fee_str)),
        json_member("fee_got", json_string(&tx.fee_got().to_unit_string(unit))),
        json_member("main_address", json_string(&main_addr)),
        json_member("action", tx.action_count().to_string()),
    ];
    if let Some(gas_max) = tx.gas_max_byte() {
        fields.push(json_member("gas_max", gas_max.to_string()));
    }
    if body {
        fields.push(json_member("body", json_string(&tx.encode().to_hex())));
    }
    if description {
        fields.push(json_member(
            "description",
            json_string(&format!(
                "Main account {} pay {} HAC tx fee",
                main_addr, fee_str
            )),
        ));
    }
    if signature {
        if let Some(report) = tx_signature_report_json(tx) {
            fields.push(json_member("signatures", report));
        }
    }
    if let Some(block) = block {
        let tx_height = block.height();
        fields.push(json_member(
            "block",
            json_object([
                json_member("height", tx_height.to_string()),
                json_member("timestamp", block.timestamp().to_string()),
            ]),
        ));
        fields.push(json_member(
            "confirm",
            last_height.saturating_sub(tx_height).to_string(),
        ));
    }
    if action {
        fields.push(json_member(
            "actions",
            action_desc_array_json(tx, unit, description),
        ));
    }
    if pending {
        fields.push(json_member("pending", "true"));
    }
    fields.join(",")
}

pub(crate) fn transaction_basic_json(
    tx: &dyn base::TransactionSign,
    block: Option<&dyn base::Block>,
    last_height: u64,
    unit: &str,
    body: bool,
    action: bool,
    signature: bool,
    description: bool,
    pending: bool,
) -> String {
    let mut s = String::from("{");
    s.push_str(&json_member("ret", "0"));
    s.push(',');
    s.push_str(&transaction_fields_json(
        tx,
        block,
        last_height,
        unit,
        body,
        action,
        signature,
        description,
        pending,
    ));
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use field::{Address, Amount, Satoshi};
    use protocol::action_std::TransferSatFromTo;
    use protocol::tx_std::StdTransaction;

    use super::*;

    #[test]
    fn action_array_uses_the_complete_action_json_contract() {
        let from = Address::from([
            0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let to = Address::from([
            0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let mut tx = StdTransaction::new_by(hacash_params::TX_TYPE_2, from, Amount::zero(), 1);
        tx.push_action_in(Arc::new(TransferSatFromTo::new(from, to, Satoshi::from(7))));

        let json = action_desc_array_json(&tx, "fin", true);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let wrapped = &value[0];
        let action = &wrapped["action"];

        assert_eq!(action["kind"], TransferSatFromTo::KIND);
        assert_eq!(action["from"], from.to_readable());
        assert_eq!(action["to"], to.to_readable());
        assert_eq!(action["satoshi"], 7);
        assert!(action.get("description").is_none());
        assert_eq!(
            wrapped["description"],
            format!(
                "Transfer 7 SAT from {} to {}",
                from.to_readable(),
                to.to_readable()
            )
        );
    }

    #[test]
    fn transaction_envelope_is_valid_json() {
        let from = Address::from([
            0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let to = Address::from([
            0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let mut tx = StdTransaction::new_by(hacash_params::TX_TYPE_2, from, Amount::zero(), 1);
        tx.push_action_in(Arc::new(TransferSatFromTo::new(from, to, Satoshi::from(7))));

        let json = transaction_basic_json(&tx, None, 0, "fin", true, true, true, true, true);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["ret"], 0);
        assert_eq!(value["type"], 2);
        assert_eq!(value["pending"], true);
        assert_eq!(value["action"], 1);
        assert!(value["actions"].is_array());
        assert!(value["body"].is_string());
    }
}
