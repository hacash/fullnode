//! Signing state machine (Unified SDK 2.0, doc 14 §4.5/§4.9): the SDK computes
//! sign hashes and consumes `SignatureProof`s; private keys never cross the
//! boundary. `attach` clones first, validates everything, then commits.

use base::Transaction;
use field::{Address, Sign};
use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::{decode_body_hex, decode_tx};
use crate::profile::CodecProfile;
use crate::schema::{
    DOMAIN_SIGNING_REQUEST, SCHEMA_ATTACH_RESULT, SCHEMA_SIGNATURE_PROOF,
    SCHEMA_SIGNATURE_REPORT, SCHEMA_SIGNING_REQUEST, SCHEMA_VERIFY_RESULT,
};

/// Structured signing request produced by `prepare_signature` (doc 14 §4.9).
/// The vault signs `digest` and returns a `SignatureProof`; nothing in here
/// is secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningRequest {
    pub schema: String,
    pub id: String,
    pub purpose: String,
    pub algorithm: String,
    pub signer_address: String,
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_binding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_binding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub request_binding: String,
}

/// External signer output (doc 14 §4.9, frozen schema). No secret fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignatureProof {
    pub schema: String,
    pub request_id: String,
    pub request_binding: String,
    pub public_key: String,
    pub signature: String,
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachResult {
    pub schema: String,
    pub body: String,
    pub complete: bool,
    pub missing_signers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub schema: String,
    pub ok: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureReport {
    pub schema: String,
    pub required: Vec<String>,
    pub present: Vec<String>,
    pub valid: Vec<String>,
    pub missing: Vec<String>,
    pub invalid: Vec<String>,
}

/// Domain-separated binding of a signing request (sha3-256 over the canonical
/// request JSON minus `request_binding` and `id`, which is derived from the
/// binding itself). Used by prepare paths and by proof verification.
pub fn request_binding_of(request: &SigningRequest) -> String {
    let mut copy = request.clone();
    copy.request_binding.clear();
    copy.id.clear();
    let json = serde_json::to_string(&copy).unwrap_or_default();
    let mut data = Vec::with_capacity(DOMAIN_SIGNING_REQUEST.len() + json.len());
    data.extend_from_slice(DOMAIN_SIGNING_REQUEST);
    data.extend_from_slice(json.as_bytes());
    hex::encode(sys::calculate_hash(data))
}

/// `tx.prepare_signature`: re-validates the body and (when provided) the
/// review/policy bindings, computes the local sign hash for `signer_address`
/// and returns a `SigningRequest`. The SDK never signs.
///
/// A provided `Review` must fully re-verify: its binding is recomputed from
/// the body, signer, profile and the review's own content, so a review whose
/// displayed fields (fee, amounts, actions, context) were edited after
/// `inspect` is rejected here instead of being carried into the request.
pub fn prepare_signature(
    body_hex: &str,
    signer_address: &str,
    review: Option<&crate::inspect::Review>,
    policy_binding: Option<&str>,
    origin: Option<&str>,
    expires_at: Option<u64>,
    profile: &CodecProfile,
) -> Result<SigningRequest, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    if tx.ty() == 1 {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedTxType,
            "type 1 transactions cannot be signed",
        ));
    }
    let signer = Address::from_readable(signer_address).map_err(SdkError::from)?;
    let unsigned_body_hash = crate::audit::unsigned_body_hash(body_hex)?;
    if let Some(review) = review {
        verify_review(review, &unsigned_body_hash, &signer, tx.as_ref(), profile)?;
    }
    if policy_binding.is_some() && review.is_none() {
        return Err(SdkError::new(
            SdkErrorCode::PolicyBindingMismatch,
            "policy_binding requires a review",
        ));
    }
    let digest = hex::encode(protocol::tx_std::sign_hash_for(tx.as_ref(), &signer).0);
    let mut request = SigningRequest {
        schema: SCHEMA_SIGNING_REQUEST.to_owned(),
        id: String::new(),
        purpose: "transaction".to_owned(),
        algorithm: "secp256k1-rfc6979-sha256".to_owned(),
        signer_address: signer.to_readable(),
        digest,
        body_hash: Some(unsigned_body_hash),
        review_binding: review.map(|review| review.review_binding.clone()),
        policy_binding: policy_binding.map(str::to_owned),
        origin: origin.map(str::to_owned),
        expires_at,
        request_binding: String::new(),
    };
    let binding = request_binding_of(&request);
    request.request_binding = binding.clone();
    request.id = binding;
    Ok(request)
}

