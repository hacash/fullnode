//! Transaction inspection: decode → protocol facts → Review (Unified SDK 2.0,
//! doc 14 §5/§6.1). The SDK never executes transactions and never consults a
//! node; the chain context is caller input.

use base::{BinaryCodecs, JsonCodecs, Transaction};
use field::{Address, Encode};
use serde::{Deserialize, Serialize};

use crate::audit::{self, Auditability};
use crate::codec::standard_codecs;
use crate::error::{SdkError, SdkErrorCode};
use crate::profile::{self, CodecProfile};
use crate::schema::{SCHEMA_REVIEW, SCHEMA_TRANSACTION_JSON};

/// Chain context for strict inspection (doc 14 §5 `tx.inspect`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectContext {
    pub current_height: u64,
    pub expected_chain_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeightRangeDesc {
    pub start: u64,
    pub end: u64,
}

/// The review object (doc 14 §6.1). Contains protocol facts and audit
/// material only; no policy decision, no bridge fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub schema: String,
    pub codec_profile_hash: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_max: Option<u8>,
    pub tx_hash: String,
    pub hash_with_fee: String,
    pub unsigned_body_hash: String,
    pub review_binding: String,
    pub protocol_valid: bool,
    pub signability: String,
    pub auditability: String,
    pub requires_user_confirmation: bool,
    pub required_signers: Vec<String>,
    pub present_signers: Vec<String>,
    pub missing_signers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_ids_allowed: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_height_range: Option<HeightRangeDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_purity: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_purity_ok: Option<bool>,
    pub actions: Vec<crate::audit::ActionDesc>,
    pub asset_serials: Vec<u64>,
}

/// Decode a transaction body with the SDK codec registry and fail-closed
/// rules: size limit, unknown tx type, malformed wire, trailing bytes.
pub fn decode_tx(body: &[u8]) -> Result<base::TxRef, SdkError> {
    if body.len() > profile::MAX_TX_SIZE {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            format!(
                "tx body {} bytes exceeds protocol maximum {}",
                body.len(),
                profile::MAX_TX_SIZE
            ),
            serde_json::json!({ "byte_offset": body.len(), "expected": profile::MAX_TX_SIZE }),
        ));
    }
    let Some(&ty_byte) = body.first() else {
        return Err(SdkError::new(SdkErrorCode::ParseFailed, "tx body is empty"));
    };
    if !matches!(ty_byte, 1 | 2 | 3) {
        return Err(SdkError::with_detail(
            SdkErrorCode::ParseFailed,
            format!("unknown transaction type {ty_byte}"),
            serde_json::json!({ "byte_offset": 0 }),
        ));
    }
    let codecs = standard_codecs().map_err(SdkError::from)?;
    let (tx, used) = codecs
        .decode_transaction(body)
        .map_err(|error| SdkError::from(error))?;
    if used != body.len() {
        return Err(SdkError::with_detail(
            SdkErrorCode::TrailingBytes,
            format!("tx parse consumed {used} of {} bytes", body.len()),
            serde_json::json!({ "byte_offset": used, "actual": body.len() }),
        ));
    }
    if tx.action_count() > profile::TX_ACTIONS_MAX {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            format!(
                "tx action count {} exceeds protocol maximum {}",
                tx.action_count(),
                profile::TX_ACTIONS_MAX
            ),
            serde_json::json!({ "expected": profile::TX_ACTIONS_MAX }),
        ));
    }
    Ok(tx)
}

