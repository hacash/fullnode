//! Operation dispatcher — binary ABI (§5).
//!
//! Single entry `sdk_invoke_binary(operation_id, payload)`:
//! - request: each operation has a fixed binary layout (simple params as W2
//!   strings/fixed-length numbers, complex objects as W4 length + JSON string,
//!   see each `route` branch);
//! - result: binary envelope `ok:u8 | W4 body_len + body` (body is the
//!   per-operation hand-written JSON, read by the TS side with native
//!   `JSON.parse`) or `ok:u8 | err_code:u16 + W2 err_msg`.
//!
//! Design A: amount/address/hex and other "semantic" fields are always
//! strings; parsing stays in Rust.
//!
//! Request parsing is driven by `profile::OP_DEFS` — the same single source
//! the JS facade is generated from — so a request-layout edit can never
//! desync the packer and the parser. Only `tx.build` keeps a hand-written
//! payload (the §4 TransactionSpec binary).

use std::sync::OnceLock;

use crate::error::{SdkError, SdkErrorCode};
use crate::profile::{
    capabilities, CodecProfile, OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY, OP_ACCOUNT_VERIFY_ADDRESS,
    OP_AMOUNT_FORMAT_PROTOCOL, OP_AMOUNT_PARSE_PROTOCOL, OP_MESSAGE_PREPARE_SIGNATURE,
    OP_MESSAGE_VERIFY, OP_POLICY_EVALUATE, OP_SYSTEM_CAPABILITIES, OP_SYSTEM_CODEC_PROFILE,
    OP_SYSTEM_SDK_VERSION, OP_TX_ATTACH_SIGNATURE, OP_TX_ATTACH_SIGNATURE_UNBOUND, OP_TX_BUILD,
    OP_TX_DECODE, OP_TX_ENCODE, OP_TX_INSPECT, OP_TX_INSPECT_REPORT, OP_TX_PREPARE_SIGNATURE,
    OP_TX_SIGNATURE_REPORT, OP_TX_VERIFY, OpRequestField, OP_DEFS,
};

pub(crate) fn profile() -> &'static CodecProfile {
    static PROFILE: OnceLock<CodecProfile> = OnceLock::new();
    PROFILE.get_or_init(CodecProfile::standard)
}

/// Transport version of the binary WASM surface (§5). Bumping this means the
/// binary envelope/payload semantics changed.
pub const TRANSPORT_VERSION: u32 = 3;
// v3: the error envelope adds a W2 detail (v2 had only code + message, which
// dropped the public semantics of `SdkError.detail`)

/// Error code → u16 (`SdkErrorCode::ERROR_CODES` order, stable ABI; 0 =
/// unknown).
pub fn error_code_id(code: &str) -> u16 {
    crate::error::ERROR_CODES
        .iter()
        .position(|c| *c == code)
        .map(|i| i as u16 + 1)
        .unwrap_or(0)
}

// ================================ binary request parsing ================================

struct ReqReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ReqReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], SdkError> {
        if self.pos + n > self.buf.len() {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request payload truncated",
            ));
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, SdkError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, SdkError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, SdkError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, SdkError> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn w2_str(&mut self) -> Result<String, SdkError> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "request string is not utf8"))
    }

    fn w4_json(&mut self) -> Result<String, SdkError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "request json is not utf8"))
    }

    fn opt_w2_str(&mut self) -> Result<Option<String>, SdkError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.w2_str()?)),
            _ => Err(SdkError::new(SdkErrorCode::ParseFailed, "invalid option marker")),
        }
    }

    fn opt_w4_json(&mut self) -> Result<Option<String>, SdkError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.w4_json()?)),
            _ => Err(SdkError::new(SdkErrorCode::ParseFailed, "invalid option marker")),
        }
    }

    fn require_done(&self) -> Result<(), SdkError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(SdkError::new(
                SdkErrorCode::TrailingBytes,
                "request payload has trailing bytes",
            ))
        }
    }
}

// ================================ binary result envelope ================================

fn encode_result_ok(body: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + body.len());
    out.push(1);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body.as_bytes());
    out
}