/// Shared approval-context verification: the review must cover this body,
/// this signer, this profile, and its binding must recompute from its own
/// content (see `crate::inspect::review_binding_of`). Used by
/// `prepare_signature`, `attach_signature` and `encode_transaction_json`.
pub fn verify_review(
    review: &crate::inspect::Review,
    unsigned_body_hash: &str,
    signer: &Address,
    tx: &dyn Transaction,
    profile: &CodecProfile,
) -> Result<(), SdkError> {
    if review.unsigned_body_hash != unsigned_body_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "review does not match this transaction body",
            serde_json::json!({
                "expected": review.unsigned_body_hash,
                "actual": unsigned_body_hash,
            }),
        ));
    }
    if review.codec_profile_hash != profile.profile_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::CodecProfileMismatch,
            "review was created under a different codec profile",
            serde_json::json!({
                "expected": profile.profile_hash,
                "actual": review.codec_profile_hash,
            }),
        ));
    }
    if let Some(bound_signer) = &review.signer_address {
        if bound_signer != &signer.to_readable() {
            return Err(SdkError::with_detail(
                SdkErrorCode::ReviewBindingMismatch,
                "review was bound to a different signer",
                serde_json::json!({
                    "expected": bound_signer,
                    "actual": signer.to_readable(),
                }),
            ));
        }
    }
    let binding = crate::inspect::review_binding_of(review, unsigned_body_hash, tx, profile);
    if binding != review.review_binding {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "review binding does not verify",
            serde_json::json!({ "expected": review.review_binding, "actual": binding }),
        ));
    }
    Ok(())
}

/// Frozen proof envelope checks shared by the transaction attach path and the
/// message verify path, so both reject a malformed proof identically.
pub fn validate_proof_format(proof: &SignatureProof) -> Result<(), SdkError> {
    if proof.schema != SCHEMA_SIGNATURE_PROOF {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!("unsupported proof schema {:?}", proof.schema),
        ));
    }
    if proof.algorithm != "secp256k1-rfc6979-sha256" {
        return Err(SdkError::with_detail(
            SdkErrorCode::UnsupportedFeature,
            "unsupported signature algorithm",
            serde_json::json!({ "actual": proof.algorithm }),
        ));
    }
    Ok(())
}

/// A signing request with `expires_at` set stops being valid at that instant;
/// enforced by both consumer paths (`tx.attach_signature`, `message.verify`).
pub fn check_request_expiry(request: &SigningRequest) -> Result<(), SdkError> {
    if let Some(expires_at) = request.expires_at {
        let now = sys::curtimes();
        if now > expires_at {
            return Err(SdkError::with_detail(
                SdkErrorCode::RequestExpired,
                format!("signing request expired at {expires_at}"),
                serde_json::json!({ "expires_at": expires_at, "now": now }),
            ));
        }
    }
    Ok(())
}

fn parse_proof(proof: &SignatureProof) -> Result<(Sign, Address), SdkError> {
    validate_proof_format(proof)?;
    let publickey: [u8; 33] = hex::decode(&proof.public_key)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(SdkErrorCode::InvalidPublicKey, "public key must be 33-byte hex"))?;
    let signature: [u8; 64] = hex::decode(&proof.signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(SdkErrorCode::BadSignature, "signature must be 64-byte hex"))?;
    let signer = Address::from(sys::Account::get_address_by_public_key(publickey));
    Ok((Sign { publickey, signature }, signer))
}

