//! Message signing requests (Unified SDK 2.0, doc 14 §5, audit decision).
//!
//! Frozen convention: the caller-provided 32-byte digest is signed as-is
//! (no domain prefix, matching the current wallet/login ecosystem). The SDK
//! only prepares and verifies; signing happens in the wallet vault.

use field::Address;
use serde::{Deserialize, Serialize};

use crate::attach::{SignatureProof, SigningRequest};
use crate::error::{SdkError, SdkErrorCode};
use crate::schema::SCHEMA_SIGNING_REQUEST;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MessagePrepareParams {
    pub digest: String,
    pub signer_address: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageVerifyResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `message.prepare_signature`: build an authentication `SigningRequest` over
/// the given 32-byte digest. The digest convention is frozen: raw hash, no
/// prefix, no domain separation (doc 14 audit decision).
pub fn prepare_message_signature(
    params: &MessagePrepareParams,
) -> Result<SigningRequest, SdkError> {
    let digest: [u8; 32] = hex::decode(params.digest.trim_start_matches("0x").trim_start_matches("0X"))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            SdkError::with_detail(
                SdkErrorCode::ParseFailed,
                "message digest must be 32-byte hex",
                serde_json::json!({ "actual": params.digest }),
            )
        })?;
    let signer = Address::from_readable(&params.signer_address).map_err(SdkError::from)?;
    let mut request = SigningRequest {
        schema: SCHEMA_SIGNING_REQUEST.to_owned(),
        id: String::new(),
        purpose: "authentication".to_owned(),
        algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        signer_address: signer.to_readable(),
        digest: hex::encode(digest),
        body_hash: None,
        review_binding: None,
        policy_binding: None,
        origin: params.origin.clone(),
        expires_at: params.expires_at,
        request_binding: String::new(),
    };
    let binding = crate::attach::request_binding_of(&request);
    request.request_binding = binding.clone();
    request.id = binding;
    Ok(request)
}

/// `message.verify`: verify a proof over the digest and return the signer
/// address. The proof envelope (schema/algorithm), the proof's request
/// id/binding and the request expiry are all checked against the request.
pub fn verify_message_signature(
    request: &SigningRequest,
    proof: &SignatureProof,
) -> Result<MessageVerifyResult, SdkError> {
    if request.purpose != "authentication" {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedFeature,
            "message.verify requires an authentication request",
        ));
    }
    crate::attach::check_request_expiry(request)?;
    crate::attach::validate_proof_format(proof)?;
    let expected_binding = crate::attach::request_binding_of(request);
    if proof.request_binding != expected_binding || proof.request_id != request.id {
        return Err(SdkError::new(
            SdkErrorCode::ReviewBindingMismatch,
            "proof does not match the signing request",
        ));
    }
    let digest: [u8; 32] = hex::decode(&request.digest)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(SdkErrorCode::ParseFailed, "request digest invalid"))?;
    let public_key: [u8; 33] = hex::decode(&proof.public_key)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(SdkErrorCode::InvalidPublicKey, "public key must be 33-byte hex"))?;
    let signature: [u8; 64] = hex::decode(&proof.signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(SdkErrorCode::BadSignature, "signature must be 64-byte hex"))?;
    let address = Address::from(sys::Account::get_address_by_public_key(public_key));
    if address.to_readable() != request.signer_address {
        return Err(SdkError::with_detail(
            SdkErrorCode::BadSignature,
            "public key does not match the request signer address",
            serde_json::json!({ "actual": address.to_readable() }),
        ));
    }
    let ok = sys::Account::verify_signature(&digest, &public_key, &signature);
    Ok(MessageVerifyResult {
        ok,
        address: if ok { Some(address.to_readable()) } else { None },
        error: if ok { None } else { Some("signature verification failed".to_owned()) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_and_verifies_message_signature() {
        let account = sys::Account::create_by("123456").unwrap();
        let digest = hex::encode(sys::calculate_hash(b"challenge"));
        let request = prepare_message_signature(&MessagePrepareParams {
            digest: digest.clone(),
            signer_address: account.readable().to_owned(),
            origin: None,
            expires_at: None,
        })
        .unwrap();
        let signature: [u8; 64] = account.do_sign(&sys::calculate_hash(b"challenge"));
        let proof = SignatureProof {
            schema: crate::schema::SCHEMA_SIGNATURE_PROOF.to_owned(),
            request_id: request.id.clone(),
            request_binding: request.request_binding.clone(),
            public_key: hex::encode(account.public_key().serialize_compressed()),
            signature: hex::encode(signature),
            algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        };
        let result = verify_message_signature(&request, &proof).unwrap();
        assert!(result.ok);
        assert_eq!(result.address.unwrap(), account.readable());
        let _ = digest;
    }

    #[test]
    fn rejects_tampered_proof_binding() {
        let account = sys::Account::create_by("123456").unwrap();
        let request = prepare_message_signature(&MessagePrepareParams {
            digest: hex::encode(sys::calculate_hash(b"challenge")),
            signer_address: account.readable().to_owned(),
            origin: None,
            expires_at: None,
        })
        .unwrap();
        let proof = SignatureProof {
            schema: crate::schema::SCHEMA_SIGNATURE_PROOF.to_owned(),
            request_id: request.id.clone(),
            request_binding: "tampered".to_owned(),
            public_key: hex::encode(account.public_key().serialize_compressed()),
            signature: "00".repeat(64),
            algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        };
        let error = verify_message_signature(&request, &proof).unwrap_err();
        assert_eq!(error.code, "review_binding_mismatch");
    }

    #[test]
    fn rejects_wrong_proof_schema_and_algorithm() {
        let account = sys::Account::create_by("123456").unwrap();
        let request = prepare_message_signature(&MessagePrepareParams {
            digest: hex::encode(sys::calculate_hash(b"challenge")),
            signer_address: account.readable().to_owned(),
            origin: None,
            expires_at: None,
        })
        .unwrap();
        let signature: [u8; 64] = account.do_sign(&sys::calculate_hash(b"challenge"));
        let proof = SignatureProof {
            schema: "hacash.sdk/wrong@1".to_owned(),
            request_id: request.id.clone(),
            request_binding: request.request_binding.clone(),
            public_key: hex::encode(account.public_key().serialize_compressed()),
            signature: hex::encode(signature),
            algorithm: "not-a-real-algorithm".to_owned(),
        };
        let error = verify_message_signature(&request, &proof).unwrap_err();
        assert_eq!(error.code, "unsupported_schema");

        let mut fixed = proof;
        fixed.schema = crate::schema::SCHEMA_SIGNATURE_PROOF.to_owned();
        let error = verify_message_signature(&request, &fixed).unwrap_err();
        assert_eq!(error.code, "unsupported_feature");
    }

    #[test]
    fn rejects_expired_request() {
        let account = sys::Account::create_by("123456").unwrap();
        let request = prepare_message_signature(&MessagePrepareParams {
            digest: hex::encode(sys::calculate_hash(b"challenge")),
            signer_address: account.readable().to_owned(),
            origin: None,
            expires_at: Some(1), // long past
        })
        .unwrap();
        let signature: [u8; 64] = account.do_sign(&sys::calculate_hash(b"challenge"));
        let proof = SignatureProof {
            schema: crate::schema::SCHEMA_SIGNATURE_PROOF.to_owned(),
            request_id: request.id.clone(),
            request_binding: request.request_binding.clone(),
            public_key: hex::encode(account.public_key().serialize_compressed()),
            signature: hex::encode(signature),
            algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        };
        let error = verify_message_signature(&request, &proof).unwrap_err();
        assert_eq!(error.code, "request_expired");
    }
}
