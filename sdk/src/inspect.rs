//! Transaction inspection: decode → protocol facts → Review (Unified SDK 2.0,
//! doc 14 §5/§6.1). The SDK never executes transactions and never consults a
//! node; the chain context is caller input.

use base::{BinaryCodecs, JsonCodecs, Transaction};
use field::Address;

use crate::audit::{self, Auditability};
use crate::codec::standard_codecs;
use crate::error::{SdkError, SdkErrorCode};
use crate::profile::CodecProfile;
use crate::schema::{SCHEMA_REVIEW, SCHEMA_TRANSACTION_JSON};

/// Chain context for strict inspection (doc 14 §5 `tx.inspect`).
#[derive(Debug, Clone, PartialEq)]
pub struct InspectContext {
    pub current_height: u64,
    pub expected_chain_id: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeightRangeDesc {
    pub start: u64,
    pub end: u64,
}

/// The review object (doc 14 §6.1). Contains protocol facts and audit
/// material only; no policy decision, no bridge fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub schema: String,
    pub codec_profile_hash: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
        pub gas_max: Option<u8>,
    pub tx_hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub review_binding: String,
    /// The signer this review's binding was computed for, if any. Stored in
    /// the review so `review_binding` is recomputable by anyone holding the
    /// review (see `review_binding_of`).
    pub signer_address: Option<String>,
    /// The strict-inspect context the binding was computed with; `None` for
    /// report mode. Stored for the same recomputability reason.
    pub inspect_context: Option<InspectContext>,
    pub protocol_valid: bool,
    pub signability: String,
    pub auditability: String,
    pub requires_user_confirmation: bool,
    /// Consensus-level limit violations of the *decoded* transaction (body
    /// size, type-3 signer count). Facts, never decode denials: the SDK
    /// reports what the wire carries and the upper layer decides.
    pub limits_violations: Vec<String>,
    pub required_signers: Vec<String>,
    pub present_signers: Vec<String>,
    pub missing_signers: Vec<String>,
        pub chain_ids_allowed: Option<Vec<u32>>,
        pub valid_height_range: Option<HeightRangeDesc>,
        pub fee_purity: Option<u64>,
        pub fee_purity_ok: Option<bool>,
    pub actions: Vec<crate::audit::ActionDesc>,
    pub asset_serials: Vec<u64>,
}

/// Decode a transaction body with the SDK codec registry and fail-closed
/// rules: unknown tx type, malformed wire, trailing bytes. The registry (the
/// chain-codec surface) decides which tx types exist; consensus-level rules
/// (size caps, signer counts) are reported as review facts, never as decode
/// denials — the SDK exposes what the wire carries and lets the upper layer
/// judge.
pub fn decode_tx(body: &[u8]) -> Result<base::TxRef, SdkError> {
    if body.is_empty() {
        return Err(SdkError::new(SdkErrorCode::ParseFailed, "tx body is empty"));
    }
    let codecs = standard_codecs().map_err(SdkError::from)?;
    let (tx, used) = codecs
        .decode_transaction(body)
        .map_err(|error| SdkError::from(error))?;
    if used != body.len() {
        return Err(SdkError::with_detail(
            SdkErrorCode::TrailingBytes,
            format!("tx parse consumed {used} of {} bytes", body.len()),
            crate::json::obj(vec![crate::json::kv("byte_offset", used.to_string()), crate::json::kv("actual", body.len().to_string())]),
        ));
    }
    Ok(tx)
}

/// Re-encode the transaction with its signature set cleared. Used for the
/// stable `unsigned_body_hash`; Type-2/3 wire order is preserved exactly.
/// The concrete type list lives in `protocol::tx_std` (the crate that owns
/// the types), so a new transaction type needs no change here.
pub fn encode_without_signs(tx: &dyn Transaction) -> Result<Vec<u8>, SdkError> {
    protocol::tx_std::encode_without_signs(tx).map_err(SdkError::from)
}

/// Guard facts come from the protocol's single `guard_facts` analysis
/// (`protocol::action_std`, co-located with the guard action definitions and
/// their execute bodies); the SDK never re-derives guard semantics. The
/// effective chain set is the intersection of all `ChainAllow` actions and
/// the effective height range the intersection of all `HeightScope` actions.

fn collect_guard_facts(tx: &dyn Transaction) -> protocol::action_std::GuardFacts {
    protocol::action_std::guard_facts(tx)
}