/// Re-encode the transaction with its signature set cleared. Used for the
/// stable `unsigned_body_hash`; Type-2/3 wire order is preserved exactly.
pub fn encode_without_signs(tx: &dyn Transaction) -> Result<Vec<u8>, SdkError> {
    use protocol::tx_std::{TransactionType1, TransactionType2, TransactionType3};
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType1>() {
        let mut copy = t.clone();
        copy.signs = field::ListW2::<field::Sign>::default();
        return Ok(copy.encode());
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType2>() {
        let mut copy = t.clone();
        copy.signs = field::ListW2::<field::Sign>::default();
        return Ok(copy.encode());
    }
    if let Some(t) = tx.as_any().downcast_ref::<TransactionType3>() {
        let mut copy = t.clone();
        copy.signs = field::ListW2::<field::Sign>::default();
        return Ok(copy.encode());
    }
    Err(SdkError::new(
        SdkErrorCode::UnsupportedTxType,
        format!("transaction type {} has no unsigned-body form", tx.ty()),
    ))
}

/// Guard facts extracted from the action list: union of allowed chains,
/// intersected height range, per-action notes and protocol violations
/// (empty `ReqSignList`, malformed height range).
struct GuardFacts {
    chains: Vec<u32>,
    height_range: Option<HeightRangeDesc>,
    /// (action_index, note) pairs attached to the matching action descriptor.
    action_notes: Vec<(usize, String)>,
    /// Protocol-level violations: signing must be rejected, decode stays ok.
    protocol_violations: Vec<String>,
}

fn collect_guard_facts(tx: &dyn Transaction) -> GuardFacts {
    use protocol::action_std::{ChainAllow, HeightScope, ReqSignList};
    let mut facts = GuardFacts {
        chains: Vec::new(),
        height_range: None,
        action_notes: Vec::new(),
        protocol_violations: Vec::new(),
    };
    for (index, action) in tx.actions().iter().enumerate() {
        if let Some(chain_allow) = action.as_any().downcast_ref::<ChainAllow>() {
            for id in chain_allow.chains.as_list() {
                if !facts.chains.contains(&id.uint()) {
                    facts.chains.push(id.uint());
                }
            }
        } else if let Some(scope) = action.as_any().downcast_ref::<HeightScope>() {
            let start = scope.start.uint();
            let end = scope.end.uint();
            if start > end && end != 0 {
                facts.protocol_violations.push(format!(
                    "height_scope left {start} exceeds right {end}"
                ));
                facts
                    .action_notes
                    .push((index, format!("height_scope left {start} exceeds right {end}")));
                continue;
            }
            let range = HeightRangeDesc {
                start,
                end: if end == 0 { u64::MAX } else { end },
            };
            facts.height_range = Some(match facts.height_range {
                None => range,
                Some(prev) => HeightRangeDesc {
                    start: prev.start.max(range.start),
                    end: prev.end.min(range.end),
                },
            });
        } else if let Some(req_sign) = action.as_any().downcast_ref::<ReqSignList>() {
            if req_sign.signers.is_empty() {
                let note = "req_sign_list with empty signer list is protocol-invalid".to_owned();
                facts.protocol_violations.push(note.clone());
                facts.action_notes.push((index, note));
            }
        }
    }
    facts
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
    context_json: &str,
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

    let sign_hash_for_signer = signer.map(|signer| {
        hex::encode(protocol::tx_std::sign_hash_for(tx.as_ref(), &signer).0)
    });

    let mut actions = Vec::with_capacity(tx.action_count());
    let mut asset_serials = Vec::new();
    let mut auditability = Auditability::Full;
    for (index, action) in tx.actions().iter().enumerate() {
        let mut desc = audit::describe_action(action, index, &index.to_string(), 0);
        if let Some((_, note)) = guard_facts.action_notes.iter().find(|(idx, _)| *idx == index) {
            desc.audit_notes.push(note.clone());
        }
        auditability = Auditability::worse(auditability, classify(&desc.auditability));
        collect_asset_serials(action, &mut asset_serials);
        actions.push(desc);
    }
    let requires_user_confirmation = auditability != Auditability::Full;

    let signability = if tx.ty() == 1 {
        "unsupported_tx_type".to_owned()
    } else {
        "signable".to_owned()
    };

    // Fee purity is a local protocol fact for Type-2; Type-3 purity depends on
    // runtime gas, reported as unknown rather than a fake fixed fact.
    let (fee_purity, fee_purity_ok) = if tx.ty() == 2 {
        let purity = tx.fee_purity();
        (
            Some(purity),
            Some(purity >= profile.protocol_params.fee_purity_floor),
        )
    } else {
        (None, None)
    };

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
        protocol_valid: guard_facts.protocol_violations.is_empty(),
        signability,
        auditability: auditability.as_str().to_owned(),
        requires_user_confirmation,
        required_signers: readable_signers(&report.required),
        present_signers: readable_signers(&report.present),
        missing_signers: readable_signers(&report.missing),
        chain_ids_allowed: if guard_facts.chains.is_empty() {
            None
        } else {
            Some(guard_facts.chains)
        },
        valid_height_range: guard_facts.height_range.map(|range| HeightRangeDesc {
            start: range.start,
            end: if range.end == u64::MAX { 0 } else { range.end },
        }),
        fee_purity,
        fee_purity_ok,
        actions,
        asset_serials,
    };

    // Bind the review: canonical digest excludes review_binding itself.
    let signer_readable = signer.as_ref().map(|addr| addr.to_readable());
    let review_value = serde_json::to_value(&review).map_err(SdkError::from)?;
    let review_digest = audit::canonical_review_digest(&review_value);
    let binding = audit::compute_review_binding(
        &unsigned_body_hash,
        signer_readable.as_deref(),
        sign_hash_for_signer.as_deref(),
        &profile.profile_hash,
        context_json,
        &review_digest,
    );
    review.review_binding = binding;
    Ok(review)
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
    build_review(&body, signer_address, "", profile)
}