fn encode_result_err(error: &SdkError) -> Vec<u8> {
    let code = error_code_id(&error.code);
    let msg = error.message.as_bytes();
    let detail = error.detail.as_deref().map(str::as_bytes);
    let mut out = Vec::with_capacity(5 + msg.len() + detail.map_or(0, |d| d.len()));
    out.push(0);
    out.extend_from_slice(&code.to_be_bytes());
    out.extend_from_slice(&(msg.len() as u16).to_be_bytes());
    out.extend_from_slice(msg);
    // W2 detail: length 0 when there is no detail (keeps byte-compatible
    // reading with the v2 envelope)
    let detail = detail.unwrap_or(&[]);
    out.extend_from_slice(&(detail.len() as u16).to_be_bytes());
    out.extend_from_slice(detail);
    out
}

/// Binary envelope result (the ok branch's body is a hand-written JSON string).
pub fn invoke_binary(operation_id: u16, payload: &[u8]) -> Vec<u8> {
    match route(operation_id, payload) {
        Ok(body) => encode_result_ok(&body),
        Err(error) => encode_result_err(&error),
    }
}

// ================================ operation routing ================================

/// One parsed request value (shape per the `OpRequestField` that produced it).
enum ReqValue {
    Str(String),
    Json(String),
    OptStr(Option<String>),
    OptJson(Option<String>),
    OptU64(Option<u64>),
    U8(u8),
    OptInspectContext(Option<(u64, u32)>),
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
            ReqValue::Json(s) => Ok(s),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not a json string",
            )),
        }
    }

    fn opt_json(&self) -> Result<Option<&str>, SdkError> {
        match self {
            ReqValue::OptJson(value) => Ok(value.as_deref()),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an optional json string",
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

    fn inspect_context(&self) -> Result<Option<(u64, u32)>, SdkError> {
        match self {
            ReqValue::OptInspectContext(value) => Ok(*value),
            _ => Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "request field is not an inspect context",
            )),
        }
    }
}

/// Parse one operation's binary request strictly from its `OP_DEFS` layout —
/// the same single source the JS facade is generated from — so the packer and
/// the parser can never drift. Values are addressed by arg name, so a layout
/// reorder does not silently rewire the route arms either.
fn parse_request(operation_id: u16, payload: &[u8]) -> Result<Vec<(&'static str, ReqValue)>, SdkError> {
    let def = OP_DEFS.get(operation_id as usize - 1).ok_or_else(|| {
        SdkError::new(
            SdkErrorCode::UnknownOperation,
            format!("unknown operation id {}", operation_id),
        )
    })?;
    if def.request.is_empty() && !payload.is_empty() {
        return Err(SdkError::new(
            SdkErrorCode::UnknownField,
            "operation takes no payload",
        ));
    }
    let mut r = ReqReader::new(payload);
    let mut values = Vec::with_capacity(def.request.len());
    for field in def.request {
        let value = match field {
            OpRequestField::W2Str(_) => ReqValue::Str(r.w2_str()?),
            OpRequestField::OptW2Str(_) => ReqValue::OptStr(r.opt_w2_str()?),
            OpRequestField::W4Json(_) => ReqValue::Json(r.w4_json()?),
            OpRequestField::OptW4Json(_) => ReqValue::OptJson(r.opt_w4_json()?),
            OpRequestField::OptU64(_) => {
                let value = match r.u8()? {
                    0 => None,
                    1 => Some(r.u64()?),
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            "invalid option marker",
                        ))
                    }
                };
                ReqValue::OptU64(value)
            }
            OpRequestField::U8(_) => ReqValue::U8(r.u8()?),
            OpRequestField::OptInspectContext(_) => {
                let value = match r.u8()? {
                    0 => None,
                    1 => Some((r.u64()?, r.u32()?)),
                    _ => {
                        return Err(SdkError::new(
                            SdkErrorCode::ParseFailed,
                            "invalid inspect context marker",
                        ))
                    }
                };
                ReqValue::OptInspectContext(value)
            }
        };
        values.push((field.arg_name(), value));
    }
    r.require_done()?;
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

