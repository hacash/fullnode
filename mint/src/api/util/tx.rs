use sys::ToHex;

use super::request::json_string;

pub(crate) fn action_desc_array_json(
    tx: &dyn base::Transaction,
    unit: &str,
    description: bool,
) -> String {
    let items = tx
        .actions()
        .iter()
        .map(|act| {
            let mut fields = vec![format!("\"kind\":{}", act.kind())];
            if description {
                fields.push(format!(
                    "\"description\":{}",
                    json_string(&act.description())
                ));
            }
            if let Some(transfer) = act.as_transfer_like() {
                fields.push(format!(
                    "\"amount\":{}",
                    json_string(&transfer.transfer_amount().to_unit_string(unit))
                ));
                if let base::TransferPayload::Asset { serial, amount } = transfer.transfer_payload()
                {
                    fields.push(format!(
                        "\"asset\":{{\"serial\":{},\"amount\":{}}}",
                        serial, amount
                    ));
                }
                if let Some(base::AddrOrPtr::Addr(from)) = transfer.transfer_from() {
                    fields.push(format!("\"from\":{}", json_string(&from.to_readable())));
                }
                let to = match transfer.transfer_to_ptr() {
                    Some(base::AddrOrPtr::Addr(addr)) => addr,
                    Some(base::AddrOrPtr::Ptr(i)) => {
                        tx.addrs().get(i as usize).copied().unwrap_or_default()
                    }
                    None => tx.main(),
                };
                fields.push(format!("\"to\":{}", json_string(&to.to_readable())));
            }
            format!("{{{}}}", fields.join(","))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", items)
}

pub(crate) fn tx_signature_report_json(tx: &dyn base::Transaction) -> Option<String> {
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
    tx: &dyn base::Transaction,
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
    tx: &dyn base::Transaction,
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