/// `tx.inspect`: strict mode. Requires chain context and validates the
/// height/chain guards against it.
pub fn inspect(
    body_hex: &str,
    signer_address: Option<&str>,
    context: &InspectContext,
    profile: &CodecProfile,
) -> Result<Review, SdkError> {
    let body = decode_body_hex(body_hex)?;
    let tx = decode_tx(&body)?;
    let context_json = serde_json::to_string(context).map_err(SdkError::from)?;
    let review = build_review(&body, signer_address, &context_json, profile)?;
    // Strict guard checks against the caller-provided context.
    for action in tx.actions() {
        if let Some(scope) = action.as_any().downcast_ref::<protocol::action_std::HeightScope>() {
            let left = scope.start.uint();
            let right = match scope.end.uint() {
                0 => u64::MAX,
                h => h,
            };
            if left > right {
                return Err(SdkError::new(
                    SdkErrorCode::ParseFailed,
                    format!("height_scope left {left} exceeds right {right}"),
                ));
            }
            let height = context.current_height;
            if height < left || height > right {
                return Err(SdkError::with_detail(
                    SdkErrorCode::ExpiredHeight,
                    format!(
                        "current height {height} outside allowed range ({left}, {right})"
                    ),
                    serde_json::json!({
                        "expected": format!("{}..{}", left, right),
                        "actual": height,
                    }),
                ));
            }
        } else if let Some(chain_allow) =
            action.as_any().downcast_ref::<protocol::action_std::ChainAllow>()
        {
            let allowed: Vec<u32> = chain_allow.chains.as_list().iter().map(|id| id.uint()).collect();
            if !allowed.contains(&context.expected_chain_id) {
                return Err(SdkError::with_detail(
                    SdkErrorCode::WrongChainId,
                    format!(
                        "expected chain {} not in allowed chains {:?}",
                        context.expected_chain_id, allowed
                    ),
                    serde_json::json!({
                        "expected": allowed,
                        "actual": context.expected_chain_id,
                    }),
                ));
            }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureEntry {
    pub public_key: String,
    pub signature: String,
}

/// `tx.decode`: strict structured codec output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionJson {
    pub schema: String,
    pub tx_type: u8,
    pub timestamp: u64,
    pub main: String,
    pub fee: String,
    #[serde(skip_serializing_if = "Option::is_none")]
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
/// duplicates and limits, then re-encodes. The round-trip invariant
/// `encode(decode(body)) == body` holds for every registered action whose
/// json codec round-trips; anything else fails with `UnsupportedSchema`.
pub fn encode_transaction_json(
    transaction: &TransactionJson,
    expected_review_binding: Option<&str>,
    profile: &CodecProfile,
) -> Result<crate::build::BuiltTransaction, SdkError> {
    use base::TransactionBuild;
    use field::Sign;

    if transaction.schema != SCHEMA_TRANSACTION_JSON {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedSchema,
            format!("unsupported transaction json schema {:?}", transaction.schema),
        ));
    }
    if !matches!(transaction.tx_type, 2 | 3) {
        return Err(SdkError::new(
            SdkErrorCode::UnsupportedTxType,
            format!("encode supports type 2/3 only, got {}", transaction.tx_type),
        ));
    }
    if transaction.actions.len() > profile::TX_ACTIONS_MAX {
        return Err(SdkError::with_detail(
            SdkErrorCode::LimitExceeded,
            "action count exceeds protocol maximum",
            serde_json::json!({ "expected": profile::TX_ACTIONS_MAX }),
        ));
    }
    if let Some(expected) = expected_review_binding {
        // Body-level binding over the unsigned body; full-context bindings are
        // enforced through prepare_signature (see attach_signature).
        let body_binding = audit::compute_review_binding(
            &transaction.unsigned_body_hash,
            None,
            None,
            &profile.profile_hash,
            "",
            "",
        );
        if body_binding != expected {
            return Err(SdkError::with_detail(
                SdkErrorCode::ReviewBindingMismatch,
                "expected review binding does not match this transaction json",
                serde_json::json!({ "expected": expected, "actual": body_binding }),
            ));
        }
    }

    let main = Address::from_readable(&transaction.main).map_err(SdkError::from)?;
    let fee = field::Amount::from(&transaction.fee).map_err(SdkError::from)?;
    let fee_fin = fee.to_fin_string();
    let codecs = standard_codecs().map_err(SdkError::from)?;
    let mut actions = Vec::with_capacity(transaction.actions.len());
    for desc in &transaction.actions {
        let action = codecs
            .decode_action_json(desc.kind, &desc.json)
            .map_err(SdkError::from)?
            .ok_or_else(|| {
                SdkError::with_detail(
                    SdkErrorCode::UnsupportedSchema,
                    format!("action kind {} has no json codec for re-encoding", desc.kind),
                    serde_json::json!({ "action_index": desc.index, "action_kind": desc.kind }),
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

    let (encoded, decoded) = if transaction.tx_type == 3 {
        let mut tx = protocol::tx_std::TransactionType3::new_by(main, fee, transaction.timestamp);
        if let Some(gas) = transaction.gas_max {
            tx.gas_max = field::Uint1::from(gas);
        }
        for action in actions {
            tx.push_action(action).map_err(SdkError::from)?;
        }
        for sign in signs {
            tx.push_sign(sign).map_err(SdkError::from)?;
        }
        let encoded = tx.encode();
        let decoded = decode_tx(&encoded)?;
        (encoded, decoded)
    } else {
        if transaction.gas_max.is_some_and(|gas| gas != 0) {
            return Err(SdkError::new(
                SdkErrorCode::ParseFailed,
                "type 2 transactions require gas_max = 0",
            ));
        }
        let mut tx = protocol::tx_std::TransactionType2::new_by(main, fee, transaction.timestamp);
        for action in actions {
            tx.push_action(action).map_err(SdkError::from)?;
        }
        for sign in signs {
            tx.push_sign(sign).map_err(SdkError::from)?;
        }
        let encoded = tx.encode();
        let decoded = decode_tx(&encoded)?;
        (encoded, decoded)
    };
    let body_hex = hex::encode(&encoded);
    // Round-trip invariant: rebuilding a decode output must reproduce it.
    if decoded.encode() != encoded {
        return Err(SdkError::new(
            SdkErrorCode::ParseFailed,
            "encoded body failed the round-trip invariant",
        ));
    }
    let unsigned_body_hash = crate::audit::unsigned_body_hash(&body_hex)?;
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
