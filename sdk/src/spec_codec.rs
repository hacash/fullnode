//! Schema-driven JSON decoding of a transaction spec, and native encoding of
//! action fields. JSON field names and shapes follow `ActionSchema`; values
//! are converted to native bytes and constructed by the protocol decoder.

use sys::{errf, Ret};

use base::FieldWire;
use field::Encode;

use crate::build::{ActionSpec, TransactionSpec};
use crate::error::{SdkError, SdkErrorCode};
use crate::jsonparse::{find, number_value, object_pairs, optional_number, optional_string, parse_failed, reject_unknown, required, required_number, required_string, semantic_string, string_value};

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
    let actions_raw = required(&pairs, "actions", "JSON")?;
    let action_items = field::json_split_array(actions_raw)
        .map_err(|e| parse_failed(format!("transaction spec actions is not an array: {e}")))?;
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

fn parse_action_spec_json(raw: &str, index: usize) -> Result<ActionSpec, SdkError> {
    let context = format!("transaction action {index}");
    let (kind, fields) = parse_action_json_fields(raw, &context)?;
    Ok(ActionSpec { kind, fields })
}

fn parse_action_json_fields(
    raw: &str,
    context: &str,
) -> Result<(String, Vec<(String, WireValue)>), SdkError> {
    let pairs = object_pairs(raw, context)?;
    let kind = required_string(&pairs, "kind", context)?;
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
    reject_unknown(&pairs, &allowed, context)?;
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
        let Some(raw) = find(pairs, field.name) else {
            if field.optional {
                continue;
            }
            return Err(parse_failed(format!(
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
    let text = string_value(raw, context, "JSON")?;
    let clean = text.trim_start_matches("0x").trim_start_matches("0X");
    hex::decode(clean).map_err(|_| parse_failed(format!("{context} must be hex")))
}

fn parse_wire_json(raw: &str, wire: &FieldWire, context: &str) -> Result<WireValue, SdkError> {
    match wire {
        FieldWire::U1 | FieldWire::U2 | FieldWire::U4 | FieldWire::U5 | FieldWire::U8 => {
            Ok(WireValue::Num(number_value(raw, context, "JSON")?))
        }
        FieldWire::Address | FieldWire::AddrOrPtr | FieldWire::AddrOrList => {
            Ok(WireValue::Str(string_value(raw, context, "JSON")?))
        }
        FieldWire::Amount
        | FieldWire::WireAmount
        | FieldWire::Satoshi
        | FieldWire::Fold64
        | FieldWire::Timestamp
        | FieldWire::DiamondNumber => Ok(WireValue::Str(semantic_string(raw, context, "JSON")?)),
        FieldWire::Fixed(_) | FieldWire::BytesW1 | FieldWire::BytesW2 | FieldWire::SignW2 => {
            Ok(WireValue::Hex(parse_hex_bytes(raw, context)?))
        }
        FieldWire::DiamondName => Ok(WireValue::Hex(
            string_value(raw, context, "JSON")?.into_bytes(),
        )),
        FieldWire::AssetAmt => {
            let pairs = object_pairs(raw, context)?;
            reject_unknown(&pairs, &["serial", "amount"], context)?;
            Ok(WireValue::Struct(vec![
                (
                    "serial".to_owned(),
                    WireValue::Str(semantic_string(
                        required(&pairs, "serial", "JSON")?,
                        "serial",
                        "JSON",
                    )?),
                ),
                (
                    "amount".to_owned(),
                    WireValue::Str(semantic_string(
                        required(&pairs, "amount", "JSON")?,
                        "amount",
                        "JSON",
                    )?),
                ),
            ]))
        }
        FieldWire::ListW1(_) | FieldWire::ListW2(_) => {
            let items = field::json_split_array(raw)
                .map_err(|e| parse_failed(format!("{context} is not an array: {e}")))?;
            let elem = element_wire(wire);
            items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_wire_json(item, &elem, &format!("{context}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(WireValue::List)
        }
        FieldWire::Struct(name) => {
            let pairs = object_pairs(raw, context)?;
            let fields = struct_fields_of(name).ok_or_else(|| {
                parse_failed(format!("{context} references unknown struct {name}"))
            })?;
            let allowed: Vec<&str> = fields
                .iter()
                .filter(|field| field.name != "kind")
                .map(|field| field.name)
                .collect();
            reject_unknown(&pairs, &allowed, context)?;
            parse_schema_json_fields(&pairs, fields, context).map(WireValue::Struct)
        }
        FieldWire::ActionListW1 => {
            let items = field::json_split_array(raw)
                .map_err(|e| parse_failed(format!("{context} is not an array: {e}")))?;
            let mut values = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let nested_context = format!("{context}[{index}]");
                let (kind, mut fields) = parse_action_json_fields(item, &nested_context)?;
                fields.insert(0, ("kind".to_owned(), WireValue::Str(kind)));
                values.push(WireValue::Struct(fields));
            }
            Ok(WireValue::List(values))
        }
        // Legacy named list wires with no registered schema producer today
        // (diamond/chain/address lists are generic `ListW1`/`ListW2` of a leaf
        // name, timestamps are not action fields). Kept explicit so a future
        // schema cannot silently take a wrong shape here.
        FieldWire::ChainIDList
        | FieldWire::DiamondNameList
        | FieldWire::ContractAddrListW1
        | FieldWire::AssetAmtW1
        | FieldWire::ActionList => Err(parse_failed(format!(
            "{context}: wire {wire:?} is not produced by any registered schema"
        ))),
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
            // Length checked above: copy into the fixed array instead of a
            // `try_into().expect()` so no panic path (and its location string)
            // is linked into the wasm.
            let mut name = [0u8; DiamondName::SIZE];
            name.copy_from_slice(bytes);
            DiamondName::from(name).encode_to(out);
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
            let mut publickey = [0u8; Sign::PUBLICKEY_SIZE];
            publickey.copy_from_slice(&bytes[..Sign::PUBLICKEY_SIZE]);
            let mut signature = [0u8; Sign::SIGNATURE_SIZE];
            signature.copy_from_slice(&bytes[Sign::PUBLICKEY_SIZE..]);
            Sign {
                publickey,
                signature,
            }
            .encode_to(out);
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
        FieldWire::ActionListW1 => {
            let items = value.as_list()?;
            Uint1::from_usize(items.len())?.encode_to(out);
            for item in items {
                encode_nested_action(out, item)?;
            }
        }
        // Legacy named list wires with no registered schema producer today;
        // parsing already rejects them, encoding must not silently pick a shape.
        FieldWire::ChainIDList
        | FieldWire::DiamondNameList
        | FieldWire::ContractAddrListW1
        | FieldWire::AssetAmtW1
        | FieldWire::ActionList => {
            return errf!("wire {wire:?} is not produced by any registered schema");
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
