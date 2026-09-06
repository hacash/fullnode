//! `tx.build`: construct unsigned Type-2/3 bodies from a protocol-JSON action spec.
//! Each action object is decoded by the registry; kinds outside the SDK codec
//! profile are rejected.

use base::{JsonCodecs, TxCreateRequest};
use field::{AddrOrList, Address, Amount};

use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::decode_tx;
use crate::schema::{SCHEMA_BUILT_TRANSACTION, SCHEMA_TRANSACTION_SPEC};

/// One protocol action object, stored as its original JSON text plus the
/// cached numeric `kind` extracted when the spec is parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionSpec {
    pub json: String,
    pub kind: u16,
}

impl ActionSpec {
    pub fn new(json: impl Into<String>) -> Result<Self, SdkError> {
        let json = json.into();
        let kind = parse_action_kind(&json)?;
        Ok(Self { json, kind })
    }
}

fn parse_action_kind(json: &str) -> Result<u16, SdkError> {
    let pairs = field::json_object_pairs(json, "action").map_err(SdkError::from)?;
    let raw = pairs
        .iter()
        .find(|(key, _)| *key == "kind")
        .map(|(_, value)| *value)
        .ok_or_else(|| {
            SdkError::new(SdkErrorCode::ParseFailed, "action is missing numeric kind")
        })?;
    // Same semantics as the registry's `action_kind_from_json`, which decodes
    // the very JSON this gate feeds: a numeric id, or a registered action name
    // (quoted or bare). The spec gate must not be stricter than the codec.
    let name = if let Ok(text) = field::json_expect_unquoted(raw) {
        if let Ok(kind) = text.parse::<u16>() {
            return Ok(kind);
        }
        text.trim().to_owned()
    } else {
        field::json_expect_quoted_decoded(raw)
            .map(|decoded| decoded.trim().to_owned())
            .map_err(|_| {
                SdkError::new(
                    SdkErrorCode::ParseFailed,
                    "action kind must be a number or its registered name",
                )
            })?
    };
    crate::selection::action_schema_named(&name)
        .map(|schema| schema.kind)
        .ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("unknown action kind name {name:?}"),
            )
        })
}

#[derive(Debug, Clone)]
pub struct TransactionSpec {
    pub schema: Option<String>,
    pub tx_type: u8,
    pub main: String,
    pub fee: String,
    pub timestamp: Option<u64>,
    pub gas_max: Option<u8>,
    pub addrlist: Option<AddrOrList>,
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

    let mut request = TxCreateRequest::new(spec.tx_type, main, fee, timestamp)
        .with_gas_max(spec.gas_max.unwrap_or(0));
    if let Some(addrlist) = spec.addrlist.clone() {
        let primary = addrlist.to_list().into_iter().next();
        if primary.as_ref() != Some(&main) {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "transaction spec main must equal addrlist primary address",
            ));
        }
        request = request.with_addrlist(addrlist);
    }

    let codecs = crate::codec::standard_codecs().map_err(SdkError::from)?;
    let mut actions = Vec::with_capacity(spec.actions.len());
    for (i, action) in spec.actions.iter().enumerate() {
        actions.push(build_action(i, action, codecs)?);
    }

    let body =
        protocol::tx_std::encode_standard_tx(request, &actions, &[]).map_err(SdkError::from)?;

    let body_hex = hex::encode(&body);
    let decoded = decode_tx(&body)?;
    let re_encoded = decoded.encode();
    if re_encoded != body {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "built body failed the encode(decode(body)) == body round-trip",
        ));
    }
    decoded.req_sign().map_err(SdkError::from)?;
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

