//! Operation dispatcher — JSON boundary (§5): `sdk_invoke_json(operation_id, payload)` takes a
//! JSON request object → envelope `{"ok":1,"body":...}` / `{"ok":0,...}`.
//! Request parsing is driven by the SDK-local `profile::OPERATIONS` registry.

use std::sync::OnceLock;

use crate::error::{SdkError, SdkErrorCode};
use crate::json::{SdkJsonFrom, SdkJsonTo};
use crate::jsonparse;
use crate::profile::{
    CodecProfile, RequestField, OPERATIONS, OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY,
    OP_ACCOUNT_VERIFY_ADDRESS, OP_ACCOUNT_VERIFY_SIGNATURE, OP_ACTION_DESCRIBE, OP_AMOUNT_FORMAT,
    OP_AMOUNT_PARSE, OP_DIAMOND_LOOKUP, OP_MESSAGE_PREPARE_SIGNATURE, OP_MESSAGE_VERIFY,
    OP_POLICY_EVALUATE, OP_SYSTEM_PARAMS, OP_SYSTEM_SDK_VERSION, OP_TX_ATTACH_SIGNATURE,
    OP_TX_ATTACH_SIGNATURE_UNBOUND, OP_TX_BUILD, OP_TX_DECODE, OP_TX_ENCODE, OP_TX_ESTIMATE_FEE,
    OP_TX_INSPECT, OP_TX_INSPECT_REPORT, OP_TX_PREPARE_SIGNATURE, OP_TX_SIGNATURE_REPORT,
    OP_TX_VERIFY, OP_VM_CODE, OP_VM_DECODE_CALL,
};

pub(crate) fn profile() -> &'static CodecProfile {
    static PROFILE: OnceLock<CodecProfile> = OnceLock::new();
    PROFILE.get_or_init(CodecProfile::standard)
}

/// Transport version of the JSON WASM surface (§5). Bumping this means the
/// envelope/payload semantics changed.
pub const TRANSPORT_VERSION: u32 = 9;
// v9: dropped system.capabilities / system.codec_profile; renamed
//     amount.parse_protocol -> amount.parse and amount.format_protocol ->
//     amount.format (operation ids shifted).
// v8: JS facade forwards JSON as-is (no Number/BigInt or ActionSpec adapters).
// v7: every operation (incl. tx.build) uses a JSON request object. History:
// v6 moved the boundary to JSON; v5 was bjson field streams; v4 added guard facts; v3 added W2 detail.

/// Error code → u16 (`SdkErrorCode::ERROR_CODES` order, stable ABI; 0 =
/// unknown).
pub fn error_code_id(code: &str) -> u16 {
    crate::error::ERROR_CODES
        .iter()
        .position(|c| *c == code)
        .map(|i| i as u16 + 1)
        .unwrap_or(0)
}

// ================================ JSON result envelope ================================

fn encode_result_ok(body: &str) -> String {
    format!("{{\"ok\":1,\"body\":{body}}}")
}

fn encode_result_err(error: &SdkError) -> String {
    let code = error_code_id(&error.code);
    let detail = error.detail.as_deref().unwrap_or("");
    format!(
        "{{\"ok\":0,\"code\":{code},\"msg\":{},\"detail\":{}}}",
        field::json_escape(&error.message),
        field::json_escape(detail)
    )
}

/// JSON envelope result (the ok branch's body is the per-operation JSON
/// value produced by the boundary serializers).
pub fn invoke_json(operation_id: u16, payload: &[u8]) -> String {
    match route(operation_id, payload) {
        Ok(body) => encode_result_ok(&body),
        Err(error) => encode_result_err(&error),
    }
}

// ================================ JSON request parsing ================================

/// One parsed request value (shape per the `RequestField` that produced it).
enum ReqValue {
    Str(String),
    /// Raw JSON value slice of a complex object (the route decodes it with
    /// the typed boundary parser).
    Json(String),
    OptStr(Option<String>),
    OptJson(Option<String>),
    OptU64(Option<u64>),
    U8(u8),
    OptInspectContext(Option<crate::inspect::InspectContext>),
}

