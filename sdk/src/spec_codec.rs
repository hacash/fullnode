//! Schema-driven JSON decoding of a transaction spec, and native encoding of
//! action fields. JSON field names and shapes follow `ActionSchema`; values
//! are converted to native bytes and constructed by the protocol decoder.

use sys::{errf, Ret};

use base::FieldWire;
use field::Encode;

use crate::build::{ActionSpec, TransactionSpec};
use crate::error::{SdkError, SdkErrorCode};

/// One schema field value after JSON parse (and before native encode).
#[derive(Debug, Clone, PartialEq)]
pub enum WireValue {
    Num(u64),
    Str(String),
    Hex(Vec<u8>),
    List(Vec<WireValue>),
    Struct(Vec<(String, WireValue)>),
}

impl WireValue {
    fn as_num(&self) -> Ret<u64> {
        match self {
            WireValue::Num(n) => Ok(*n),
            _ => errf!("expected numeric wire value, got {self:?}"),
        }
    }

    fn as_str(&self) -> Ret<&str> {
        match self {
            WireValue::Str(s) => Ok(s),
            _ => errf!("expected string wire value, got {self:?}"),
        }
    }

    fn as_hex(&self) -> Ret<&[u8]> {
        match self {
            WireValue::Hex(b) => Ok(b),
            _ => errf!("expected hex wire value, got {self:?}"),
        }
    }

    fn as_list(&self) -> Ret<&[WireValue]> {
        match self {
            WireValue::List(items) => Ok(items),
            _ => errf!("expected list wire value, got {self:?}"),
        }
    }

    fn as_struct(&self) -> Ret<&[(String, WireValue)]> {
        match self {
            WireValue::Struct(items) => Ok(items),
            _ => errf!("expected struct wire value, got {self:?}"),
        }
    }
}

/// Wire shape of list elements: resolved by name to a struct schema or built-in leaf.
fn element_wire(wire: &FieldWire) -> FieldWire {
    let name = match wire {
        FieldWire::ListW1(name) | FieldWire::ListW2(name) => name,
        _ => unreachable!("element_wire called on non-list"),
    };
    if crate::selection::action_schema_named(name).is_some()
        || crate::selection::struct_schema_named(name).is_some()
    {
        return FieldWire::Struct(name);
    }
    base::builtin_leaf_wire(name).unwrap_or(FieldWire::Struct(name))
}

fn struct_fields_of(name: &str) -> Option<&'static [base::FieldSchema]> {
    if let Some(schema) = crate::selection::action_schema_named(name) {
        return Some(schema.fields);
    }
    crate::selection::struct_schema_named(name).map(|schema| schema.fields)
}

/// Decode the public JSON TransactionSpec: top-level layout is fixed,
/// action/struct fields resolve from the same schema registry as encode.
pub fn decode_transaction_spec_json(json: &str) -> Result<TransactionSpec, SdkError> {
    let pairs = json_object_pairs(json, "transaction spec")?;
    reject_unknown_json_fields(
        &pairs,
        &[
            "schema",
            "tx_type",
            "main",
            "fee",
            "timestamp",
            "gas_max",
            "actions",
        ],
        "transaction spec",
    )?;
    let schema = json_optional_string(&pairs, "schema")?;
    let tx_type = json_required_number(&pairs, "tx_type")?;
    let main = json_required_string(&pairs, "main")?;
    let fee = json_required_string(&pairs, "fee")?;
    let timestamp = json_optional_number(&pairs, "timestamp")?;
    let gas_max = json_optional_number(&pairs, "gas_max")?;
    let actions_raw = json_required(&pairs, "actions")?;
    let action_items = field::json_split_array(actions_raw)
        .map_err(|e| json_parse_failed(format!("transaction spec actions is not an array: {e}")))?;
    let mut actions = Vec::with_capacity(action_items.len());
    for (index, raw) in action_items.iter().enumerate() {
        actions.push(parse_action_spec_json(raw, index)?);
    }
    Ok(TransactionSpec {
        schema,
        tx_type,
        main,
        fee,
        timestamp,
        gas_max,
        actions,
    })
}

