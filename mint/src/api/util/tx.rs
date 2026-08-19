use field::{DiamondName, JSONFormater};
use sys::ToHex;

use super::request::json_string;

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
    let items = tx
        .actions()
        .iter()
        .map(|act| {
            let mut obj = match serde_json::from_str::<serde_json::Value>(
                &act.to_json_fmt(&JSONFormater::new_unit(unit)),
            ) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            };
            obj.insert("kind".to_owned(), serde_json::json!(act.kind()));
            if description {
                obj.insert(
                    "description".to_owned(),
                    serde_json::json!(act.description()),
                );
            }
            serde_json::to_string(&serde_json::Value::Object(obj))
                .unwrap_or_else(|_| "{}".to_owned())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", items)
}

pub(crate) fn tx_signature_report_json(tx: &dyn base::TransactionSign) -> Option<String> {
    let report = protocol::tx_std::signature_report(tx).ok()?;
    let items = report
        .required
        .iter()
        .map(|addr| {
            format!(
                "{{\"address\":{},\"complete\":{}}}",
                json_string(&addr.to_readable()),
                report.valid.contains(addr)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    Some(format!("[{}]", items))
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
        format!("\"hash\":{}", json_string(&tx.hash().as_ref().to_hex())),
        format!(
            "\"hash_with_fee\":{}",
            json_string(&tx.hash_with_fee().as_ref().to_hex())
        ),
        format!("\"type\":{}", tx.ty()),
        format!("\"timestamp\":{}", tx.timestamp().value()),
        format!("\"fee\":{}", json_string(&fee_str)),
        format!(
            "\"fee_got\":{}",
            json_string(&tx.fee_got().to_unit_string(unit))
        ),
        format!("\"main_address\":{}", json_string(&main_addr)),
        format!("\"action\":{}", tx.action_count()),
    ];
    if let Some(gas_max) = tx.gas_max_byte() {
        fields.push(format!("\"gas_max\":{}", gas_max));
    }
    if body {
        fields.push(format!("\"body\":{}", json_string(&tx.encode().to_hex())));
    }
    if description {
        fields.push(format!(
            "\"description\":{}",
            json_string(&format!(
                "Main account {} pay {} HAC tx fee",
                main_addr, fee_str
            ))
        ));
    }
    if signature {
        if let Some(report) = tx_signature_report_json(tx) {
            fields.push(format!("\"signatures\":{}", report));
        }
    }
    if let Some(block) = block {
        let tx_height = block.height();
        fields.push(format!(
            "\"block\":{{\"height\":{},\"timestamp\":{}}}",
            tx_height,
            block.timestamp()
        ));
        fields.push(format!(
            "\"confirm\":{}",
            last_height.saturating_sub(tx_height)
        ));
    }
    if action {
        fields.push(format!(
            "\"actions\":{}",
            action_desc_array_json(tx, unit, description)
        ));
    }
    if pending {
        fields.push("\"pending\":true".to_owned());
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
    format!(
        "{{\"ret\":0,{}}}",
        transaction_fields_json(
            tx,
            block,
            last_height,
            unit,
            body,
            action,
            signature,
            description,
            pending,
        )
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use field::{Address, Amount, Satoshi};
    use protocol::action_std::SatFromToTrs;
    use protocol::tx_std::TransactionType2;

    use super::*;

    #[test]
    fn action_array_uses_the_complete_action_json_contract() {
        let from = Address::from([
            0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let to = Address::from([
            0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let mut tx = TransactionType2::new_by(from, Amount::zero(), 1);
        tx.push_action_in(Arc::new(SatFromToTrs::new(from, to, Satoshi::from(7))));

        let json = action_desc_array_json(&tx, "fin", true);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let action = &value[0];

        assert_eq!(action["kind"], SatFromToTrs::KIND);
        assert_eq!(action["from"], from.to_readable());
        assert_eq!(action["to"], to.to_readable());
        assert_eq!(action["satoshi"], 7);
        // The action-codec-derive contract generates a human-readable
        // description for transfer actions.
        assert_eq!(
            action["description"],
            format!(
                "Transfer 7 SAT from {} to {}",
                from.to_readable(),
                to.to_readable()
            )
        );
    }
}