fn route(operation_id: u16, payload: &[u8]) -> Result<String, SdkError> {
    let profile = profile();
    match operation_id {
        OP_SYSTEM_CAPABILITIES => {
            parse_request(operation_id, payload)?;
            Ok(capabilities(profile).to_json_string())
        }
        OP_SYSTEM_SDK_VERSION => {
            parse_request(operation_id, payload)?;
            Ok(crate::json::obj(vec![
                crate::json::kv("schema", crate::json::q(crate::schema::SCHEMA_SDK_VERSION)),
                crate::json::kv("package_version", crate::json::q(crate::profile::SDK_VERSION)),
                crate::json::kv(
                    "abi",
                    crate::json::obj(vec![
                        crate::json::kv("major", crate::profile::ABI_MAJOR.to_string()),
                        crate::json::kv("minor", crate::profile::ABI_MINOR.to_string()),
                    ]),
                ),
            ]))
        }
        OP_SYSTEM_CODEC_PROFILE => {
            parse_request(operation_id, payload)?;
            Ok(profile.to_json_string())
        }
        OP_TX_BUILD => {
            let spec = crate::spec_codec::decode_transaction_spec_binary(payload)?;
            Ok(crate::build::build_transaction(&spec)?.to_json_string())
        }
        OP_TX_INSPECT_REPORT => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::inspect::inspect_report(
                req_field(&v, "body")?.str()?,
                req_field(&v, "signer_address")?.opt_str()?,
                profile,
            )?
            .to_json_string())
        }
        OP_TX_INSPECT => {
            let v = parse_request(operation_id, payload)?;
            let context = req_field(&v, "context")?
                .inspect_context()?
                .map(|(current_height, expected_chain_id)| crate::inspect::InspectContext {
                    current_height,
                    expected_chain_id,
                })
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
            )?
            .to_json_string())
        }
        OP_TX_PREPARE_SIGNATURE => {
            let v = parse_request(operation_id, payload)?;
            let review = match req_field(&v, "options.review")?.opt_json()? {
                Some(json) => Some(crate::inspect::Review::from_json(json)?),
                None => None,
            };
            let policy = match req_field(&v, "options.policy")?.opt_json()? {
                Some(json) => Some(crate::policy::Policy::from_json(json)?),
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
            let v = parse_request(operation_id, payload)?;
            let proof = crate::attach::SignatureProof::from_json(req_field(&v, "proof")?.json()?)?;
            let review = crate::inspect::Review::from_json(req_field(&v, "review")?.json()?)?;
            let request =
                crate::attach::SigningRequest::from_json(req_field(&v, "request")?.json()?)?;
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
            let v = parse_request(operation_id, payload)?;
            let proof = crate::attach::SignatureProof::from_json(req_field(&v, "proof")?.json()?)?;
            Ok(crate::attach::attach_signature_unbound(
                req_field(&v, "body")?.str()?,
                &proof,
                profile,
            )?
            .to_json_string())
        }
        OP_TX_VERIFY => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::attach::verify_signatures(req_field(&v, "body")?.str()?)?.to_json_string())
        }
        OP_TX_SIGNATURE_REPORT => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::attach::signature_report(req_field(&v, "body")?.str()?)?.to_json_string())
        }
        OP_TX_DECODE => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::inspect::decode_transaction_json(req_field(&v, "body")?.str()?)?
                .to_json_string())
        }
        OP_TX_ENCODE => {
            let v = parse_request(operation_id, payload)?;
            let transaction =
                crate::inspect::TransactionJson::from_json(req_field(&v, "transaction")?.json()?)?;
            let review = match req_field(&v, "review")?.opt_json()? {
                Some(json) => Some(crate::inspect::Review::from_json(json)?),
                None => None,
            };
            Ok(crate::inspect::encode_transaction_json(
                &transaction,
                review.as_ref(),
                profile,
            )?
            .to_json_string())
        }
        OP_ACCOUNT_VERIFY_ADDRESS => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::account::verify_address(req_field(&v, "address")?.str()?).to_json_string())
        }
        OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::account::address_from_public_key(req_field(&v, "public_key")?.str()?)?
                .to_json_string())
        }
        OP_AMOUNT_PARSE_PROTOCOL => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::amount::parse_protocol(req_field(&v, "value")?.str()?)?.to_json_string())
        }
        OP_AMOUNT_FORMAT_PROTOCOL => {
            let v = parse_request(operation_id, payload)?;
            Ok(crate::json::q(&crate::amount::format_protocol(
                req_field(&v, "value")?.str()?,
                req_field(&v, "unit")?.u8()?,
            )?))
        }
        OP_MESSAGE_PREPARE_SIGNATURE => {
            let v = parse_request(operation_id, payload)?;
            let params = crate::message::MessagePrepareParams::from_json(req_field(&v, "params")?.json()?)?;
            Ok(crate::message::prepare_message_signature(&params)?.to_json_string())
        }
        OP_MESSAGE_VERIFY => {
            let v = parse_request(operation_id, payload)?;
            let request = crate::attach::SigningRequest::from_json(req_field(&v, "request")?.json()?)?;
            let proof = crate::attach::SignatureProof::from_json(req_field(&v, "proof")?.json()?)?;
            Ok(crate::message::verify_message_signature(&request, &proof)?.to_json_string())
        }
        OP_POLICY_EVALUATE => {
            let v = parse_request(operation_id, payload)?;
            let review = crate::inspect::Review::from_json(req_field(&v, "review")?.json()?)?;
            let policy = match req_field(&v, "policy")?.opt_json()? {
                Some(json) => crate::policy::Policy::from_json(json)?,
                None => crate::policy::Policy::default(),
            };
            Ok(crate::policy::evaluate_policy(&review, &policy)?.to_json_string())
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

    fn invoke_ok(op: u16, payload: &[u8]) -> String {
        let response = invoke_binary(op, payload);
        if response[0] != 1 {
            let code = u16::from_be_bytes([response[1], response[2]]);
            let mlen = u16::from_be_bytes([response[3], response[4]]) as usize;
            let msg = String::from_utf8_lossy(&response[5..5 + mlen]).to_string();
            panic!("unexpected error envelope: code={code} msg={msg:?}");
        }
        let len = u32::from_be_bytes([
            response[1],
            response[2],
            response[3],
            response[4],
        ]) as usize;
        String::from_utf8(response[5..5 + len].to_vec()).unwrap()
    }

    #[test]
    fn system_operations() {
        let caps = invoke_ok(OP_SYSTEM_CAPABILITIES, &[]);
        assert!(caps.contains("\"schema\""));
        assert!(caps.contains("\"features\""));
        let version = invoke_ok(OP_SYSTEM_SDK_VERSION, &[]);
        assert!(version.contains("\"package_version\""));
        let profile = invoke_ok(OP_SYSTEM_CODEC_PROFILE, &[]);
        assert!(profile.contains("\"profile_hash\""));
    }

    #[test]
    fn error_envelope_shape() {
        let response = invoke_binary(9999, &[]);
        assert_eq!(response[0], 0);
        let code = u16::from_be_bytes([response[1], response[2]]);
        assert_eq!(code, 1); // UnknownOperation
    }

    #[test]
    fn build_with_binary_spec_roundtrip() {
        // Manually construct the same layout as codec.ts encodeTransactionSpec
        let mut payload = Vec::new();
        payload.push(2); // tx_type
        // main
        let main = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";
        payload.extend_from_slice(&(main.len() as u16).to_be_bytes());
        payload.extend_from_slice(main.as_bytes());
        let fee = "0.001";
        payload.extend_from_slice(&(fee.len() as u16).to_be_bytes());
        payload.extend_from_slice(fee.as_bytes());
        payload.extend_from_slice(&1700000000u64.to_be_bytes());
        payload.push(0); // gas_max
        payload.extend_from_slice(&1u16.to_be_bytes()); // action count
        payload.extend_from_slice(&1u16.to_be_bytes()); // kind transfer_hac_to
        // to: W2 string
        payload.extend_from_slice(&(main.len() as u16).to_be_bytes());
        payload.extend_from_slice(main.as_bytes());
        // hacash: W2 string
        let amount = "1.5";
        payload.extend_from_slice(&(amount.len() as u16).to_be_bytes());
        payload.extend_from_slice(amount.as_bytes());

        let body = invoke_ok(OP_TX_BUILD, &payload);
        assert!(body.contains("\"body\""), "unexpected: {body}");
        assert!(body.contains("\"tx_type\":2"), "unexpected: {body}");
    }

    #[test]
    fn binary_spec_rejects_trailing_bytes() {
        let response = invoke_binary(OP_TX_BUILD, &[0x02, 0x00, 0x00]);
        assert_eq!(response[0], 0);
        let code = u16::from_be_bytes([response[1], response[2]]);
        assert_eq!(code, error_code_id(SdkErrorCode::ParseFailed.as_str()));
    }
}

