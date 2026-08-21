//! Signing state machine (Unified SDK 2.0, doc 14 §4.5/§4.9): the SDK computes sign hashes and
//! consumes `SignatureProof`s; private keys never cross the boundary. It verifies only the bindings it issued and attaches via `insert_attached_sign`; chain-rule outcomes are reported, never judged.

use field::{Address, Sign};

use crate::error::{SdkError, SdkErrorCode};
use crate::inspect::{decode_body_hex, decode_tx};
use crate::json::SdkJsonTo;
use crate::profile::CodecProfile;
use crate::schema::{
    DOMAIN_SIGNING_REQUEST, SCHEMA_ATTACH_RESULT, SCHEMA_SIGNATURE_PROOF, SCHEMA_SIGNATURE_REPORT,
    SCHEMA_SIGNING_REQUEST, SCHEMA_VERIFY_RESULT,
};

/// Structured signing request produced by `prepare_signature` (doc 14 §4.9).
/// The vault signs `digest`; `id`/`request_binding` are the recomputed binding, so any field edit breaks them.
#[derive(Debug, Clone, PartialEq)]
pub struct SigningRequest {
    pub schema: String,
    pub id: String,
    pub purpose: String,
    pub algorithm: String,
    pub signer_address: String,
    pub digest: String,
    pub body_hash: Option<String>,
    pub review_binding: Option<String>,
    /// The policy decision the SDK itself computed (when a policy was supplied).
    /// A `deny` is bound in as a fact; the SDK never refuses to prepare/attach for it.
    pub policy_decision: Option<crate::policy::PolicyDecision>,
    pub origin: Option<String>,
    pub expires_at: Option<u64>,
    pub request_binding: String,
}

/// External signer output (doc 14 §4.9, frozen schema). No secret fields.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureProof {
    pub schema: String,
    pub request_id: String,
    pub request_binding: String,
    pub public_key: String,
    pub signature: String,
    pub algorithm: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachResult {
    pub schema: String,
    pub body: String,
    pub complete: bool,
    pub present_signers: Vec<String>,
    pub valid_signers: Vec<String>,
    pub missing_signers: Vec<String>,
    pub invalid_signers: Vec<String>,
    pub signature_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifyResult {
    pub schema: String,
    pub ok: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignatureReport {
    pub schema: String,
    pub required: Vec<String>,
    pub present: Vec<String>,
    pub valid: Vec<String>,
    pub missing: Vec<String>,
    pub invalid: Vec<String>,
}

/// Domain-separated binding of a signing request: sha3-256 over the canonical
/// request JSON minus `request_binding` and `id`. Used by prepare/proof paths.
pub fn request_binding_of(request: &SigningRequest) -> String {
    let mut copy = request.clone();
    copy.request_binding.clear();
    copy.id.clear();
    let body = copy.to_json_string();
    let mut data = Vec::with_capacity(DOMAIN_SIGNING_REQUEST.len() + body.len());
    data.extend_from_slice(DOMAIN_SIGNING_REQUEST);
    data.extend_from_slice(body.as_bytes());
    hex::encode(sys::calculate_hash(data))
}

/// `tx.prepare_signature`: re-validates the body and (when provided) the
/// review/policy bindings, computes the local sign hash for `signer_address`, and returns a `SigningRequest`. The SDK never signs.
pub fn prepare_signature(
    body_hex: &str,
    signer_address: &str,
    review: Option<&crate::inspect::Review>,
    policy: Option<&crate::policy::Policy>,
    origin: Option<&str>,
    expires_at: Option<u64>,
    profile: &CodecProfile,
) -> Result<SigningRequest, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let signer = Address::from_readable(signer_address).map_err(SdkError::from)?;
    let unsigned_body_hash = crate::audit::unsigned_body_hash(body_hex)?;
    if let Some(review) = review {
        verify_review(
            review,
            &unsigned_body_hash,
            Some(&signer),
            tx.as_ref(),
            profile,
        )?;
    }
    let policy_decision = match policy {
        Some(policy) => {
            let review = review.ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::PolicyBindingMismatch,
                    "policy requires a review",
                )
            })?;
            Some(crate::policy::evaluate_policy(review, policy)?)
        }
        None => None,
    };
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
        policy_decision,
        origin: origin.map(str::to_owned),
        expires_at,
        request_binding: String::new(),
    };
    let binding = request_binding_of(&request);
    request.request_binding = binding.clone();
    request.id = binding;
    Ok(request)
}