fn build_action(
    index: usize,
    spec: &ActionSpec,
    codecs: &impl JsonCodecs,
) -> Result<base::ActionRef, SdkError> {
    codecs.decode_action_json(&spec.json).map_err(|error| {
        SdkError::new(
            SdkErrorCode::ParseFailed,
            format!("actions[{index}] kind {}: {error}", spec.kind),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::{AddrOrList, Address, ToJSON};

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn action_json(json: impl Into<String>) -> ActionSpec {
        ActionSpec::new(json.into()).expect("test action spec")
    }

    fn hac_to(to: &str, amount: &str) -> ActionSpec {
        action_json(format!(r#"{{"kind":1,"to":"{to}","hacash":"{amount}"}}"#))
    }

    fn sample_spec() -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 2,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            addrlist: None,
            actions: vec![
                hac_to(MAIN, "12:244"),
                action_json(r#"{"kind":1042,"start":1000000,"end":0}"#),
            ],
        }
    }

    fn raw_spec(json: &str) -> TransactionSpec {
        TransactionSpec {
            schema: Some(SCHEMA_TRANSACTION_SPEC.to_owned()),
            tx_type: 3,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            addrlist: None,
            actions: vec![ActionSpec::new(json.to_owned()).expect("test action spec")],
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

    /// The spec gate accepts the registered action name wherever it accepts the
    /// numeric id — both must produce byte-identical bodies.
    #[test]
    fn name_kind_specs_build_like_numeric_ones() {
        let named = raw_spec(&format!(
            r#"{{"kind":"transfer_hacd_to","to":"{MAIN}","diamonds":"WMEKBS"}}"#
        ));
        let numeric = raw_spec(&format!(
            r#"{{"kind":7,"to":"{MAIN}","diamonds":"WMEKBS"}}"#
        ));
        assert_eq!(named.actions[0].kind, numeric.actions[0].kind);
        let a = build_transaction(&named).unwrap();
        let b = build_transaction(&numeric).unwrap();
        assert_eq!(a.body, b.body, "name and numeric kind must build identical bodies");
    }

    #[test]
    fn unknown_action_kind_name_is_rejected() {
        let error = ActionSpec::new(r#"{"kind":"no_such_action","to":"x"}"#.to_owned())
            .unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(
            error.message.contains("unknown action kind name"),
            "{}",
            error.message
        );
    }

    #[test]
    fn delegates_unknown_tx_type_to_protocol_constructor() {
        let mut spec = sample_spec();
        spec.tx_type = 0;
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(
            error
                .message
                .contains("unsupported standard user transaction type 0")
        );
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
            &crate::audit::DescribeOptions::default(),
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
        spec.actions[0] = action_json(format!(
            r#"{{"kind":14,"from":"{}","to":"{MAIN}","hacash":"12:244"}}"#,
            other.readable()
        ));
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::TransferHacFromTo::KIND
        );
        let required = decoded.req_sign().unwrap();
        let other_address = Address::from_readable(other.readable()).unwrap();
        assert!(required.contains(&other_address));
    }

    #[test]
    fn explicit_from_equal_to_main_keeps_the_from_to_form() {
        let mut spec = sample_spec();
        spec.actions[0] = action_json(format!(
            r#"{{"kind":14,"from":"{MAIN}","to":"{MAIN}","hacash":"12:244"}}"#
        ));
        let built = build_transaction(&spec).unwrap();
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::TransferHacFromTo::KIND,
            "an explicit from equal to main must keep the from_to wire form (the SDK never rewrites it)"
        );
    }

    #[test]
    fn raw_actions_build_through_the_protocol_codec() {
        let floor = protocol::action_std::BalanceFloor::new(
            field::AddrOrPtr::Addr(Address::from_readable(MAIN).unwrap()),
            Amount::from("12:244").unwrap(),
            field::Satoshi::from(100),
            field::DiamondNumber::from(5),
        );
        let mut floor = floor;
        floor.assets = field::AssetAmtW1::from(vec![field::AssetAmt {
            serial: field::Fold64::from(7).unwrap(),
            amount: field::Fold64::from(100).unwrap(),
        }])
        .unwrap();
        let mut maincall = vm::action::ContractMainCall::new();
        maincall.codeconf = field::Uint1::from(1);
        maincall.codes = field::BytesW2::from(vec![0x01, 0x02, 0x03]).unwrap();
        let child = protocol::action_std::TransferHacTo::new(
            Address::from_readable(MAIN).unwrap(),
            Amount::from("12:244").unwrap(),
        );
        let ast =
            protocol::action_std::AstSelect::create_by(1, 1, vec![std::sync::Arc::new(child)])
                .unwrap();
        let cases = [floor.to_json(), maincall.to_json(), ast.to_json()];
        let expected_kinds = [
            protocol::action_std::BalanceFloor::KIND,
            vm::action::ContractMainCall::KIND,
            protocol::action_std::AstSelect::KIND,
        ];
        for (json, expected) in cases.iter().zip(expected_kinds) {
            let built = build_transaction(&raw_spec(json)).expect(json);
            let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
            assert_eq!(
                hex::encode(decoded.encode()),
                built.body,
                "{json}: raw build must round-trip the tx body"
            );
            assert_eq!(
                decoded.actions()[0].kind(),
                expected,
                "{json}: the protocol's own decoder must construct the native action"
            );
        }
    }

    #[test]
    fn raw_host_opcode_kind_is_outside_the_sdk_profile() {
        let error = build_transaction(&raw_spec(r#"{"kind":1793}"#)).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(
            error.message.starts_with("actions[0] kind 1793:"),
            "{}",
            error.message
        );
    }

    #[test]
    fn addrlist_resolves_pointer_actions_and_omitting_it_fails() {
        let extra = sys::Account::create_by("654321").unwrap();
        let main_addr = Address::from_readable(MAIN).unwrap();
        let extra_addr = Address::from_readable(extra.readable()).unwrap();
        let addrlist = AddrOrList::from_list(vec![main_addr, extra_addr]).unwrap();
        let pointer_from = action_json(r#"{"kind":13,"from":1,"hacash":"12:244"}"#);
        let mut spec = sample_spec();
        spec.actions = vec![pointer_from.clone()];
        spec.addrlist = Some(addrlist);
        let built = build_transaction(&spec).expect("pointer with addrlist");
        let decoded = decode_tx(&hex::decode(&built.body).unwrap()).unwrap();
        assert_eq!(
            decoded.actions()[0].kind(),
            protocol::action_std::TransferHacFrom::KIND
        );

        spec.addrlist = None;
        let error = build_transaction(&spec).unwrap_err();
        assert_eq!(error.code, "parse_failed");
        assert!(
            error.message.contains("addr ptr") || error.message.contains("out of range"),
            "{error:?}"
        );
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
