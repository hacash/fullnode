//! Operation dispatcher (Unified SDK 2.0, doc 14 §8/§9). Every operation
//! receives one JSON object and returns a `ResultEnvelope`; the raw transport
//! is a single `sdk_invoke` function so new operations never change the
//! WASM surface.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::{SdkError, SdkErrorCode};
use crate::profile::{capabilities, CodecProfile};
use crate::schema::ResultEnvelope;

pub(crate) fn profile() -> &'static CodecProfile {
    static PROFILE: OnceLock<CodecProfile> = OnceLock::new();
    PROFILE.get_or_init(CodecProfile::standard)
}

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    pub operation: String,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// One-shot dispatch: parse → route → envelope. Never panics on caller input.
pub fn invoke(request_json: &str) -> String {
    let envelope: ResultEnvelope<serde_json::Value> = dispatch(request_json);
    serde_json::to_string(&envelope).unwrap_or_else(|_| {
        let error = SdkError::new(
            SdkErrorCode::ParseFailed,
            "failed to serialize dispatcher response",
        );
        serde_json::to_string(&ResultEnvelope::<serde_json::Value>::err(error))
            .unwrap_or_else(|_| {
            r#"{"ok":false,"error":{"schema":"hacash.sdk/error@1","code":"parse_failed","message":"response serialization failed"}}"#.to_owned()
        })
    })
}

fn dispatch(request_json: &str) -> ResultEnvelope<serde_json::Value> {
    let request: InvokeRequest = match serde_json::from_str(request_json) {
        Ok(request) => request,
        Err(error) => return ResultEnvelope::err(SdkError::from(error)),
    };
    let payload = request.payload;
    let result = route(&request.operation, payload);
    match result {
        Ok(value) => ResultEnvelope::ok(value),
        Err(error) => ResultEnvelope::err(error),
    }
}