fn parse_signer_address(signer_address: Option<&str>) -> Result<Option<Address>, SdkError> {
    match signer_address {
        None | Some("") => Ok(None),
        Some(raw) => Address::from_readable(raw)
            .map(Some)
            .map_err(|error| SdkError::from(error)),
    }
}

fn readable_signers(addrs: &[Address]) -> Vec<String> {
    addrs.iter().map(|addr| addr.to_readable()).collect()
}

/// Common review construction shared by report and strict modes.
fn build_review(
    body: &[u8],
    signer_address: Option<&str>,
    context: Option<&InspectContext>,
    profile: &CodecProfile,
) -> Result<Review, SdkError> {
    let tx = decode_tx(body)?;
    let signer = parse_signer_address(signer_address)?;
    let unsigned_body = encode_without_signs(tx.as_ref())?;
    let mut data = Vec::with_capacity(audit::DOMAIN_UNSIGNED_BODY.len() + unsigned_body.len());
    data.extend_from_slice(audit::DOMAIN_UNSIGNED_BODY);
    data.extend_from_slice(&unsigned_body);
    let unsigned_body_hash = hex::encode(sys::calculate_hash(data));

    let report = protocol::tx_std::signature_report(tx.as_ref())
        .map_err(|error| SdkError::from(error))?;
    let guard_facts = collect_guard_facts(tx.as_ref());

    let mut actions = Vec::with_capacity(tx.action_count());
    let mut asset_serials = Vec::new();
    let mut auditability = Auditability::Full;
    for (index, action) in tx.actions().iter().enumerate() {
        let mut desc = audit::describe_action(action, index, &index.to_string(), 0);
        // Guard notes are protocol violations (the single `guard_facts`
        // analysis): the per-action `protocol_valid` fact reflects them, so
        // the descriptor never claims validity the review denies.
        if let Some((_, note)) = guard_facts.action_notes.iter().find(|(idx, _)| *idx == index) {
            desc.audit_notes.push(note.clone());
            desc.protocol_valid = false;
        }
        auditability = Auditability::worse(auditability, classify(&desc.auditability));
        collect_asset_serials(action, &mut asset_serials);
        actions.push(desc);
    }
    let requires_user_confirmation = auditability != Auditability::Full;

    // Consensus-level limit facts, reported (never gated) for the decoded tx.
    let mut limits_violations = Vec::new();
    if body.len() > base::MAX_TX_SIZE {
        limits_violations.push(format!(
            "tx body {} bytes exceeds consensus maximum {}",
            body.len(),
            base::MAX_TX_SIZE
        ));
    }
    // The signer cap (type 3 only) is the protocol's rule, evaluated on the
    // required signer set; the SDK reports it as a fact.
    if let Err(error) =
        protocol::tx_std::check_signers_cap(tx.as_ref(), profile.protocol_params.max_type3_signers)
    {
        limits_violations.push(error.to_string());
    }

    // Every tx type the codec registry can decode is signable; the chain
    // decides whether it accepts the signed body (e.g. flag-gated types).
    let signability = if standard_codecs()
        .map(|codecs| codecs.registered_tx_types().contains(&tx.ty()))
        .unwrap_or(false)
    {
        "signable".to_owned()
    } else {
        "unsupported_tx_type".to_owned()
    };

    // Fee purity is a protocol trait fact (`Transaction::fee_purity`,
    // type-specific computation owned by protocol); the floor comparison is
    // reported as-is and the upper layer judges it.
    let fee_purity = Some(tx.fee_purity());
    let fee_purity_ok = Some(tx.fee_purity() >= profile.protocol_params.fee_purity_floor);

    let mut review = Review {
        schema: SCHEMA_REVIEW.to_owned(),
        codec_profile_hash: profile.profile_hash.clone(),
        tx_type: tx.ty(),
        timestamp: tx.timestamp().value(),
        main: tx.main().to_readable(),
        fee: tx.fee().to_fin_string(),
        gas_max: tx.gas_max_byte(),
        tx_hash: hex::encode(tx.hash().0),
        hash_with_fee: hex::encode(tx.hash_with_fee().0),
        unsigned_body_hash: unsigned_body_hash.clone(),
        review_binding: String::new(),
        signer_address: signer.as_ref().map(|addr| addr.to_readable()),
        inspect_context: context.cloned(),
        protocol_valid: guard_facts.protocol_violations.is_empty(),
        signability,
        auditability: auditability.as_str().to_owned(),
        requires_user_confirmation,
        limits_violations,
        required_signers: readable_signers(&report.required),
        present_signers: readable_signers(&report.present),
        missing_signers: readable_signers(&report.missing),
        chain_ids_allowed: guard_facts.chains,
        valid_height_range: guard_facts.height_range.map(|range| HeightRangeDesc {
            start: range.0,
            end: range.1,
        }),
        fee_purity,
        fee_purity_ok,
        actions,
        asset_serials,
    };

    // Bind the review with the shared single-source computation, so the same
    // recomputation in prepare/attach/encode verifies it.
    review.review_binding = review_binding_of(&review, &unsigned_body_hash, tx.as_ref(), profile);
    Ok(review)
}