fn json_parse_failed(message: impl Into<String>) -> SdkError {
    SdkError::new(SdkErrorCode::ParseFailed, message)
}

fn json_object_pairs<'a>(raw: &'a str, context: &str) -> Result<Vec<(&'a str, &'a str)>, SdkError> {
    let pairs = field::json_split_object(raw)
        .map_err(|e| json_parse_failed(format!("{context} is not a JSON object: {e}")))?;
    let mut seen = std::collections::HashSet::new();
    for (key, _) in &pairs {
        if !seen.insert(*key) {
            return Err(json_parse_failed(format!(
                "{context} field {key} is duplicated"
            )));
        }
    }
    Ok(pairs)
}

fn reject_unknown_json_fields(
    pairs: &[(&str, &str)],
    allowed: &[&str],
    context: &str,
) -> Result<(), SdkError> {
    for (key, _) in pairs {
        if !allowed.iter().any(|known| *known == *key) {
            return Err(SdkError::new(
                SdkErrorCode::UnknownField,
                format!("{context} field {key} is unknown"),
            ));
        }
    }
    Ok(())
}

fn json_find<'a>(pairs: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

fn json_required<'a>(pairs: &'a [(&'a str, &'a str)], name: &str) -> Result<&'a str, SdkError> {
    json_find(pairs, name).ok_or_else(|| json_parse_failed(format!("JSON field {name} missing")))
}

fn json_string_value(raw: &str, name: &str) -> Result<String, SdkError> {
    field::json_expect_quoted_decoded(raw)
        .map_err(|e| json_parse_failed(format!("JSON field {name} is not a string: {e}")))
}

fn json_semantic_string(raw: &str, name: &str) -> Result<String, SdkError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('"') {
        json_string_value(trimmed, name)
    } else if !trimmed.is_empty()
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b':'))
    {
        Ok(trimmed.to_owned())
    } else {
        Err(json_parse_failed(format!(
            "JSON field {name} is not a semantic string"
        )))
    }
}

fn json_required_string(pairs: &[(&str, &str)], name: &str) -> Result<String, SdkError> {
    json_string_value(json_required(pairs, name)?, name)
}

fn json_optional_string(pairs: &[(&str, &str)], name: &str) -> Result<Option<String>, SdkError> {
    json_find(pairs, name)
        .map(|raw| json_string_value(raw, name))
        .transpose()
}

fn json_number_value<T: std::str::FromStr>(raw: &str, name: &str) -> Result<T, SdkError> {
    let raw = raw.trim();
    let text = if raw.starts_with('"') {
        json_string_value(raw, name)?
    } else {
        raw.to_owned()
    };
    text.parse()
        .map_err(|_| json_parse_failed(format!("JSON field {name} is not a number")))
}

fn json_required_number<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    name: &str,
) -> Result<T, SdkError> {
    json_number_value(json_required(pairs, name)?, name)
}

fn json_optional_number<T: std::str::FromStr>(
    pairs: &[(&str, &str)],
    name: &str,
) -> Result<Option<T>, SdkError> {
    json_find(pairs, name)
        .map(|raw| json_number_value(raw, name))
        .transpose()
}

fn parse_action_spec_json(raw: &str, index: usize) -> Result<ActionSpec, SdkError> {
    let context = format!("transaction action {index}");
    let (kind, fields) = parse_action_json_fields(raw, &context)?;
    Ok(ActionSpec { kind, fields })
}