// ================================ cross-language consistency ================================
// The JS side cannot import Rust consts, so the operation/error tables and the
// operation methods live in the generated `js/generated/` artifacts (op_tables.mjs,
// operations.mjs), and the codec carries the schema hash. These tests parse the
// checked-in JS artifacts and lock them to the Rust side, so adding an
// operation/error/schema on one side without regenerating the other fails CI.

fn js_const_block<'a>(source: &'a str, marker: &str) -> Option<&'a str> {
    let start = source.find(marker)? + marker.len();
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    for (i, b) in bytes[start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source.get(start..start + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn js_const_array<'a>(source: &'a str, marker: &str) -> Option<&'a str> {
    let start = source.find(marker)? + marker.len();
    let rest = source.get(start..)?;
    let end = rest.find("];")?;
    source.get(start..start + end)
}

#[test]
fn generated_op_table_matches_rust() {
    use crate::profile::OPERATIONS;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/js/generated/op_tables.mjs");
    let js = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));

    let block = js_const_block(&js, "export const OP = {").expect("OP block");
    let mut entries: Vec<(String, u16)> = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() || line == "{" || line == "}" {
            continue;
        }
        let (key, value) = line.split_once(':').unwrap_or_else(|| panic!("bad OP entry {line:?}"));
        let key = key.trim().to_owned();
        let value: u16 = value
            .trim()
            .trim_end_matches(',')
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("bad OP value {line:?}"));
        entries.push((key, value));
    }

    assert_eq!(
        entries.len(),
        OPERATIONS.len(),
        "JS OP table size must match Rust OPERATIONS"
    );
    for (i, (key, value)) in entries.iter().enumerate() {
        assert_eq!(*value, i as u16 + 1, "JS OP ids are positional");
        let name = OPERATIONS[*value as usize - 1];
        // JS key = operation name uppercased with '.' replaced by '_'
        // (underscores already inside a name segment, e.g. `sdk_version`,
        // stay underscores).
        let expected_key = name.to_uppercase().replace('.', "_");
        assert_eq!(
            *key, expected_key,
            "JS OP key must be the uppercased operation name for {name}"
        );
    }
}