/// Canonical review binding — the single computation shared by `build_review`,
/// `prepare_signature`, `attach_signature` and `encode_transaction_json`:
/// sha3-256 over domain + unsigned_body_hash + signer + sign_hash + codec
/// profile hash + inspect context + canonical review digest (which excludes
/// `review_binding` itself and non-deterministic display text). Anyone holding
/// the review can recompute it, so a review whose displayed fields were
/// edited after inspection never re-verifies.
pub fn review_binding_of(
    review: &Review,
    unsigned_body_hash: &str,
    tx: &dyn Transaction,
    profile: &CodecProfile,
) -> String {
    let sign_hash = review
        .signer_address
        .as_deref()
        .and_then(|raw| Address::from_readable(raw).ok())
        .map(|signer| hex::encode(protocol::tx_std::sign_hash_for(tx, &signer).0));
    let context_json = match &review.inspect_context {
        Some(context) => context.to_json_string(),
        None => String::new(),
    };
    let review_digest = audit::canonical_review_digest(review);
    audit::compute_review_binding(
        unsigned_body_hash,
        review.signer_address.as_deref(),
        sign_hash.as_deref(),
        &profile.profile_hash,
        &context_json,
        &review_digest,
    )
}

fn classify(grade: &str) -> Auditability {
    match grade {
        "structured" => Auditability::Structured,
        "branching" => Auditability::Branching,
        "opaque" => Auditability::Opaque,
        _ => Auditability::Full,
    }
}

fn collect_asset_serials(action: &base::ActionRef, out: &mut Vec<u64>) {
    if let Some(transfer) = action.as_transfer_like() {
        if let base::TransferPayload::Asset { serial, .. } = transfer.transfer_payload() {
            if !out.contains(&serial) {
                out.push(serial);
            }
        }
    }
}

/// `tx.inspect_report`: complete report without chain context; never fails on
/// expired heights or wrong chains.
pub fn inspect_report(
    body_hex: &str,
    signer_address: Option<&str>,
    profile: &CodecProfile,
) -> Result<Review, SdkError> {
    let body = decode_body_hex(body_hex)?;
    build_review(&body, signer_address, None, profile)
}

/// `tx.inspect`: strict mode. Requires chain context and validates the
/// height/chain guards against it. The guard facts come from the single
/// `collect_guard_facts` implementation (the same one that feeds the review),
/// so the per-action semantics can never drift between report and strict mode.
pub fn inspect(
    body_hex: &str,
    signer_address: Option<&str>,
    context: &InspectContext,
    profile: &CodecProfile,
) -> Result<Review, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let review = build_review(&body, signer_address, Some(context), profile)?;
    // Strict guard checks against the caller-provided context, from the shared
    // guard facts (intersection of all ChainAllow/HeightScope actions).
    let facts = collect_guard_facts(tx.as_ref());
    if let Some((start, end)) = &facts.height_range {
        if *start > *end && *end != 0 {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                format!("height_scope constraints unsatisfiable ({start}..{end})"),
            ));
        }
        let height = context.current_height;
        if height < *start || (*end != 0 && height > *end) {
            return Err(SdkError::with_detail(
                SdkErrorCode::ExpiredHeight,
                format!(
                    "current height {height} outside allowed range ({start}, {end})"
                ),
                crate::json::obj(vec![crate::json::kv(
                    "expected",
                    format!("{start}..{end}")
                ), crate::json::kv("actual", height.to_string())]),
            ));
        }
    }
    if let Some(chains) = &facts.chains {
        if !chains.contains(&context.expected_chain_id) {
            return Err(SdkError::with_detail(
                SdkErrorCode::WrongChainId,
                format!(
                    "expected chain {} not in allowed chains {:?}",
                    context.expected_chain_id, chains
                ),
                crate::json::obj(vec![crate::json::kv(
                    "expected",
                    crate::json::arr(chains.iter().map(|c| c.to_string()).collect())
                ), crate::json::kv("actual", context.expected_chain_id.to_string())]),
            ));
        }
    }
    Ok(review)
}

