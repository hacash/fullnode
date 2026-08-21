//! `tx.build`: construct unsigned Type-2/3 bodies from a wire-shaped action spec.
//! JSON fields follow `ActionSchema` names and shapes; the protocol decoder
//! is the only construction path. Kinds outside the SDK codec profile are rejected.

use base::{BinaryCodecs, TxCreateRequest};
use field::{Address, Amount};

use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::decode_tx;
use crate::schema::{SCHEMA_BUILT_TRANSACTION, SCHEMA_TRANSACTION_SPEC};
pub use crate::spec_codec::WireValue;

/// One wire action: `kind` is the schema name, `fields` are schema fields
/// excluding `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionSpec {
    pub kind: String,
    pub fields: Vec<(String, WireValue)>,
}

impl ActionSpec {
    pub fn new(kind: impl Into<String>, fields: Vec<(String, WireValue)>) -> Self {
        Self {
            kind: kind.into(),
            fields,
        }
    }

    pub fn to_json_string(&self) -> String {
        use crate::json::{kv, obj, q};
        let mut parts = vec![kv("kind", q(&self.kind))];
        parts.extend(
            self.fields
                .iter()
                .map(|(name, value)| kv(name, wire_value_json(value))),
        );
        obj(parts)
    }
}

#[derive(Debug, Clone)]
pub struct TransactionSpec {
    pub schema: Option<String>,
    pub tx_type: u8,
    pub main: String,
    pub fee: String,
    pub timestamp: Option<u64>,
    pub gas_max: Option<u8>,
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTransaction {
    pub schema: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
    pub hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub body: String,
}

pub fn build_transaction(spec: &TransactionSpec) -> Result<BuiltTransaction, SdkError> {
    if let Some(schema) = &spec.schema {
        if schema != SCHEMA_TRANSACTION_SPEC {
            return Err(SdkError::new(
                SdkErrorCode::UnsupportedSchema,
                format!("unsupported spec schema {schema:?}"),
            ));
        }
    }
    let main = Address::from_readable(&spec.main).map_err(|error| SdkError::from(error))?;
    let fee = Amount::from(&spec.fee).map_err(|error| SdkError::from(error))?;
    let fee_fin = fee.to_fin_string();
    let timestamp = spec.timestamp.unwrap_or_else(crate::now_secs);

    let mut actions = Vec::with_capacity(spec.actions.len());
    for action in &spec.actions {
        actions.push(build_action(action)?);
    }

    let body = protocol::tx_std::encode_standard_tx(
        TxCreateRequest::new(spec.tx_type, main, fee, timestamp)
            .with_gas_max(spec.gas_max.unwrap_or(0)),
        &actions,
        &[],
    )
    .map_err(SdkError::from)?;

    let body_hex = hex::encode(&body);
    let decoded = decode_tx(&body)?;
    let re_encoded = decoded.encode();
    if re_encoded != body {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "built body failed the encode(decode(body)) == body round-trip",
        ));
    }
    let unsigned_body_hash = crate::audit::unsigned_body_hash(&body_hex)?;
    Ok(BuiltTransaction {
        schema: SCHEMA_BUILT_TRANSACTION.to_owned(),
        tx_type: spec.tx_type,
        timestamp,
        main: main.to_readable(),
        fee: fee_fin,
        hash: hex::encode(decoded.hash().0),
        hash_with_fee: hex::encode(decoded.hash_with_fee().0),
        unsigned_body_hash,
        body: body_hex,
    })
}

fn build_action(spec: &ActionSpec) -> Result<base::ActionRef, SdkError> {
    build_raw(&spec.kind, &spec.fields)
}

