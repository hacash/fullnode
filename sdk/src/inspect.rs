//! Transaction inspection: decode → protocol facts → Review (Unified SDK 2.0, doc 14 §5/§6.1).
//! The SDK never executes or consults a node; the chain context is caller input, and guard/topology findings are facts, never denials.

use base::{BinaryCodecs, Transaction, TransactionSign};
use field::Address;

use crate::audit::{self, Auditability};
use crate::codec::standard_codecs;
use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::profile::CodecProfile;
use crate::schema::{SCHEMA_REVIEW, SCHEMA_TRANSACTION_JSON};

/// Chain context for strict inspection (doc 14 §5 `tx.inspect`).
#[derive(Debug, Clone, PartialEq)]
pub struct InspectContext {
    pub current_height: u64,
    pub expected_chain_id: u32,
    /// Consensus activation flags at the caller's claimed height. `None` means
    /// activation is not judged (neither treated as on nor off).
    pub consensus_flags: Option<u64>,
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
    /// The signer this review's binding was computed for, if any; stored so
    /// `review_binding` is recomputable by anyone holding the review.
    pub signer_address: Option<String>,
    /// The strict-inspect context the binding was computed with; `None` for
    /// report mode. Stored for the same recomputability reason.
    pub inspect_context: Option<InspectContext>,
    /// Strict-mode facts (context provided): height outside the effective
    /// HeightScope range / chain outside the ChainAllow set. Facts, never denials.
    pub expired_height: Option<bool>,
    pub wrong_chain: Option<bool>,
    /// Derived: every fact bucket below is empty. Never used to refuse inspect.
    pub protocol_valid: bool,
    pub signability: String,
    pub auditability: String,
    /// Alias of `auditability != "full"`. Product policy belongs in
    /// `policy.evaluate`; this field is kept as a derived convenience.
    pub requires_user_confirmation: bool,
    /// Consensus-level limit violations of the *decoded* transaction (body
    /// size, type-3 signer count). Facts, never decode denials.
    pub limits_violations: Vec<String>,
    /// Protocol action-tree topology findings (scope, min tx type, AST depth,
    /// top-rule). Facts, never inspect denials.
    pub topology_violations: Vec<String>,
    /// Guard-action protocol violations from `guard_facts`.
    pub guard_violations: Vec<String>,
    /// Height / address / gas_max / activation findings from `schedule_facts`.
    pub schedule_violations: Vec<String>,
    pub required_signers: Vec<String>,
    pub present_signers: Vec<String>,
    pub valid_signers: Vec<String>,
    pub missing_signers: Vec<String>,
    pub invalid_signers: Vec<String>,
    /// `signature_report` computation errors. Never an inspect denial.
    pub signature_errors: Vec<String>,
    pub chain_ids_allowed: Option<Vec<u32>>,
    pub valid_height_range: Option<HeightRangeDesc>,
    pub fee_purity: Option<u64>,
    pub fee_purity_ok: Option<bool>,
    pub actions: Vec<crate::audit::ActionDesc>,
    pub asset_serials: Vec<u64>,
}

/// Decode a transaction body with the SDK codec registry and fail-closed rules (unknown type,
/// malformed wire, trailing bytes via `decode_transaction_exact`). Consensus-level rules are reported as review facts, never decode denials.
pub fn decode_tx(body: &[u8]) -> Result<base::TxRef, SdkError> {
    let codecs = standard_codecs().map_err(SdkError::from)?;
    codecs
        .decode_transaction_exact(body)
        .map_err(SdkError::from)
}

/// Re-encode the transaction with its signature set cleared, preserving
/// Type-2/3 wire order; used for the stable `unsigned_body_hash`.
pub fn encode_without_signs(tx: &dyn Transaction) -> Result<Vec<u8>, SdkError> {
    protocol::tx_std::encode_without_signs(tx).map_err(SdkError::from)
}