pub(crate) fn decode_body_hex(body_hex: &str) -> Result<Vec<u8>, SdkError> {
    hex::decode(body_hex.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "body hex invalid"))
}

/// Signature entries for `tx.decode` (doc 14 §6.4). The low-level encode path
/// accepts appended entries and re-validates everything.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureEntry {
    pub public_key: String,
    pub signature: String,
}

/// `tx.decode`: strict structured codec output.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionJson {
    pub schema: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
        pub gas_max: Option<u8>,
    pub tx_hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub actions: Vec<crate::audit::ActionDesc>,
    pub signatures: Vec<SignatureEntry>,
}

pub fn decode_transaction_json(body_hex: &str) -> Result<TransactionJson, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let mut actions = Vec::with_capacity(tx.action_count());
    for (index, action) in tx.actions().iter().enumerate() {
        actions.push(crate::audit::describe_action(action, index, &index.to_string(), 0));
    }
    let signatures = tx
        .signs()
        .iter()
        .map(|sign| SignatureEntry {
            public_key: hex::encode(sign.publickey),
            signature: hex::encode(sign.signature),
        })
        .collect();
    let unsigned = encode_without_signs(tx.as_ref())?;
    let mut data = Vec::with_capacity(audit::DOMAIN_UNSIGNED_BODY.len() + unsigned.len());
    data.extend_from_slice(audit::DOMAIN_UNSIGNED_BODY);
    data.extend_from_slice(&unsigned);
    Ok(TransactionJson {
        schema: SCHEMA_TRANSACTION_JSON.to_owned(),
        tx_type: tx.ty(),
        timestamp: tx.timestamp().value(),
        main: tx.main().to_readable(),
        fee: tx.fee().to_fin_string(),
        gas_max: tx.gas_max_byte(),
        tx_hash: hex::encode(tx.hash().0),
        hash_with_fee: hex::encode(tx.hash_with_fee().0),
        unsigned_body_hash: hex::encode(sys::calculate_hash(data)),
        actions,
        signatures,
    })
}