fn parse_action_json_fields(
    raw: &str,
    context: &str,
) -> Result<(String, Vec<(String, WireValue)>), SdkError> {
    let pairs = json_object_pairs(raw, context)?;
    let kind = json_required_string(&pairs, "kind")?;
    let schema = crate::selection::action_schema_named(&kind).ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::UnknownAction,
            format!("{context} kind {kind:?} is not registered"),
        )
    })?;
    let allowed: Vec<&str> = std::iter::once("kind")
        .chain(
            schema
                .fields
                .iter()
                .filter(|field| field.name != "kind")
                .map(|field| field.name),
        )
        .collect();
    reject_unknown_json_fields(&pairs, &allowed, context)?;
    let fields = parse_schema_json_fields(&pairs, schema.fields, context)?;
    Ok((kind, fields))
}

fn parse_schema_json_fields(
    pairs: &[(&str, &str)],
    fields: &[base::FieldSchema],
    context: &str,
) -> Result<Vec<(String, WireValue)>, SdkError> {
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        if field.name == "kind" {
            continue;
        }
        let Some(raw) = json_find(pairs, field.name) else {
            if field.optional {
                continue;
            }
            return Err(json_parse_failed(format!(
                "{context} field {} missing",
                field.name
            )));
        };
        values.push((
            field.name.to_owned(),
            parse_wire_json(raw, &field.wire, &format!("{context}.{}", field.name))?,
        ));
    }
    Ok(values)
}

fn parse_hex_bytes(raw: &str, context: &str) -> Result<Vec<u8>, SdkError> {
    let text = json_string_value(raw, context)?;
    let clean = text.trim_start_matches("0x").trim_start_matches("0X");
    hex::decode(clean).map_err(|_| json_parse_failed(format!("{context} must be hex")))
}