/// Schema fields → native payload bytes → the protocol's own action decoder.
fn build_raw(kind: &str, fields: &[(String, WireValue)]) -> Result<base::ActionRef, SdkError> {
    let mut buf = Vec::new();
    crate::spec_codec::encode_action(&mut buf, kind, fields).map_err(SdkError::from)?;
    let codecs = crate::codec::standard_codecs().map_err(SdkError::from)?;
    let (action, used) = codecs.decode_action(&buf).map_err(|error| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("action {kind:?} has no usable transaction action codec ({error})"),
        )
    })?;
    let canonical = action.encode();
    if canonical.len() > buf.len() || &buf[..canonical.len()] != canonical.as_slice() {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            format!(
                "action {kind:?} decoded {used} of {} bytes and the consumed form does not re-encode",
                buf.len()
            ),
        ));
    }
    Ok(action)
}

fn wire_value_json(value: &WireValue) -> String {
    use crate::json::{arr, obj, q};
    match value {
        WireValue::Num(n) => n.to_string(),
        WireValue::Str(s) => q(s),
        WireValue::Hex(b) => q(&hex::encode(b)),
        WireValue::List(items) => arr(items.iter().map(wire_value_json).collect()),
        WireValue::Struct(items) => obj(items
            .iter()
            .map(|(name, value)| crate::json::kv(name, wire_value_json(value)))
            .collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn wv_str(s: &str) -> WireValue {
        WireValue::Str(s.to_owned())
    }
    fn wv_num(n: u64) -> WireValue {
        WireValue::Num(n)
    }
    fn action(kind: &str, fields: Vec<(&str, WireValue)>) -> ActionSpec {
        ActionSpec::new(
            kind,
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    fn sample_spec() -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 2,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![
                action(
                    "transfer_hac_to",
                    vec![("to", wv_str(MAIN)), ("hacash", wv_str("12:244"))],
                ),
                action(
                    "height_scope",
                    vec![("start", wv_num(1_000_000)), ("end", wv_num(0))],
                ),
            ],
        }
    }

    #[test]
    fn builds_type2_and_round_trips() {
        let built = build_transaction(&sample_spec()).unwrap();
        assert_eq!(built.tx_type, 2);
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(hex::encode(decoded.encode()), built.body);
        assert_eq!(decoded.action_count(), 2);
    }

    #[test]
    fn delegates_unknown_tx_type_to_protocol_constructor() {
        let mut spec = sample_spec();
        spec.tx_type = 0;
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(error
            .message
            .contains("unsupported standard user transaction type 0"));
    }

    #[test]
    fn type2_with_gas_max_builds_and_inspect_reports_schedule_fact() {
        let mut spec = sample_spec();
        spec.tx_type = 2;
        spec.gas_max = Some(10);
        let built = build_transaction(&spec).expect("type 2 with gas_max is wire-legal");
        let review = crate::inspect::inspect_report(
            &built.body,
            None,
            &crate::profile::CodecProfile::standard(),
        )
        .unwrap();
        assert!(
            review
                .schedule_violations
                .iter()
                .any(|f| f.contains("gas_max must be zero")),
            "got {:?}",
            review.schedule_violations
        );
        assert!(!review.protocol_valid);
    }

    #[test]
    fn explicit_from_builds_from_to_transfer_and_becomes_signer() {
        let other = sys::Account::create_by("654321").unwrap();
        let mut spec = sample_spec();
        spec.actions[0] = action(
            "transfer_hac_from_to",
            vec![
                ("from", wv_str(other.readable())),
                ("to", wv_str(MAIN)),
                ("hacash", wv_str("12:244")),
            ],
        );
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::HacFromToTrs::KIND
        );
        let required = decoded.req_sign().unwrap();
        let other_address = field::Address::from_readable(other.readable()).unwrap();
        assert!(required.contains(&other_address));
    }

    #[test]
    fn explicit_from_equal_to_main_keeps_the_from_to_form() {
        let mut spec = sample_spec();
        spec.actions[0] = action(
            "transfer_hac_from_to",
            vec![
                ("from", wv_str(MAIN)),
                ("to", wv_str(MAIN)),
                ("hacash", wv_str("12:244")),
            ],
        );
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::HacFromToTrs::KIND,
            "an explicit from equal to main must keep the from_to wire form (the SDK never rewrites it)"
        );
    }

    fn raw_spec(kind: &str, fields: Vec<(String, WireValue)>) -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![ActionSpec::new(kind, fields)],
        }
    }

    /// Every registered action builds through the schema path and the
    /// encode(decode(body)) == body invariant holds.
    #[test]
    fn raw_actions_build_through_the_protocol_codec() {
        let cases: Vec<(&str, Vec<(String, WireValue)>)> = vec![
            (
                "balance_floor",
                vec![
                    ("addr".to_owned(), wv_str(MAIN)),
                    ("hacash".to_owned(), wv_str("12:244")),
                    ("satoshi".to_owned(), wv_str("100")),
                    ("diamond".to_owned(), wv_str("5")),
                    (
                        "assets".to_owned(),
                        WireValue::List(vec![WireValue::Struct(vec![
                            ("serial".to_owned(), wv_str("7")),
                            ("amount".to_owned(), wv_str("100")),
                        ])]),
                    ),
                ],
            ),
            (
                "contract_main_call",
                vec![
                    ("marks".to_owned(), WireValue::Hex(vec![0, 0, 0])),
                    ("codeconf".to_owned(), wv_num(1)),
                    ("codes".to_owned(), WireValue::Hex(vec![0x01, 0x02, 0x03])),
                ],
            ),
            (
                "ast_select",
                vec![
                    ("exe_min".to_owned(), wv_num(1)),
                    ("exe_max".to_owned(), wv_num(1)),
                    (
                        "actions".to_owned(),
                        WireValue::List(vec![WireValue::Struct(vec![
                            ("kind".to_owned(), wv_str("transfer_hac_to")),
                            ("to".to_owned(), wv_str(MAIN)),
                            ("hacash".to_owned(), wv_str("12:244")),
                        ])]),
                    ),
                ],
            ),
        ];
        let expected_kinds = [
            protocol::action_std::BalanceFloor::KIND,
            vm::action::ContractMainCall::KIND,
            protocol::action_std::AstSelect::KIND,
        ];
        for ((kind, fields), expected) in cases.into_iter().zip(expected_kinds) {
            let built = build_transaction(&raw_spec(kind, fields)).expect(kind);
            let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
            assert_eq!(
                hex::encode(decoded.encode()),
                built.body,
                "{kind}: raw build must round-trip the tx body"
            );
            assert_eq!(
                decoded.actions()[0].kind(),
                expected,
                "{kind}: the protocol's own decoder must construct the native action"
            );
        }
    }

    #[test]
    fn raw_host_opcode_kind_is_outside_the_sdk_profile() {
        let error = build_transaction(&raw_spec("block_height", vec![])).unwrap_err();
        assert_eq!(error.code, "parse_failed");
    }

    #[test]
    fn golden_wire_specs_build() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden_seed.json");
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let mut vectors = 0usize;
        for (_, value) in field::json_split_object(&json).expect("golden object") {
            if !value.starts_with('[') {
                continue;
            }
            for vector in field::json_split_array(value).expect("vectors") {
                let mut name = String::new();
                let mut wire = String::new();
                for (key, v) in field::json_split_object(vector).expect("vector") {
                    match key {
                        "name" => {
                            name = field::json_expect_quoted_decoded(v)
                                .expect("name")
                                .to_owned()
                        }
                        "wire" => wire = v.to_owned(),
                        _ => {}
                    }
                }
                let spec = crate::spec_codec::decode_transaction_spec_json(&wire)
                    .unwrap_or_else(|e| panic!("{name}: JSON decode failed: {e}"));
                let built = build_transaction(&spec)
                    .unwrap_or_else(|e| panic!("{name}: rebuild failed: {e}"));
                let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
                assert_eq!(
                    hex::encode(decoded.encode()),
                    built.body,
                    "{name}: built body must round-trip"
                );
                vectors += 1;
            }
        }
        assert!(
            vectors >= 20,
            "expected a full golden vector set, got {vectors}"
        );
    }
}