/// `tx.encode`: rebuild a body from `tx.decode` output (low-level path, doc
/// 14 §6.4). Callers may append external signatures to `signatures[]`; the
/// SDK re-validates action json, signature format, required signers,
/// duplicates and limits, then re-encodes. The rebuilt body must reproduce
/// the declared `unsigned_body_hash` — a mismatch fails with
/// `transaction_json_mismatch` instead of silently emitting a different
/// transaction (this also catches action jsons that do not round-trip).
/// When a `review` is provided, its binding must cover the rebuilt body and
/// the review's declared hash, closing the same chain as `attach_signature`.
pub fn encode_transaction_json(
    transaction: &TransactionJson,
    review: Option<&Review>,
    profile: &CodecProfile,
) -> Result<crate::build::BuiltTransaction, SdkError> {
    use field::Sign;

    if transaction.schema != SCHEMA_TRANSACTION_JSON {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!("unsupported transaction json schema {:?}", transaction.schema),
        ));
    }
    let codecs = standard_codecs().map_err(SdkError::from)?;
    if !codecs.registered_tx_types().contains(&transaction.tx_type) {
        return Err(SdkError::with_detail(
            SdkErrorCode::UnsupportedTxType,
            format!(
                "encode supports the registered transaction types only, got {}",
                transaction.tx_type
            ),
            format!("{{\"actual\":{}}}", transaction.tx_type),
        ));
    }

    let main = Address::from_readable(&transaction.main).map_err(SdkError::from)?;
    let fee = field::Amount::from(&transaction.fee).map_err(SdkError::from)?;
    let fee_fin = fee.to_fin_string();
    let mut actions = Vec::with_capacity(transaction.actions.len());
    for desc in &transaction.actions {
        let action = codecs
            .decode_action_json(desc.kind, &desc.json)
            .map_err(SdkError::from)?
            .ok_or_else(|| {
                SdkError::with_detail(
                    SdkErrorCode::UnsupportedSchema,
                    format!("action kind {} has no json codec for re-encoding", desc.kind),
                    crate::json::obj(vec![crate::json::kv("action_index", desc.index.to_string()), crate::json::kv("action_kind", desc.kind.to_string())]),
                )
            })?;
        actions.push(action);
    }
    let mut signs = Vec::with_capacity(transaction.signatures.len());
    let mut seen: Vec<[u8; 33]> = Vec::new();
    for entry in &transaction.signatures {
        let publickey: [u8; 33] = hex::decode(&entry.public_key)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| SdkError::new(SdkErrorCode::InvalidPublicKey, "signature public key must be 33-byte hex"))?;
        let signature: [u8; 64] = hex::decode(&entry.signature)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| SdkError::new(SdkErrorCode::BadSignature, "signature must be 64-byte hex"))?;
        if seen.contains(&publickey) {
            return Err(SdkError::new(
                SdkErrorCode::DuplicateSigner,
                "duplicate public key in signatures",
            ));
        }
        seen.push(publickey);
        signs.push(Sign { publickey, signature });
    }

    // The body is built by the protocol's own standard-tx constructor (type
    // list and gas rule owned by protocol), then re-decoded for the
    // round-trip invariant.
    let encoded = protocol::tx_std::encode_standard_tx(
        base::TxCreateRequest::new(transaction.tx_type, main, fee, transaction.timestamp)
            .with_gas_max(transaction.gas_max.unwrap_or(0)),
        &actions,
        &signs,
    )
    .map_err(SdkError::from)?;
    let decoded = decode_tx(&encoded)?;
    let body_hex = hex::encode(&encoded);
    // Round-trip invariant: rebuilding a decode output must reproduce it.
    if decoded.encode() != encoded {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "encoded body failed the round-trip invariant",
        ));
    }
    let unsigned_body_hash = crate::audit::unsigned_body_hash(&body_hex)?;
    // Integrity gate: the rebuilt body must reproduce the declared hash.
    // Without this, tampering with an action's json would silently emit a
    // different transaction than the one the review binding was computed over.
    if unsigned_body_hash != transaction.unsigned_body_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::TransactionJsonMismatch,
            "rebuilt body does not match the declared unsigned_body_hash",
            crate::json::obj(vec![crate::json::kv("expected", crate::json::q(&transaction.unsigned_body_hash)), crate::json::kv("actual", crate::json::q(&unsigned_body_hash))]),
        ));
    }
    // When an approval context is supplied, its binding must cover the body
    // that was actually rebuilt (same chain as attach_signature).
    if let Some(review) = review {
        if review.unsigned_body_hash != unsigned_body_hash {
            return Err(SdkError::with_detail(
                SdkErrorCode::ReviewBindingMismatch,
                "review does not match the rebuilt body",
                crate::json::obj(vec![crate::json::kv("expected", crate::json::q(&review.unsigned_body_hash)), crate::json::kv("actual", crate::json::q(&unsigned_body_hash))]),
            ));
        }
        if review.codec_profile_hash != profile.profile_hash {
            return Err(SdkError::with_detail(
                SdkErrorCode::CodecProfileMismatch,
                "review was created under a different codec profile",
                crate::json::obj(vec![crate::json::kv("expected", crate::json::q(&profile.profile_hash)), crate::json::kv("actual", crate::json::q(&review.codec_profile_hash))]),
            ));
        }
        let binding = review_binding_of(review, &unsigned_body_hash, decoded.as_ref(), profile);
        if binding != review.review_binding {
            return Err(SdkError::with_detail(
                SdkErrorCode::ReviewBindingMismatch,
                "review binding does not verify for the rebuilt body",
                crate::json::obj(vec![crate::json::kv("expected", crate::json::q(&review.review_binding)), crate::json::kv("actual", crate::json::q(&binding))]),
            ));
        }
    }
    Ok(crate::build::BuiltTransaction {
        schema: crate::schema::SCHEMA_BUILT_TRANSACTION.to_owned(),
        tx_type: transaction.tx_type,
        timestamp: transaction.timestamp,
        main: main.to_readable(),
        fee: fee_fin,
        hash: hex::encode(decoded.hash().0),
        hash_with_fee: hex::encode(decoded.hash_with_fee().0),
        unsigned_body_hash,
        body: body_hex,
    })
}