impl ReqValue {
    fn str(&self) -> Result<&str, SdkError> {
        match self {
            ReqValue::Str(s) => Ok(s),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not a string",
            )),
        }
    }

    fn opt_str(&self) -> Result<Option<&str>, SdkError> {
        match self {
            ReqValue::OptStr(value) => Ok(value.as_deref()),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an optional string",
            )),
        }
    }

    fn json(&self) -> Result<&str, SdkError> {
        match self {
            ReqValue::Json(raw) => Ok(raw),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not a JSON value",
            )),
        }
    }

    fn opt_json(&self) -> Result<Option<&str>, SdkError> {
        match self {
            ReqValue::OptJson(value) => Ok(value.as_deref()),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an optional JSON object",
            )),
        }
    }

    fn opt_u64(&self) -> Result<Option<u64>, SdkError> {
        match self {
            ReqValue::OptU64(value) => Ok(*value),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an optional u64",
            )),
        }
    }

    fn u8(&self) -> Result<u8, SdkError> {
        match self {
            ReqValue::U8(value) => Ok(*value),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not a u8",
            )),
        }
    }

    fn inspect_context(&self) -> Result<Option<crate::inspect::InspectContext>, SdkError> {
        match self {
            ReqValue::OptInspectContext(value) => Ok(value.clone()),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an inspect context",
            )),
        }
    }
}

/// Parse one operation's JSON request strictly from its `OPERATIONS` layout;
/// unknown fields and duplicated keys are rejected, recursively for nested objects.
fn parse_request_json(
    operation_id: u16,
    payload: &str,
) -> Result<Vec<(&'static str, ReqValue)>, SdkError> {
    let index = operation_id
        .checked_sub(1)
        .map(usize::from)
        .ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::UnknownOperation,
                format!("unknown operation id {}", operation_id),
            )
        })?;
    let operation = OPERATIONS.get(index).ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::UnknownOperation,
            format!("unknown operation id {}", operation_id),
        )
    })?;
    if operation.request.is_empty() {
        // Raw callers normally send an empty object; an empty payload is also
        // accepted for direct wasm-bindgen use.
        let trimmed = payload.trim();
        if !trimmed.is_empty() && trimmed != "{}" {
            return Err(SdkError::new(
                SdkErrorCode::UnknownField,
                "operation takes no payload",
            ));
        }
        return Ok(Vec::new());
    }
    // Field-level errors carry the operation name so raw callers can locate
    // the registry entry (`operation tx.build field spec missing`).
    let context = format!("operation {}", operation.name);
    let pairs = jsonparse::object_pairs(payload, &context)?;
    let allowed: Vec<&str> = operation.request.iter().map(|f| f.arg_name()).collect();
    jsonparse::reject_unknown(&pairs, &allowed, &context)?;
    let mut values = Vec::with_capacity(operation.request.len());
    for field in operation.request {
        let name = field.arg_name();
        let value = match field {
            RequestField::String(_) => {
                ReqValue::Str(jsonparse::required_string(&pairs, name, &context)?)
            }
            RequestField::OptionalString(_) => {
                ReqValue::OptStr(jsonparse::optional_string(&pairs, name, &context)?)
            }
            RequestField::Json(_) | RequestField::TransactionSpec(_) => {
                ReqValue::Json(jsonparse::required(&pairs, name, &context)?.to_owned())
            }
            RequestField::OptionalJson(_) => {
                ReqValue::OptJson(jsonparse::find(&pairs, name).map(str::to_owned))
            }
            RequestField::OptionalU64(_) => {
                ReqValue::OptU64(jsonparse::optional_number(&pairs, name, &context)?)
            }
            RequestField::U8(_) => {
                ReqValue::U8(jsonparse::required_number(&pairs, name, &context)?)
            }
            RequestField::OptionalInspectContext(_) => {
                let value = match jsonparse::find(&pairs, name) {
                    Some(raw) => Some(crate::inspect::InspectContext::from_json_str(raw).map_err(
                        |e| {
                            jsonparse::parse_failed(format!(
                                "{context} field {name} is not an inspect context: {e}"
                            ))
                        },
                    )?),
                    None => None,
                };
                ReqValue::OptInspectContext(value)
            }
        };
        values.push((name, value));
    }
    Ok(values)
}