/// Shared approval-context verification: the review must cover this body and profile and its
/// binding must recompute from its own content (see `crate::inspect::review_binding_of`); `signer` is the acting signer, `None` on paths without one.
pub fn verify_review(
    review: &crate::inspect::Review,
    unsigned_body_hash: &str,
    signer: Option<&Address>,
    tx: &dyn base::TransactionSign,
    profile: &CodecProfile,
) -> Result<(), SdkError> {
    if review.unsigned_body_hash != unsigned_body_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "review does not match this transaction body",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&review.unsigned_body_hash)),
                crate::json::kv("actual", crate::json::q(&unsigned_body_hash)),
            ]),
        ));
    }
    if review.codec_profile_hash != profile.profile_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::CodecProfileMismatch,
            "review was created under a different codec profile",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&profile.profile_hash)),
                crate::json::kv("actual", crate::json::q(&review.codec_profile_hash)),
            ]),
        ));
    }
    if let Some(signer) = signer {
        if let Some(bound_signer) = &review.signer_address {
            if bound_signer != &signer.to_readable() {
                return Err(SdkError::with_detail(
                    SdkErrorCode::ReviewBindingMismatch,
                    "review was bound to a different signer",
                    crate::json::obj(vec![
                        crate::json::kv("expected", crate::json::q(&bound_signer)),
                        crate::json::kv("actual", crate::json::q(&signer.to_readable())),
                    ]),
                ));
            }
        }
    }
    let binding = crate::inspect::review_binding_of(review, unsigned_body_hash, tx, profile);
    if binding != review.review_binding {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "review binding does not verify",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&review.review_binding)),
                crate::json::kv("actual", crate::json::q(&binding)),
            ]),
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
            format!("{{\"actual\":{}}}", proof.algorithm),
        ));
    }
    Ok(())
}

/// A signing request with `expires_at` set stops being valid at that instant;
/// enforced by both consumer paths (`tx.attach_signature`, `message.verify`).
pub fn check_request_expiry(request: &SigningRequest) -> Result<(), SdkError> {
    if let Some(expires_at) = request.expires_at {
        let now = crate::now_secs();
        if now > expires_at {
            return Err(SdkError::with_detail(
                SdkErrorCode::RequestExpired,
                format!("signing request expired at {expires_at}"),
                crate::json::obj(vec![
                    crate::json::kv("expires_at", expires_at.to_string()),
                    crate::json::kv("now", now.to_string()),
                ]),
            ));
        }
    }
    Ok(())
}

pub(crate) fn parse_proof(proof: &SignatureProof) -> Result<(Sign, Address), SdkError> {
    validate_proof_format(proof)?;
    let publickey: [u8; 33] = crate::inspect::decode_hex_fixed(
        &proof.public_key,
        SdkErrorCode::InvalidPublicKey,
        "public key must be 33-byte hex",
    )?;
    let signature: [u8; 64] = crate::inspect::decode_hex_fixed(
        &proof.signature,
        SdkErrorCode::BadSignature,
        "signature must be 64-byte hex",
    )?;
    let signer = Address::from(sys::Account::get_address_by_public_key(publickey));
    Ok((
        Sign {
            publickey,
            signature,
        },
        signer,
    ))
}

/// One-shot integrity check of a signing request: `id`/`request_binding` must equal the
/// binding recomputed over the content, so any field edit after `prepare` is detected; also enforces the frozen algorithm envelope and expiry.
pub fn verify_request_integrity(request: &SigningRequest) -> Result<(), SdkError> {
    let binding = request_binding_of(request);
    if request.id != binding || request.request_binding != binding {
        return Err(SdkError::with_detail(
            SdkErrorCode::InvalidSigningRequest,
            "signing request id/binding do not match its content",
            crate::json::obj(vec![
                crate::json::kv("expected_binding", crate::json::q(&binding)),
                crate::json::kv("actual_id", crate::json::q(&request.id)),
                crate::json::kv("actual_binding", crate::json::q(&request.request_binding)),
            ]),
        ));
    }
    if request.algorithm != "secp256k1-rfc6979-sha256" {
        return Err(SdkError::with_detail(
            SdkErrorCode::UnsupportedFeature,
            "unsupported signature algorithm",
            format!("{{\"actual\":{}}}", request.algorithm),
        ));
    }
    check_request_expiry(request)?;
    Ok(())
}