// Guard facts come from the protocol's single `guard_facts` analysis; the
// SDK never re-derives guard semantics.

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
    options: &crate::audit::DescribeOptions,
) -> Result<Review, SdkError> {
    let tx = decode_tx(body)?;
    let signer = parse_signer_address(signer_address)?;
    let unsigned_body_hash = crate::audit::unsigned_body_hash_bytes(body)?;

    let (sign_report, signature_errors) = match protocol::tx_std::signature_report(tx.as_ref()) {
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
    let guard_facts = protocol::action_std::guard_facts(tx.as_ref());

    // Strict-mode guard evaluation via `GuardFacts::against_context`, surfaced
    // as facts (never denials); `None` without a context.
    let (expired_height, wrong_chain) = match context {
        Some(ctx) => {
            let (expired, wrong) =
                guard_facts.against_context(ctx.current_height, ctx.expected_chain_id);
            (Some(expired), Some(wrong))
        }
        None => (None, None),
    };

    let mut actions = Vec::with_capacity(tx.action_count());
    let mut asset_serials = Vec::new();
    let mut auditability = Auditability::Full;
    for (index, action) in tx.actions().iter().enumerate() {
        let mut desc = audit::describe_action(action.as_ref(), index, &index.to_string(), 0, options);
        // Guard notes are protocol violations (`guard_facts`); the per-action
        // `protocol_valid` fact reflects them.
        if let Some((_, note)) = guard_facts
            .action_notes
            .iter()
            .find(|(idx, _)| *idx == index)
        {
            desc.audit_notes.push(note.clone());
            desc.protocol_valid = false;
        }
        auditability = Auditability::worse(auditability, classify(&desc.auditability));
        collect_asset_serials(action, &mut asset_serials);
        actions.push(desc);
    }

    let mut limits_violations = Vec::new();
    let size = field::Encode::size(tx.as_ref());
    if base::tx_exceeds_max_size(size, hacash_params::MAX_TX_SIZE) {
        limits_violations.push(format!(
            "tx body {size} bytes exceeds consensus maximum {}",
            hacash_params::MAX_TX_SIZE
        ));
    }
    if let Err(error) =
        protocol::tx_std::check_signers_cap(tx.as_ref(), profile.protocol_params.max_type3_signers)
    {
        limits_violations.push(error.to_string());
    }

    let flags = context.and_then(|ctx| ctx.consensus_flags);
    let topology =
        protocol::topology_facts(tx.ty(), tx.actions(), flags, crate::profile::AST_DEPTH_MAX);
    for (index, note) in &topology.action_notes {
        if let Some(desc) = actions.get_mut(*index) {
            desc.audit_notes.push(note.clone());
            desc.protocol_valid = false;
        }
    }
    let topology_violations = topology.findings;

    let height = context.map(|ctx| ctx.current_height);
    let schedule_violations = protocol::schedule_facts(tx.as_ref(), height, flags).findings;
    let guard_violations = guard_facts.protocol_violations.clone();

    let requires_user_confirmation = auditability != Auditability::Full;
    let signability = "signable".to_owned();
    let protocol_valid = guard_violations.is_empty()
        && topology_violations.is_empty()
        && limits_violations.is_empty()
        && schedule_violations.is_empty();

    let fee_purity = Some(tx.fee_purity());
    let fee_purity_ok = context.map(|ctx| {
        tx.fee_purity()
            >= profile
                .protocol_params
                .fee_purity_floor_at(ctx.current_height)
    });

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
        expired_height,
        wrong_chain,
        protocol_valid,
        signability,
        auditability: auditability.as_str().to_owned(),
        requires_user_confirmation,
        limits_violations,
        topology_violations,
        guard_violations,
        schedule_violations,
        required_signers: readable_signers(&sign_report.required),
        present_signers: readable_signers(&sign_report.present),
        valid_signers: readable_signers(&sign_report.valid),
        missing_signers: readable_signers(&sign_report.missing),
        invalid_signers: readable_signers(&sign_report.invalid),
        signature_errors,
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

/// Canonical review binding — the single computation shared by build/prepare/attach/encode:
/// sha3-256 over domain + unsigned_body_hash + signer + sign_hash + codec profile hash + inspect context + canonical review digest (excludes `review_binding`); recomputable by anyone holding the review.
pub fn review_binding_of(
    review: &Review,
    unsigned_body_hash: &str,
    tx: &dyn TransactionSign,
    profile: &CodecProfile,
) -> String {
    let sign_hash = review
        .signer_address
        .as_deref()
        .and_then(|raw| Address::from_readable(raw).ok())
        .map(|signer| hex::encode(protocol::tx_std::sign_hash_for(tx, &signer).0));
    let context = match &review.inspect_context {
        Some(context) => context.to_json_string().into_bytes(),
        None => Vec::new(),
    };
    let review_digest = audit::canonical_review_digest(review);
    audit::compute_review_binding(
        unsigned_body_hash,
        review.signer_address.as_deref(),
        sign_hash.as_deref(),
        &profile.profile_hash,
        &context,
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
    options: &crate::audit::DescribeOptions,
) -> Result<Review, SdkError> {
    let body = decode_body_hex(body_hex)?;
    build_review(&body, signer_address, None, profile, options)
}

/// `tx.inspect`: strict mode — the caller-provided chain context is bound into the review
/// binding and evaluated into facts (`expired_height`/`wrong_chain`), never review denials.
pub fn inspect(
    body_hex: &str,
    signer_address: Option<&str>,
    context: &InspectContext,
    profile: &CodecProfile,
    options: &crate::audit::DescribeOptions,
) -> Result<Review, SdkError> {
    let body = decode_body_hex(body_hex)?;
    build_review(&body, signer_address, Some(context), profile, options)
}

pub(crate) fn decode_body_hex(body_hex: &str) -> Result<Vec<u8>, SdkError> {
    hex::decode(body_hex.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "body hex invalid"))
}

/// Strict hex decode into a fixed-size byte array (optional `0x` prefix);
/// the single helper for public keys/signatures/digests.
pub(crate) fn decode_hex_fixed<const N: usize>(
    raw: &str,
    code: SdkErrorCode,
    message: &'static str,
) -> Result<[u8; N], SdkError> {
    hex::decode(raw.trim_start_matches("0x").trim_start_matches("0X"))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SdkError::new(code, message))
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

pub fn decode_transaction_json(
    body_hex: &str,
    options: &crate::audit::DescribeOptions,
) -> Result<TransactionJson, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let mut actions = Vec::with_capacity(tx.action_count());
    for (index, action) in tx.actions().iter().enumerate() {
        actions.push(crate::audit::describe_action(
            action.as_ref(),
            index,
            &index.to_string(),
            0,
            options,
        ));
    }
    let signatures = tx
        .signs()
        .iter()
        .map(|sign| SignatureEntry {
            public_key: hex::encode(sign.publickey),
            signature: hex::encode(sign.signature),
        })
        .collect();
    Ok(TransactionJson {
        schema: SCHEMA_TRANSACTION_JSON.to_owned(),
        tx_type: tx.ty(),
        timestamp: tx.timestamp().value(),
        main: tx.main().to_readable(),
        fee: tx.fee().to_fin_string(),
        gas_max: tx.gas_max_byte(),
        tx_hash: hex::encode(tx.hash().0),
        hash_with_fee: hex::encode(tx.hash_with_fee().0),
        unsigned_body_hash: crate::audit::unsigned_body_hash_bytes(&body)?,
        actions,
        signatures,
    })
}

/// `tx.encode`: rebuild a body from `tx.decode` output (doc 14 §6.4), re-validating action
/// json/signatures. The rebuilt body must reproduce the declared `unsigned_body_hash` (fails with `transaction_json_mismatch`), and a provided review must cover it.
pub fn encode_transaction_json(
    transaction: &TransactionJson,
    review: Option<&Review>,
    profile: &CodecProfile,
) -> Result<crate::build::BuiltTransaction, SdkError> {
    use field::Sign;

    if transaction.schema != SCHEMA_TRANSACTION_JSON {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!(
                "unsupported transaction json schema {:?}",
                transaction.schema
            ),
        ));
    }
    let codecs = standard_codecs().map_err(SdkError::from)?;

    let main = Address::from_readable(&transaction.main).map_err(SdkError::from)?;
    let fee = field::Amount::from(&transaction.fee).map_err(SdkError::from)?;
    let fee_fin = fee.to_fin_string();
    let mut actions = Vec::with_capacity(transaction.actions.len());
    for desc in &transaction.actions {
        // Re-encode from the wire form (the wasm core is JSON-free; the action
        // carrier in `TransactionJson` is `raw` = wire hex).
        let wire = hex::decode(&desc.raw)
            .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "action raw must be wire hex"))?;
        let action = codecs
            .decode_action_exact(&wire)
            .map_err(SdkError::from)
            .map_err(|e| {
                SdkError::with_detail(
                    SdkErrorCode::UnsupportedSchema,
                    format!("action kind {} failed to re-encode from wire", desc.kind),
                    crate::json::obj(vec![
                        crate::json::kv("action_index", desc.index.to_string()),
                        crate::json::kv("action_kind", desc.kind.to_string()),
                        crate::json::kv("error", crate::json::q(&e.message)),
                    ]),
                )
            })?;
        actions.push(action);
    }
    let mut signs = Vec::with_capacity(transaction.signatures.len());
    for entry in &transaction.signatures {
        let publickey: [u8; 33] = decode_hex_fixed(
            &entry.public_key,
            SdkErrorCode::InvalidPublicKey,
            "signature public key must be 33-byte hex",
        )?;
        let signature: [u8; 64] = decode_hex_fixed(
            &entry.signature,
            SdkErrorCode::BadSignature,
            "signature must be 64-byte hex",
        )?;
        // Duplicate keys are not judged here: insertion replaces same-key
        // entries, and the chain decides signer-set acceptance at verify time.
        signs.push(Sign {
            publickey,
            signature,
        });
    }

    // Build via the protocol's standard-tx constructor (signatures inserted
    // mechanically), then re-decode for the round-trip invariant.
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
    // Integrity gate: the rebuilt body must reproduce the declared hash,
    // or action-json tampering would silently emit a different transaction.
    if unsigned_body_hash != transaction.unsigned_body_hash {
        return Err(SdkError::with_detail(
            SdkErrorCode::TransactionJsonMismatch,
            "rebuilt body does not match the declared unsigned_body_hash",
            crate::json::obj(vec![
                crate::json::kv("expected", crate::json::q(&transaction.unsigned_body_hash)),
                crate::json::kv("actual", crate::json::q(&unsigned_body_hash)),
            ]),
        ));
    }
    // When a review is supplied, its binding must cover the rebuilt body
    // (same chain as attach_signature); no signer is bound on this path.
    if let Some(review) = review {
        crate::attach::verify_review(review, &unsigned_body_hash, None, decoded.as_ref(), profile)?;
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