fn route(operation: &str, payload: serde_json::Value) -> Result<serde_json::Value, SdkError> {
    let profile = profile();
    match operation {
        "system.capabilities" => serde_json::to_value(capabilities(profile)).map_err(SdkError::from),
        "system.sdk_version" => serde_json::to_value(serde_json::json!({
            "schema": crate::schema::SCHEMA_SDK_VERSION,
            "package_version": crate::profile::SDK_VERSION,
            "abi": { "major": crate::profile::ABI_MAJOR, "minor": crate::profile::ABI_MINOR },
        }))
        .map_err(SdkError::from),
        "system.codec_profile" => serde_json::to_value(profile).map_err(SdkError::from),
        "tx.build" => {
            #[derive(Deserialize)]
            struct Params {
                spec: crate::build::TransactionSpec,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::build::build_transaction(&params.spec)?)
                .map_err(SdkError::from)
        }
        "tx.inspect_report" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
                #[serde(default)]
                signer_address: Option<String>,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::inspect::inspect_report(
                &params.body,
                params.signer_address.as_deref(),
                profile,
            )?)
            .map_err(SdkError::from)
        }
        "tx.inspect" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
                #[serde(default)]
                signer_address: Option<String>,
                #[serde(default)]
                context: Option<crate::inspect::InspectContext>,
            }
            let params: Params = parse_payload(payload)?;
            let context = params.context.ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::MissingInspectContext,
                    "strict inspect requires context.current_height and context.expected_chain_id",
                )
            })?;
            serde_json::to_value(crate::inspect::inspect(
                &params.body,
                params.signer_address.as_deref(),
                &context,
                profile,
            )?)
            .map_err(SdkError::from)
        }
        "tx.prepare_signature" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
                signer_address: String,
                #[serde(default)]
                review: Option<crate::inspect::Review>,
                #[serde(default)]
                policy_binding: Option<String>,
                #[serde(default)]
                origin: Option<String>,
                #[serde(default)]
                expires_at: Option<u64>,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::attach::prepare_signature(
                &params.body,
                &params.signer_address,
                params.review.as_ref(),
                params.policy_binding.as_deref(),
                params.origin.as_deref(),
                params.expires_at,
                profile,
            )?)
            .map_err(SdkError::from)
        }
        "tx.attach_signature" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
                proof: crate::attach::SignatureProof,
                #[serde(default)]
                review: Option<crate::inspect::Review>,
                #[serde(default)]
                request: Option<crate::attach::SigningRequest>,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::attach::attach_signature(
                &params.body,
                &params.proof,
                params.review.as_ref(),
                params.request.as_ref(),
                profile,
            )?)
            .map_err(SdkError::from)
        }
        "tx.verify" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::attach::verify_signatures(&params.body)?)
                .map_err(SdkError::from)
        }
        "tx.signature_report" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::attach::signature_report(&params.body)?)
                .map_err(SdkError::from)
        }
        "tx.decode" => {
            #[derive(Deserialize)]
            struct Params {
                body: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::inspect::decode_transaction_json(&params.body)?)
                .map_err(SdkError::from)
        }
        "tx.encode" => {
            #[derive(Deserialize)]
            struct Params {
                transaction: crate::inspect::TransactionJson,
                #[serde(default)]
                review: Option<crate::inspect::Review>,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::inspect::encode_transaction_json(
                &params.transaction,
                params.review.as_ref(),
                profile,
            )?)
            .map_err(SdkError::from)
        }
        "account.verify_address" => {
            #[derive(Deserialize)]
            struct Params {
                address: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::account::verify_address(&params.address))
                .map_err(SdkError::from)
        }
        "account.address_from_public_key" => {
            #[derive(Deserialize)]
            struct Params {
                public_key: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::account::address_from_public_key(&params.public_key)?)
                .map_err(SdkError::from)
        }
        "amount.parse_protocol" => {
            #[derive(Deserialize)]
            struct Params {
                value: String,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::amount::parse_protocol(&params.value)?)
                .map_err(SdkError::from)
        }
        "amount.format_protocol" => {
            #[derive(Deserialize)]
            struct Params {
                value: String,
                unit: u8,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::amount::format_protocol(&params.value, params.unit)?)
                .map_err(SdkError::from)
        }
        "message.prepare_signature" => {
            let params: crate::message::MessagePrepareParams = parse_payload(payload)?;
            serde_json::to_value(crate::message::prepare_message_signature(&params)?)
                .map_err(SdkError::from)
        }
        "message.verify" => {
            #[derive(Deserialize)]
            struct Params {
                request: crate::attach::SigningRequest,
                proof: crate::attach::SignatureProof,
            }
            let params: Params = parse_payload(payload)?;
            serde_json::to_value(crate::message::verify_message_signature(
                &params.request,
                &params.proof,
            )?)
            .map_err(SdkError::from)
        }
        "policy.evaluate" => {
            #[derive(Deserialize)]
            struct Params {
                review: crate::inspect::Review,
                #[serde(default)]
                policy: Option<crate::policy::Policy>,
            }
            let params: Params = parse_payload(payload)?;
            let policy = params.policy.unwrap_or_default();
            serde_json::to_value(crate::policy::evaluate_policy(&params.review, &policy)?)
                .map_err(SdkError::from)
        }
        _ => Err(SdkError::with_detail(
            SdkErrorCode::UnknownOperation,
            format!("unknown operation {operation:?}"),
            serde_json::json!({ "actual": operation }),
        )),
    }
}

fn parse_payload<T: serde::de::DeserializeOwned>(payload: serde_json::Value) -> Result<T, SdkError> {
    serde_json::from_value(payload).map_err(|error| SdkError::from(error))
}

/// Transport version of the raw WASM surface (doc 14 §9).
pub const TRANSPORT_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke_ok(request: &str) -> serde_json::Value {
        let response = invoke(request);
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["ok"], true, "unexpected error envelope: {value}");
        value["value"].clone()
    }

    #[test]
    fn dispatcher_routes_core_operations() {
        let value = invoke_ok(r#"{"operation":"system.capabilities","payload":{}}"#);
        assert_eq!(value["abi"]["major"], 2);

        let value = invoke_ok(r#"{"operation":"system.codec_profile","payload":{}}"#);
        assert_eq!(value["fullnode_commit"], crate::profile::FULLNODE_COMMIT);
        assert!(value["registered_kinds"].as_array().unwrap().len() > 30);
        assert!(!value["profile_hash"].as_str().unwrap().is_empty());

        let value = invoke_ok(
            r#"{"operation":"account.verify_address","payload":{"address":"1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9"}}"#,
        );
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn unknown_operation_returns_envelope_error() {
        let response = invoke(r#"{"operation":"nope","payload":{}}"#);
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "unknown_operation");
    }

    #[test]
    fn malformed_request_returns_envelope_error() {
        let response = invoke("not json");
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "parse_failed");
    }
}