/// Full approval-context verification for `tx.attach_signature`: request self-consistency
/// (`verify_request_integrity`), body/digest/signer binding, proof↔request binding, and a policy decision bound to the review — its value is never an attach refusal.
fn verify_attach_context(
    tx: &dyn base::TransactionSign,
    signer: &Address,
    proof: &SignatureProof,
    review: &crate::inspect::Review,
    request: &SigningRequest,
    unsigned_body_hash: &str,
    profile: &CodecProfile,
) -> Result<(), SdkError> {
    verify_request_integrity(request)?;
    if request.purpose != "transaction" {
        return Err(SdkError::with_detail(
            SdkErrorCode::UnsupportedFeature,
            "attach requires a transaction signing request",
            format!("{{\"actual\":{}}}", request.purpose),
        ));
    }
    if request.signer_address != signer.to_readable() {
        return Err(SdkError::with_detail(
            SdkErrorCode::InvalidSigningRequest,
            "request signer does not match the proof signer",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&request.signer_address)),
                crate::json::kv("actual", crate::json::q(&signer.to_readable())),
            ]),
        ));
    }
    if request.body_hash.as_deref() != Some(unsigned_body_hash) {
        return Err(SdkError::with_detail(
            SdkErrorCode::InvalidSigningRequest,
            "request body hash does not match this transaction",
            crate::json::obj(vec![
                crate::json::kv(
                    "expected",
                    crate::json::q(request.body_hash.as_deref().unwrap_or("")),
                ),
                crate::json::kv("actual", crate::json::q(&unsigned_body_hash)),
            ]),
        ));
    }
    let sign_hash = hex::encode(protocol::tx_std::sign_hash_for(tx, signer).0);
    if request.digest != sign_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::InvalidSigningRequest,
            "request digest does not match this signer's sign hash",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&request.digest)),
                crate::json::kv("actual", crate::json::q(&sign_hash)),
            ]),
        ));
    }
    if proof.request_id != request.id || proof.request_binding != request.request_binding {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "proof does not match the signing request",
            crate::json::obj(vec![
                crate::json::kv("expected_id", crate::json::q(&request.id)),
                crate::json::kv("actual_id", crate::json::q(&proof.request_id)),
                crate::json::kv("expected_binding", crate::json::q(&request.request_binding)),
                crate::json::kv("actual_binding", crate::json::q(&proof.request_binding)),
            ]),
        ));
    }
    if request.review_binding.as_deref() != Some(&review.review_binding) {
        return Err(SdkError::with_detail(
            SdkErrorCode::ReviewBindingMismatch,
            "request review binding does not match the provided review",
            crate::json::obj(vec![
                crate::json::kv(
                    "expected",
                    crate::json::q(request.review_binding.as_deref().unwrap_or("")),
                ),
                crate::json::kv("actual", crate::json::q(&review.review_binding)),
            ]),
        ));
    }
    if let Some(decision) = &request.policy_decision {
        // The decision must be bound to this review and its binding recompute;
        // its VALUE is a business fact the caller acts on, never an attach refusal.
        if decision.review_binding != review.review_binding {
            return Err(SdkError::with_detail(
                SdkErrorCode::PolicyBindingMismatch,
                "policy decision is not bound to this review",
                crate::json::obj(vec![
                    crate::json::kv("expected", crate::json::q(&decision.review_binding)),
                    crate::json::kv("actual", crate::json::q(&review.review_binding)),
                ]),
            ));
        }
        let expected_binding = crate::policy::policy_binding_of(decision);
        if expected_binding != decision.policy_binding {
            return Err(SdkError::with_detail(
                SdkErrorCode::PolicyBindingMismatch,
                "policy decision binding does not verify",
                crate::json::obj(vec![
                    crate::json::kv("expected", crate::json::q(&expected_binding)),
                    crate::json::kv("actual", crate::json::q(&decision.policy_binding)),
                ]),
            ));
        }
    }
    verify_review(review, unsigned_body_hash, Some(signer), tx, profile)?;
    Ok(())
}