fn parse_wire_json(raw: &str, wire: &FieldWire, context: &str) -> Result<WireValue, SdkError> {
    match wire {
        FieldWire::U1 | FieldWire::U2 | FieldWire::U4 | FieldWire::U5 | FieldWire::U8 => {
            Ok(WireValue::Num(json_number_value(raw, context)?))
        }
        FieldWire::Address | FieldWire::AddrOrPtr | FieldWire::AddrOrList => {
            Ok(WireValue::Str(json_string_value(raw, context)?))
        }
        FieldWire::Amount
        | FieldWire::WireAmount
        | FieldWire::Satoshi
        | FieldWire::Fold64
        | FieldWire::Timestamp
        | FieldWire::DiamondNumber => Ok(WireValue::Str(json_semantic_string(raw, context)?)),
        FieldWire::Fixed(_)
        | FieldWire::BytesW1
        | FieldWire::BytesW2
        | FieldWire::SignW2
        | FieldWire::AssetAmtW1 => Ok(WireValue::Hex(parse_hex_bytes(raw, context)?)),
        FieldWire::DiamondName => Ok(WireValue::Hex(
            json_string_value(raw, context)?.into_bytes(),
        )),
        FieldWire::DiamondNameList => {
            let items = field::json_split_array(raw)
                .map_err(|e| json_parse_failed(format!("{context} is not an array: {e}")))?;
            items
                .iter()
                .map(|item| {
                    Ok(WireValue::Hex(
                        json_string_value(item, context)?.into_bytes(),
                    ))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(WireValue::List)
        }
        FieldWire::ChainIDList => {
            let items = field::json_split_array(raw)
                .map_err(|e| json_parse_failed(format!("{context} is not an array: {e}")))?;
            items
                .iter()
                .map(|item| {
                    let id: u32 = json_number_value(item, context)?;
                    Ok(WireValue::Hex(id.to_be_bytes().to_vec()))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(WireValue::List)
        }
        FieldWire::ContractAddrListW1 => {
            let items = field::json_split_array(raw)
                .map_err(|e| json_parse_failed(format!("{context} is not an array: {e}")))?;
            items
                .iter()
                .map(|item| Ok(WireValue::Str(json_string_value(item, context)?)))
                .collect::<Result<Vec<_>, _>>()
                .map(WireValue::List)
        }
        FieldWire::AssetAmt => {
            let pairs = json_object_pairs(raw, context)?;
            reject_unknown_json_fields(&pairs, &["serial", "amount"], context)?;
            Ok(WireValue::Struct(vec![
                (
                    "serial".to_owned(),
                    WireValue::Str(json_semantic_string(
                        json_required(&pairs, "serial")?,
                        "serial",
                    )?),
                ),
                (
                    "amount".to_owned(),
                    WireValue::Str(json_semantic_string(
                        json_required(&pairs, "amount")?,
                        "amount",
                    )?),
                ),
            ]))
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let items = field::json_split_array(raw)
                .map_err(|e| json_parse_failed(format!("{context} is not an array: {e}")))?;
            let elem = element_wire(wire);
            items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_wire_json(item, &elem, &format!("{context}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(WireValue::List)
        }
        FieldWire::Struct(name) => {
            let pairs = json_object_pairs(raw, context)?;
            let fields = struct_fields_of(name).ok_or_else(|| {
                json_parse_failed(format!("{context} references unknown struct {name}"))
            })?;
            let allowed: Vec<&str> = fields
                .iter()
                .filter(|field| field.name != "kind")
                .map(|field| field.name)
                .collect();
            reject_unknown_json_fields(&pairs, &allowed, context)?;
            parse_schema_json_fields(&pairs, fields, context).map(WireValue::Struct)
        }
        FieldWire::ActionList | FieldWire::ActionListW1 => {
            let items = field::json_split_array(raw)
                .map_err(|e| json_parse_failed(format!("{context} is not an array: {e}")))?;
            let mut values = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let nested_context = format!("{context}[{index}]");
                let (kind, mut fields) = parse_action_json_fields(item, &nested_context)?;
                fields.insert(0, ("kind".to_owned(), WireValue::Str(kind)));
                values.push(WireValue::Struct(fields));
            }
            Ok(WireValue::List(values))
        }
    }
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Encode one field value to its native wire layout.
pub(crate) fn encode_wire(out: &mut Vec<u8>, wire: &FieldWire, value: &WireValue) -> Ret<()> {
    use field::{
        AddrOrList, AddrOrPtr, Address, Amount, BytesW1, BytesW2, DiamondName, DiamondNumber,
        Fold64, Satoshi, Sign, Timestamp, Uint1, Uint2, Uint4, Uint5, WireAmount,
    };
    match wire {
        FieldWire::U1 | FieldWire::U8 => {
            let n = value.as_num()?;
            if n > 0xff {
                return errf!("u8 value {n} out of range");
            }
            Uint1::from(n as u8).encode_to(out);
        }
        FieldWire::U2 => {
            let n = value.as_num()?;
            if n > 0xffff {
                return errf!("u2 value {n} out of range");
            }
            Uint2::from(n as u16).encode_to(out);
        }
        FieldWire::U4 => {
            let n = value.as_num()?;
            if n > 0xffff_ffff {
                return errf!("u4 value {n} out of range");
            }
            Uint4::from(n as u32).encode_to(out);
        }
        FieldWire::U5 => {
            Uint5::from(value.as_num()?).encode_to(out);
        }
        FieldWire::Fixed(n) => {
            let bytes = value.as_hex()?;
            if bytes.len() != *n as usize {
                return errf!("fixed({n}) field must be {n} bytes, got {}", bytes.len());
            }
            out.extend_from_slice(bytes);
        }
        FieldWire::Amount => {
            Amount::from(value.as_str()?)?.encode_to(out);
        }
        FieldWire::WireAmount => {
            WireAmount::from(Amount::from(value.as_str()?)?).encode_to(out);
        }
        FieldWire::Address => {
            Address::from_readable(value.as_str()?)?.encode_to(out);
        }
        FieldWire::AddrOrPtr => {
            let address = Address::from_readable(value.as_str()?)?;
            AddrOrPtr::Addr(address).encode_to(out);
        }
        FieldWire::AddrOrList => {
            let address = Address::from_readable(value.as_str()?)?;
            AddrOrList::Single(address).encode_to(out);
        }
        FieldWire::Satoshi => {
            let n: u64 = value
                .as_str()?
                .parse()
                .map_err(|_| sys::Error::normal("satoshi not a decimal string"))?;
            Satoshi::from(n).encode_to(out);
        }
        FieldWire::Fold64 => {
            let n: u64 = value
                .as_str()?
                .parse()
                .map_err(|_| sys::Error::normal("fold64 not a decimal string"))?;
            Fold64::from(n)?.encode_to(out);
        }
        FieldWire::Timestamp => {
            let n: u64 = value
                .as_str()?
                .parse()
                .map_err(|_| sys::Error::normal("timestamp not a decimal string"))?;
            Timestamp::from_checked(n)?.encode_to(out);
        }
        FieldWire::DiamondNumber => {
            let n: u32 = value
                .as_str()?
                .parse()
                .map_err(|_| sys::Error::normal("diamond number not a decimal string"))?;
            DiamondNumber::from(n).encode_to(out);
        }
        FieldWire::DiamondName => {
            let bytes = value.as_hex()?;
            if bytes.len() != DiamondName::SIZE {
                return errf!(
                    "diamond name must be {} bytes, got {}",
                    DiamondName::SIZE,
                    bytes.len()
                );
            }
            DiamondName::from(bytes.try_into().expect("diamond name size checked")).encode_to(out);
        }
        FieldWire::BytesW1 => {
            BytesW1::from(value.as_hex()?.to_vec())?.encode_to(out);
        }
        FieldWire::BytesW2 => {
            BytesW2::from(value.as_hex()?.to_vec())?.encode_to(out);
        }
        FieldWire::SignW2 => {
            let bytes = value.as_hex()?;
            if bytes.len() != Sign::SIZE {
                return errf!("sign must be {} bytes, got {}", Sign::SIZE, bytes.len());
            }
            Sign {
                publickey: bytes[..Sign::PUBLICKEY_SIZE]
                    .try_into()
                    .expect("sign split"),
                signature: bytes[Sign::PUBLICKEY_SIZE..]
                    .try_into()
                    .expect("sign split"),
            }
            .encode_to(out);
        }
        FieldWire::AssetAmtW1 => {
            out.extend_from_slice(value.as_hex()?);
        }
        FieldWire::DiamondNameList => {
            let items = value.as_list()?;
            Uint1::from_usize(items.len())?.encode_to(out);
            for item in items {
                let bytes = item.as_hex()?;
                if bytes.len() != DiamondName::SIZE {
                    return errf!(
                        "diamond name must be {} bytes, got {}",
                        DiamondName::SIZE,
                        bytes.len()
                    );
                }
                DiamondName::from(bytes.try_into().expect("diamond name size checked"))
                    .encode_to(out);
            }
        }
        FieldWire::ChainIDList => {
            let items = value.as_list()?;
            Uint1::from_usize(items.len())?.encode_to(out);
            for item in items {
                let bytes = item.as_hex()?;
                if bytes.len() != 4 {
                    return errf!("chain id must be 4 bytes, got {}", bytes.len());
                }
                Uint4::from(u32::from_be_bytes(
                    bytes.try_into().expect("chain id size checked"),
                ))
                .encode_to(out);
            }
        }
        FieldWire::ContractAddrListW1 => {
            let items = value.as_list()?;
            Uint1::from_usize(items.len())?.encode_to(out);
            for item in items {
                Address::from_readable(item.as_str()?)?.encode_to(out);
            }
        }
        FieldWire::AssetAmt => {
            let items = value.as_struct()?;
            let serial: u64 = struct_str_of(items, "serial")?
                .parse()
                .map_err(|_| sys::Error::normal("asset serial not a decimal string"))?;
            let amount: u64 = struct_str_of(items, "amount")?
                .parse()
                .map_err(|_| sys::Error::normal("asset amount not a decimal string"))?;
            Fold64::from(serial)?.encode_to(out);
            Fold64::from(amount)?.encode_to(out);
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let items = value.as_list()?;
            match wire {
                FieldWire::ListW1(_) => Uint1::from_usize(items.len())?.encode_to(out),
                FieldWire::ListW2(_) => Uint2::from_usize(items.len())?.encode_to(out),
                _ => unreachable!("list arms"),
            }
            let elem_wire = element_wire(wire);
            for item in items {
                encode_wire(out, &elem_wire, item)?;
            }
        }
        FieldWire::Struct(name) => {
            encode_struct_fields(out, name, value.as_struct()?)?;
        }
        FieldWire::ActionList => {
            let items = value.as_list()?;
            Uint2::from_usize(items.len())?.encode_to(out);
            for item in items {
                encode_nested_action(out, item)?;
            }
        }
        FieldWire::ActionListW1 => {
            let items = value.as_list()?;
            Uint1::from_usize(items.len())?.encode_to(out);
            for item in items {
                encode_nested_action(out, item)?;
            }
        }
    }
    Ok(())
}

fn encode_nested_action(out: &mut Vec<u8>, value: &WireValue) -> Ret<()> {
    let items = value.as_struct()?;
    let kind = items
        .iter()
        .find(|(n, _)| n == "kind")
        .and_then(|(_, v)| match v {
            WireValue::Str(s) => Some(s.as_str()),
            _ => None,
        })
        .ok_or_else(|| sys::Error::fault("nested action missing kind"))?;
    encode_action(out, kind, items)
}

/// Encode one action: u16 kind + fields per that action's schema.
pub(crate) fn encode_action(
    out: &mut Vec<u8>,
    kind: &str,
    fields: &[(String, WireValue)],
) -> Ret<()> {
    let schema = crate::selection::action_schema_named(kind)
        .ok_or_else(|| sys::Error::fault(format!("no action schema for {kind}")))?;
    push_u16(out, schema.kind);
    encode_struct_fields(out, kind, fields)
}

fn struct_str_of<'a>(items: &'a [(String, WireValue)], name: &str) -> Ret<&'a str> {
    match items.iter().find(|(n, _)| n == name) {
        Some((_, WireValue::Str(s))) => Ok(s),
        _ => errf!("struct field {name} missing or not a string"),
    }
}

fn encode_struct_fields(out: &mut Vec<u8>, name: &str, items: &[(String, WireValue)]) -> Ret<()> {
    let fields = struct_fields_of(name)
        .ok_or_else(|| sys::Error::fault(format!("unknown struct schema {name}")))?;
    if fields.is_empty() {
        return errf!("struct schema {name} has no fields (not yet supported)");
    }
    for (field_name, _) in items {
        if items
            .iter()
            .filter(|(known, _)| known == field_name)
            .count()
            != 1
        {
            return errf!("struct {name} has duplicate field {field_name}");
        }
        if field_name == "kind" {
            continue;
        }
        if !fields.iter().any(|f| f.name == *field_name) {
            return errf!("struct {name} has unknown field {field_name}");
        }
    }
    for field in fields {
        if field.name == "kind" {
            continue;
        }
        match items.iter().find(|(n, _)| n == field.name) {
            Some((_, value)) => encode_wire(out, &field.wire, value)?,
            None if field.optional => {}
            None => {
                return errf!("struct {name} missing field {}", field.name);
            }
        }
    }
    Ok(())
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
                    "kind": "transfer_hac_to",
                    "to": "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
                    "hacash": "12:244"
                }
            ]
        }"#;
        let spec = decode_transaction_spec_json(json).unwrap();
        assert_eq!(spec.actions[0].kind, "transfer_hac_to");
        let built = crate::build::build_transaction(&spec).unwrap();
        assert_eq!(built.tx_type, 2);
    }
}