/// `tx.attach_signature`: atomic attach of one external signature. Clones,
/// re-validates body/review/request/profile/signer/signature, then commits
/// (doc 14 §4.5/§4.9). Same key + same signature is idempotent; same key with
/// a different signature is `DuplicateSigner`.
///
/// `review` (when provided) is the approval context from `tx.inspect`: it
/// must cover this body, this signer and this profile, and its binding must
/// recompute from its own content. `request` (when provided) is the
/// `SigningRequest` the vault signed: the proof must carry its id/binding and
/// the request must not be expired. This closes the full approval chain
/// review → request → proof → body; with both absent the attach degrades to
/// body+signer+signature validation only.
pub fn attach_signature(
    body_hex: &str,
    proof: &SignatureProof,
    review: Option<&crate::inspect::Review>,
    request: Option<&SigningRequest>,
    profile: &CodecProfile,
) -> Result<AttachResult, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    if tx.ty() == 1 {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedTxType,
            "type 1 transactions cannot be signed",
        ));
    }
    let (sign, signer) = parse_proof(proof)?;
    let unsigned_body_hash = crate::audit::unsigned_body_hash(body_hex)?;
    if let Some(request) = request {
        check_request_expiry(request)?;
        if proof.request_id != request.id || proof.request_binding != request.request_binding {
            return Err(SdkError::with_detail(
                SdkErrorCode::ReviewBindingMismatch,
                "proof does not match the signing request",
                serde_json::json!({
                    "expected_id": request.id,
                    "actual_id": proof.request_id,
                    "expected_binding": request.request_binding,
                    "actual_binding": proof.request_binding,
                }),
            ));
        }
    }
    if let Some(review) = review {
        verify_review(review, &unsigned_body_hash, &signer, tx.as_ref(), profile)?;
    }
    let required = tx
        .req_sign()
        .map_err(|error| SdkError::from(error))?;
    if !required.contains(&signer) {
        return Err(SdkError::with_detail(
            SdkErrorCode::NotRequiredSigner,
            format!("signer {} is not a required signer", signer.to_readable()),
            serde_json::json!({ "actual": signer.to_readable() }),
        ));
    }
    // Same-key rules: idempotent on identical signature, reject otherwise.
    for existing in tx.signs() {
        if existing.publickey == sign.publickey {
            if existing.signature == sign.signature {
                return Ok(attach_result(tx.as_ref()));
            }
            return Err(SdkError::with_detail(
                SdkErrorCode::DuplicateSigner,
                format!("signer {} already signed with a different signature", signer.to_readable()),
                serde_json::json!({ "actual": signer.to_readable() }),
            ));
        }
    }
    let sign_hash = protocol::tx_std::sign_hash_for(tx.as_ref(), &signer);
    if !sys::Account::verify_signature(&sign_hash.0, &sign.publickey, &sign.signature) {
        return Err(SdkError::with_detail(
            SdkErrorCode::BadSignature,
            format!("signature does not verify for signer {}", signer.to_readable()),
            serde_json::json!({ "actual": signer.to_readable() }),
        ));
    }
    if tx.signs().len() >= crate::profile::TX_ACTIONS_MAX {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            "signature count exceeds protocol maximum",
            serde_json::json!({ "expected": crate::profile::TX_ACTIONS_MAX }),
        ));
    }
    // Clone first, validate, then commit.
    let signed = {
        use protocol::tx_std::{TransactionType2, TransactionType3};
        let mut out: Option<base::TxRef> = None;
        if let Some(t) = tx.as_any().downcast_ref::<TransactionType2>() {
            let mut copy = t.clone();
            copy.signs.push(sign.clone()).map_err(SdkError::from)?;
            out = Some(std::sync::Arc::new(copy));
        } else if let Some(t) = tx.as_any().downcast_ref::<TransactionType3>() {
            let mut copy = t.clone();
            copy.signs.push(sign.clone()).map_err(SdkError::from)?;
            out = Some(std::sync::Arc::new(copy));
        }
        out.ok_or_else(|| SdkError::new(SdkErrorCode::UnsupportedTxType, "unsupported tx type"))?
    };
    // Post-commit re-validation: the attached signature must verify against
    // the committed body. The protocol's execute-time exact-match rules
    // (Type-3 deterministic signer set, full required coverage) are
    // deliberately not enforced here: multi-signer transactions attach
    // incrementally, and completeness is reported via `complete` /
    // `missing_signers` instead of being a per-attach gate.
    let committed_hash = protocol::tx_std::sign_hash_for(signed.as_ref(), &signer);
    if !sys::Account::verify_signature(&committed_hash.0, &sign.publickey, &sign.signature) {
        return Err(SdkError::with_detail(
            SdkErrorCode::BadSignature,
            format!(
                "signed body failed verification for signer {}",
                signer.to_readable()
            ),
            serde_json::json!({ "actual": signer.to_readable() }),
        ));
    }
    Ok(attach_result(signed.as_ref()))
}

fn attach_result(tx: &dyn Transaction) -> AttachResult {
    let report = protocol::tx_std::signature_report(tx).unwrap_or_else(|_| {
        protocol::tx_std::TxSignatureReport {
            required: vec![],
            present: vec![],
            valid: vec![],
            missing: vec![],
            invalid: vec![],
        }
    });
    AttachResult {
        schema: SCHEMA_ATTACH_RESULT.to_owned(),
        body: hex::encode(tx.encode()),
        complete: report.required.iter().all(|addr| report.valid.contains(addr)),
        missing_signers: report
            .missing
            .iter()
            .map(|addr| addr.to_readable())
            .collect(),
    }
}

/// `tx.verify`: full protocol + signature verification.
pub fn verify_signatures(body_hex: &str) -> Result<VerifyResult, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    match tx.verify_signature() {
        Ok(()) => Ok(VerifyResult {
            schema: SCHEMA_VERIFY_RESULT.to_owned(),
            ok: true,
            errors: vec![],
        }),
        Err(error) => Ok(VerifyResult {
            schema: SCHEMA_VERIFY_RESULT.to_owned(),
            ok: false,
            errors: vec![error.to_string()],
        }),
    }
}

/// `tx.signature_report`: present/missing/invalid signers; never requires
/// completeness (doc 14 §5).
pub fn signature_report(body_hex: &str) -> Result<SignatureReport, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let report = protocol::tx_std::signature_report(tx.as_ref())
        .map_err(|error| SdkError::from(error))?;
    Ok(SignatureReport {
        schema: SCHEMA_SIGNATURE_REPORT.to_owned(),
        required: report.required.iter().map(|addr| addr.to_readable()).collect(),
        present: report.present.iter().map(|addr| addr.to_readable()).collect(),
        valid: report.valid.iter().map(|addr| addr.to_readable()).collect(),
        missing: report.missing.iter().map(|addr| addr.to_readable()).collect(),
        invalid: report.invalid.iter().map(|addr| addr.to_readable()).collect(),
    })
}