/// `tx.attach_signature`: full approval-chain attach — verifies request integrity,
/// body/digest/signer binding, proof↔request and review bindings, then inserts mechanically via `insert_attached_sign` (chain signer rules are reported, never judged).
pub fn attach_signature(
    body_hex: &str,
    proof: &SignatureProof,
    review: &crate::inspect::Review,
    request: &SigningRequest,
    profile: &CodecProfile,
) -> Result<AttachResult, SdkError> {
    attach_signature_inner(body_hex, proof, Some((review, request)), profile)
}

/// `tx.attach_signature_unbound`: low-level attach of one external signature with no approval
/// context — validates body and proof envelope only; completeness is reported, not gated.
pub fn attach_signature_unbound(
    body_hex: &str,
    proof: &SignatureProof,
    profile: &CodecProfile,
) -> Result<AttachResult, SdkError> {
    attach_signature_inner(body_hex, proof, None, profile)
}

fn attach_signature_inner(
    body_hex: &str,
    proof: &SignatureProof,
    context: Option<(&crate::inspect::Review, &SigningRequest)>,
    profile: &CodecProfile,
) -> Result<AttachResult, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let (sign, signer) = parse_proof(proof)?;
    let unsigned_body_hash = crate::audit::unsigned_body_hash(body_hex)?;
    if let Some((review, request)) = context {
        verify_attach_context(
            tx.as_ref(),
            &signer,
            proof,
            review,
            request,
            &unsigned_body_hash,
            profile,
        )?;
    }
    // Mechanical insert via `insert_attached_sign` (same-key replacement, no digest check);
    // crypto / type-3 D-set acceptance is reported via signature_report / tx.verify.
    let signed = protocol::tx_std::insert_attached_sign(tx.as_ref(), sign.clone())
        .map_err(|error| SdkError::from(error))?;
    Ok(attach_result(signed.as_ref()))
}

fn attach_result(tx: &dyn base::TransactionSign) -> AttachResult {
    let (report, signature_errors) = match protocol::tx_std::signature_report(tx) {
        Ok(report) => (report, Vec::new()),
        Err(error) => (
            protocol::tx_std::TxSignatureReport {
                required: vec![],
                present: vec![],
                valid: vec![],
                missing: vec![],
                invalid: vec![],
            },
            vec![error.to_string()],
        ),
    };
    AttachResult {
        schema: SCHEMA_ATTACH_RESULT.to_owned(),
        body: hex::encode(tx.encode()),
        complete: signature_errors.is_empty()
            && report
                .required
                .iter()
                .all(|addr| report.valid.contains(addr)),
        present_signers: report
            .present
            .iter()
            .map(|addr| addr.to_readable())
            .collect(),
        valid_signers: report.valid.iter().map(|addr| addr.to_readable()).collect(),
        missing_signers: report
            .missing
            .iter()
            .map(|addr| addr.to_readable())
            .collect(),
        invalid_signers: report
            .invalid
            .iter()
            .map(|addr| addr.to_readable())
            .collect(),
        signature_errors,
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
    match protocol::tx_std::signature_report(tx.as_ref()) {
        Ok(report) => Ok(SignatureReport {
            schema: SCHEMA_SIGNATURE_REPORT.to_owned(),
            required: report
                .required
                .iter()
                .map(|addr| addr.to_readable())
                .collect(),
            present: report
                .present
                .iter()
                .map(|addr| addr.to_readable())
                .collect(),
            valid: report.valid.iter().map(|addr| addr.to_readable()).collect(),
            missing: report
                .missing
                .iter()
                .map(|addr| addr.to_readable())
                .collect(),
            invalid: report
                .invalid
                .iter()
                .map(|addr| addr.to_readable())
                .collect(),
        }),
        Err(_error) => Ok(SignatureReport {
            schema: SCHEMA_SIGNATURE_REPORT.to_owned(),
            required: vec![],
            present: vec![],
            valid: vec![],
            missing: vec![],
            invalid: vec![],
        }),
    }
}