fn req_field<'a>(
    values: &'a [(&'static str, ReqValue)],
    name: &str,
) -> Result<&'a ReqValue, SdkError> {
    values
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, value)| value)
        .ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("request field {name} missing"),
            )
        })
}

/// Parse the optional `describe` facet-control object (`{description, json,
/// code}` booleans); absent → all on.
fn describe_options(values: &[(&'static str, ReqValue)]) -> Result<crate::audit::DescribeOptions, SdkError> {
    match req_field(values, "describe")?.opt_json()? {
        Some(raw) => {
            let pairs = crate::jsonparse::object_pairs(&raw, "request field describe")?;
            crate::audit::DescribeOptions::from_json_pairs(&pairs)
        }
        None => Ok(crate::audit::DescribeOptions::default()),
    }
}

fn route(operation_id: u16, payload: &[u8]) -> Result<String, SdkError> {
    let text = std::str::from_utf8(payload)
        .map_err(|_| jsonparse::parse_failed("request payload is not UTF-8 JSON"))?;
    route_json(operation_id, text)
}

fn route_json(operation_id: u16, payload: &str) -> Result<String, SdkError> {
    let profile = profile();
    match operation_id {
        OP_SYSTEM_SDK_VERSION => {
            parse_request_json(operation_id, payload)?;
            Ok(crate::json::SdkVersion {
                schema: crate::schema::SCHEMA_SDK_VERSION.to_owned(),
                package_version: crate::profile::SDK_VERSION.to_owned(),
                abi: crate::profile::AbiVersion {
                    major: crate::profile::ABI_MAJOR,
                    minor: crate::profile::ABI_MINOR,
                },
            }
            .to_json_string())
        }
        OP_TX_BUILD => {
            let v = parse_request_json(operation_id, payload)?;
            let spec =
                crate::spec_codec::decode_transaction_spec_json(req_field(&v, "spec")?.json()?)?;
            Ok(crate::build::build_transaction(&spec)?.to_json_string())
        }
        OP_TX_INSPECT_REPORT => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::inspect::inspect_report(
                req_field(&v, "body")?.str()?,
                req_field(&v, "signer_address")?.opt_str()?,
                profile,
                &describe_options(&v)?,
            )?
            .to_json_string())
        }
        OP_TX_INSPECT => {
            let v = parse_request_json(operation_id, payload)?;
            let context = req_field(&v, "context")?
                .inspect_context()?
                .ok_or_else(|| {
                    SdkError::new(
                        SdkErrorCode::MissingInspectContext,
                        "strict inspect requires context.current_height and context.expected_chain_id",
                    )
                })?;
            Ok(crate::inspect::inspect(
                req_field(&v, "body")?.str()?,
                req_field(&v, "signer_address")?.opt_str()?,
                &context,
                profile,
                &describe_options(&v)?,
            )?
            .to_json_string())
        }
        OP_TX_PREPARE_SIGNATURE => {
            let v = parse_request_json(operation_id, payload)?;
            let review = match req_field(&v, "options.review")?.opt_json()? {
                Some(raw) => Some(crate::inspect::Review::from_json_str(raw)?),
                None => None,
            };
            let policy = match req_field(&v, "options.policy")?.opt_json()? {
                Some(raw) => Some(crate::policy::Policy::from_json_str(raw)?),
                None => None,
            };
            Ok(crate::attach::prepare_signature(
                req_field(&v, "body")?.str()?,
                req_field(&v, "signer_address")?.str()?,
                review.as_ref(),
                policy.as_ref(),
                req_field(&v, "options.origin")?.opt_str()?,
                req_field(&v, "options.expires_at")?.opt_u64()?,
                profile,
            )?
            .to_json_string())
        }
        OP_TX_ATTACH_SIGNATURE => {
            let v = parse_request_json(operation_id, payload)?;
            let proof =
                crate::attach::SignatureProof::from_json_str(req_field(&v, "proof")?.json()?)?;
            let review = crate::inspect::Review::from_json_str(req_field(&v, "review")?.json()?)?;
            let request =
                crate::attach::SigningRequest::from_json_str(req_field(&v, "request")?.json()?)?;
            Ok(crate::attach::attach_signature(
                req_field(&v, "body")?.str()?,
                &proof,
                &review,
                &request,
                profile,
            )?
            .to_json_string())
        }
        OP_TX_ATTACH_SIGNATURE_UNBOUND => {
            let v = parse_request_json(operation_id, payload)?;
            let proof =
                crate::attach::SignatureProof::from_json_str(req_field(&v, "proof")?.json()?)?;
            Ok(crate::attach::attach_signature_unbound(
                req_field(&v, "body")?.str()?,
                &proof,
                profile,
            )?
            .to_json_string())
        }
        OP_TX_VERIFY => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::attach::verify_signatures(req_field(&v, "body")?.str()?)?.to_json_string())
        }
        OP_TX_SIGNATURE_REPORT => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::attach::signature_report(req_field(&v, "body")?.str()?)?.to_json_string())
        }
        OP_TX_DECODE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(
                crate::inspect::decode_transaction_json(
                    req_field(&v, "body")?.str()?,
                    &describe_options(&v)?,
                )?
                .to_json_string(),
            )
        }
        OP_TX_ENCODE => {
            let v = parse_request_json(operation_id, payload)?;
            let transaction = crate::inspect::TransactionJson::from_json_str(
                req_field(&v, "transaction")?.json()?,
            )?;
            let review = match req_field(&v, "review")?.opt_json()? {
                Some(raw) => Some(crate::inspect::Review::from_json_str(raw)?),
                None => None,
            };
            Ok(
                crate::inspect::encode_transaction_json(&transaction, review.as_ref(), profile)?
                    .to_json_string(),
            )
        }
        OP_ACCOUNT_VERIFY_ADDRESS => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::account::verify_address(req_field(&v, "address")?.str()?).to_json_string())
        }
        OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(
                crate::account::address_from_public_key(req_field(&v, "public_key")?.str()?)?
                    .to_json_string(),
            )
        }
        OP_AMOUNT_PARSE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::amount::parse(req_field(&v, "value")?.str()?)?.to_json_string())
        }
        OP_AMOUNT_FORMAT => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::json::AmountFormatResult {
                value: crate::amount::format(
                    req_field(&v, "value")?.str()?,
                    req_field(&v, "unit")?.u8()?,
                )?,
            }
            .to_json_string())
        }
        OP_MESSAGE_PREPARE_SIGNATURE => {
            let v = parse_request_json(operation_id, payload)?;
            let params = crate::message::MessagePrepareParams::from_json_str(
                req_field(&v, "params")?.json()?,
            )?;
            Ok(crate::message::prepare_message_signature(&params)?.to_json_string())
        }
        OP_MESSAGE_VERIFY => {
            let v = parse_request_json(operation_id, payload)?;
            let request =
                crate::attach::SigningRequest::from_json_str(req_field(&v, "request")?.json()?)?;
            let proof =
                crate::attach::SignatureProof::from_json_str(req_field(&v, "proof")?.json()?)?;
            Ok(crate::message::verify_message_signature(&request, &proof)?.to_json_string())
        }
        OP_POLICY_EVALUATE => {
            let v = parse_request_json(operation_id, payload)?;
            let review = crate::inspect::Review::from_json_str(req_field(&v, "review")?.json()?)?;
            let policy = match req_field(&v, "policy")?.opt_json()? {
                Some(raw) => crate::policy::Policy::from_json_str(raw)?,
                None => crate::policy::Policy::default(),
            };
            Ok(crate::policy::evaluate_policy(&review, &policy)?.to_json_string())
        }
        OP_SYSTEM_PARAMS => {
            parse_request_json(operation_id, payload)?;
            Ok(crate::profile::params(profile).to_json_string())
        }
        OP_TX_ESTIMATE_FEE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::fee::estimate_fee(
                req_field(&v, "body")?.str()?,
                req_field(&v, "height")?.opt_u64()?,
            )?
            .to_json_string())
        }
        OP_ACCOUNT_VERIFY_SIGNATURE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::account::verify_signature(
                req_field(&v, "public_key")?.str()?,
                req_field(&v, "digest")?.str()?,
                req_field(&v, "signature")?.str()?,
            )?
            .to_json_string())
        }
        OP_DIAMOND_LOOKUP => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::diamond::lookup(
                req_field(&v, "name")?.opt_str()?,
                req_field(&v, "serial")?.opt_str()?,
            )?
            .to_json_string())
        }
        OP_VM_DECODE_CALL => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::vm::decode_call(req_field(&v, "action")?.str()?)?.to_json_string())
        }
        OP_ACTION_DESCRIBE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(
                crate::audit::describe_single(req_field(&v, "action")?.str()?, &describe_options(&v)?)?
                    .to_json_string(),
            )
        }
        OP_VM_CODE => {
            let v = parse_request_json(operation_id, payload)?;
            Ok(crate::vm::code(
                req_field(&v, "codes")?.str()?,
                req_field(&v, "code_type")?.str()?,
                req_field(&v, "format")?.opt_str()?.as_deref(),
                req_field(&v, "sourcemap")?.opt_json()?.as_deref(),
                req_field(&v, "limit")?.opt_u64()?,
                req_field(&v, "offset")?.opt_u64()?,
            )?
            .to_json_string())
        }
        _ => Err(SdkError::new(
            SdkErrorCode::UnknownOperation,
            format!("unknown operation id {}", operation_id),
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn invoke_ok(op: u16, payload: &str) -> String {
        invoke_ok_bytes(op, payload.as_bytes())
    }

    fn invoke_ok_bytes(op: u16, payload: &[u8]) -> String {
        let response = invoke_json(op, payload);
        let envelope: Vec<(&str, &str)> =
            field::json_split_object(&response).expect("envelope parses");
        let ok = envelope.iter().find(|(k, _)| *k == "ok").expect("ok field");
        if ok.1 != "1" {
            let code = envelope
                .iter()
                .find(|(k, _)| *k == "code")
                .map(|(_, v)| v.to_owned())
                .unwrap_or_default();
            let msg = envelope
                .iter()
                .find(|(k, _)| *k == "msg")
                .map(|(_, v)| field::json_expect_quoted_decoded(v).unwrap_or_default())
                .unwrap_or_default();
            panic!("unexpected error envelope: code={code} msg={msg:?}");
        }
        envelope
            .iter()
            .find(|(k, _)| *k == "body")
            .map(|(_, v)| v.to_owned())
            .unwrap_or_default()
            .to_owned()
    }

    fn field_names(json: &str) -> Vec<String> {
        field::json_split_object(json)
            .expect("body parses as JSON object")
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    #[test]
    fn system_operations() {
        let version = field_names(&invoke_ok(OP_SYSTEM_SDK_VERSION, ""));
        assert!(version.iter().any(|n| n == "package_version"));
        assert!(version.iter().any(|n| n == "abi"));
    }

    #[test]
    fn error_envelope_shape() {
        let response = invoke_json(9999, b"");
        let envelope: Vec<(&str, &str)> =
            field::json_split_object(&response).expect("envelope parses");
        let ok = envelope.iter().find(|(k, _)| *k == "ok").unwrap().1;
        assert_eq!(ok, "0");
        let code = envelope
            .iter()
            .find(|(k, _)| *k == "code")
            .unwrap()
            .1
            .trim_matches('"');
        assert_eq!(code, "1"); // UnknownOperation
    }

    #[test]
    fn zero_operation_id_is_an_error_not_an_index_underflow() {
        let error = match parse_request_json(0, "{}") {
            Ok(_) => panic!("operation id zero must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, SdkErrorCode::UnknownOperation.as_str());
    }

    /// Every operation id must reach a real dispatch arm: routed operations
    /// never answer with `unknown_operation`, even on an empty payload.
    #[test]
    fn every_operation_id_routes() {
        for id in 1..=crate::profile::OPERATIONS.len() as u16 {
            let response = invoke_json(id, b"{}");
            let envelope: Vec<(&str, &str)> =
                field::json_split_object(&response).expect("envelope parses");
            let ok = envelope.iter().find(|(k, _)| *k == "ok").unwrap().1;
            if ok == "0" {
                let code = envelope
                    .iter()
                    .find(|(k, _)| *k == "code")
                    .map(|(_, v)| v.trim_matches('"'))
                    .unwrap();
                assert_ne!(
                    code, "1",
                    "operation id {id} is not routed (fell into the unknown_operation catch-all)"
                );
            }
        }
    }

    /// Operation names are the public interface: unique, non-empty, and
    /// `domain.name` shaped; ids are the sequential registry position (1-based).
    #[test]
    fn operation_names_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for (index, operation) in crate::profile::OPERATIONS.iter().enumerate() {
            assert!(
                seen.insert(operation.name),
                "duplicate operation name {}",
                operation.name
            );
            let mut parts = operation.name.split('.');
            let domain = parts.next().expect("operation name has a domain");
            let name = parts.next().expect("operation name has a name");
            assert!(
                !domain.is_empty() && !name.is_empty() && parts.next().is_none(),
                "operation name {} is not `domain.name` shaped",
                operation.name
            );
            // Pin the 1-based sequential numbering the dispatcher relies on.
            assert_eq!(u16::try_from(index + 1).unwrap(), id_from_name(operation.name));
        }
    }

    fn id_from_name(name: &str) -> u16 {
        crate::profile::OPERATIONS
            .iter()
            .position(|operation| operation.name == name)
            .map(|index| index as u16 + 1)
            .unwrap_or(0)
    }

    #[test]
    fn build_with_json_spec_roundtrip() {
        let main = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";
        let payload = format!(
            r#"{{"spec":{{"schema":"{}","tx_type":"2","main":"{main}","fee":"0.001","timestamp":"1700000000","actions":[{{"kind":"transfer_hac_to","to":"{main}","hacash":"1.5"}}]}}}}"#,
            crate::schema::SCHEMA_TRANSACTION_SPEC
        );
        let body = invoke_ok(OP_TX_BUILD, &payload);
        let pairs = field::json_split_object(&body).expect("build body parses");
        assert!(
            pairs.iter().any(|(n, _)| *n == "body"),
            "body field missing from tx.build response"
        );
        let tx_type = pairs
            .iter()
            .find(|(k, _)| *k == "tx_type")
            .map(|(_, v)| v.trim_matches('"'))
            .unwrap();
        assert_eq!(tx_type, "2");
    }

    #[test]
    fn json_request_rejects_trailing_and_unknown_fields() {
        // Unknown request field is rejected at the JSON boundary.
        let response = invoke_json(
            OP_ACCOUNT_VERIFY_ADDRESS,
            br#"{"address":"1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr","bogus":"x"}"#,
        );
        assert!(response.contains("\"ok\":0"));
        // Non-object payload is rejected.
        let response = invoke_json(OP_ACCOUNT_VERIFY_ADDRESS, br#""just a string""#);
        assert!(response.contains("\"ok\":0"));
    }
}