#[test]
fn generated_error_names_match_rust() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/js/generated/op_tables.mjs");
    let js = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));

    let block = js_const_array(&js, "export const ERROR_NAMES = [").expect("ERROR_NAMES array");
    let names: Vec<&str> = block
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim())
        .filter(|item| !item.is_empty() && *item != "null")
        .collect();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(
            error_code_id(name),
            i as u16 + 1,
            "JS ERROR_NAMES[{i}] must match error_code_id"
        );
    }
    // The last Rust code id must be covered (the Rust table has no gaps).
    assert!(error_code_id(names.last().expect("non-empty ERROR_NAMES")) > 0);
}

#[test]
fn generated_codec_hash_matches_rust_schema_set() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/js/generated/codec.mjs");
    let Ok(js) = std::fs::read_to_string(path) else {
        eprintln!("skip: {path} not generated (run codec-schema-gen / pack.sh)");
        return;
    };
    let hash_line = js
        .lines()
        .find(|line| line.starts_with("export const SCHEMA_HASH"))
        .unwrap_or_else(|| panic!("no SCHEMA_HASH in {path}"));
    let ts_hash = hash_line
        .split('"')
        .nth(1)
        .expect("SCHEMA_HASH literal")
        .to_owned();

    let schemas: Vec<base::ActionSchema> = crate::codec::standard_codecs()
        .expect("standard codecs assembly")
        .action_schemas()
        .to_vec();
    let structs = chain_codec::struct_schemas();
    let hash = base::schema_set_hash(&schemas, &structs);
    let rust_hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        rust_hash, ts_hash,
        "generated codec drifted from the Rust schema set (regenerate via codec-schema-gen / pack.sh)"
    );
}
