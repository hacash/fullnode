//! Action descriptors and review bindings (Unified SDK 2.0, doc 14 §5/§6).
//!
//! `ActionDesc` is the UI contract for one action: canonical json, raw bytes,
//! auditability and notes. The review binding is a sha3-256 over explicit
//! domain-prefixed fields; it never depends on localized text or the binding
//! itself.

use base::{Action, ActionRef};
use field::Decode;
use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};
use crate::names::action_name;
use crate::profile::AST_DEPTH_MAX;
pub use crate::schema::{
    DOMAIN_REVIEW_BINDING, DOMAIN_UNSIGNED_BODY, SCHEMA_ACTION_DESC, SCHEMA_TRANSFER_DESC,
};

/// Tagged asset payload (doc 14 §4.7): HAC/SAT/HACD keep their distinct wire
/// shapes; only protocol Asset collapses to serial + atoms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PayloadDesc {
    Hac { amount: String },
    Satoshi { atoms: String },
    Hacd { count: u32, names: Vec<String> },
    Asset { serial: String, atoms: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferDesc {
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub payload: PayloadDesc,
}

/// Auditability grades (plan 13 §1, doc 14 §4.8). `branching`/`opaque` are
/// audit facts, never decode failures and never automatic signing denials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auditability {
    Full,
    Structured,
    Branching,
    Opaque,
}

impl Auditability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Auditability::Full => "full",
            Auditability::Structured => "structured",
            Auditability::Branching => "branching",
            Auditability::Opaque => "opaque",
        }
    }

    /// Worst of two grades (used to lift the top-level review grade).
    pub fn worse(a: Auditability, b: Auditability) -> Auditability {
        use Auditability::*;
        match (a, b) {
            (Opaque, _) | (_, Opaque) => Opaque,
            (Branching, _) | (_, Branching) => Branching,
            (Structured, _) | (_, Structured) => Structured,
            _ => Full,
        }
    }
}

/// Classify an action kind into an auditability grade. Full by default;
/// AST control flow is branching, VM maincall is opaque, the remaining VM
/// actions are structured (codec/bytecode present, execution not run here).
pub fn classify_auditability(kind: u16) -> Auditability {
    use protocol::action_std::{AstIf, AstSelect};
    use vm::action::{ContractDeploy, ContractMainCall, ContractUpdate, P2SHScriptProve};
    match kind {
        AstIf::KIND | AstSelect::KIND => Auditability::Branching,
        ContractMainCall::KIND => Auditability::Opaque,
        ContractDeploy::KIND | ContractUpdate::KIND | P2SHScriptProve::KIND => {
            Auditability::Structured
        }
        _ => Auditability::Full,
    }
}

pub fn scope_name(scope: base::ActScope) -> &'static str {
    use base::ActScope;
    if scope == ActScope::GUARD || scope == ActScope::TOP_GUARD_UNIQUE {
        "guard"
    } else if scope == ActScope::CALL || scope == ActScope::CALL_ONLY {
        "call"
    } else if scope == ActScope::AST {
        "ast"
    } else {
        "top"
    }
}

fn payload_desc(action: &dyn Action) -> Option<PayloadDesc> {
    let transfer = action.as_transfer_like()?;
    match transfer.transfer_payload() {
        base::TransferPayload::Hac { .. } => Some(PayloadDesc::Hac {
            amount: transfer.transfer_amount().to_fin_string(),
        }),
        base::TransferPayload::Sat { satoshi } => Some(PayloadDesc::Satoshi {
            atoms: satoshi.to_string(),
        }),
        base::TransferPayload::Hacd { count, names } => Some(PayloadDesc::Hacd {
            count,
            names: readable_diamond_names(&names),
        }),
        base::TransferPayload::Asset { serial, amount } => Some(PayloadDesc::Asset {
            serial: serial.to_string(),
            atoms: amount.to_string(),
        }),
    }
}

/// Diamond names ride the wire in their packed form; decode them for display.
/// Unreadable payloads degrade to an empty list rather than failing the whole
/// descriptor (the raw bytes stay available in `ActionDesc.raw`).
fn readable_diamond_names(names: &[u8]) -> Vec<String> {
    if let Ok((list, used)) = <field::DiamondNameListMax200 as Decode>::decode(names) {
        if used == names.len() {
            return list.as_list().iter().map(|name| name.to_readable()).collect();
        }
    }
    Vec::new()
}

/// One action in the review tree. `path` is the nested index path ("0", "1/2").
/// AST control-flow children are collected into `children` when present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDesc {
    pub schema: String,
    pub index: usize,
    pub path: String,
    pub kind: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub scope: String,
    pub json: String,
    pub raw: String,
    pub protocol_valid: bool,
    pub auditability: String,
    pub audit_notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferDesc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ActionDesc>>,
}

pub fn describe_action(action: &ActionRef, index: usize, path: &str, depth: usize) -> ActionDesc {
    let kind = action.kind();
    let mut notes = Vec::new();
    let children = collect_children(action, depth, &mut notes);
    let auditability = classify_auditability(kind);
    let transfer = action.as_transfer_like().map(|transfer| TransferDesc {
        schema: SCHEMA_TRANSFER_DESC.to_owned(),
        from: transfer.transfer_from().and_then(|from| match from {
            base::AddrOrPtr::Addr(addr) => Some(addr.to_readable()),
            // Address-table pointers resolve against the tx addrlist; a
            // pointer without the list is left unresolved in M1.
            base::AddrOrPtr::Ptr(_) => None,
        }),
        to: transfer.transfer_to().to_readable(),
        payload: payload_desc(action.as_ref()).unwrap_or(PayloadDesc::Hac {
            amount: transfer.transfer_amount().to_fin_string(),
        }),
    });
    ActionDesc {
        schema: SCHEMA_ACTION_DESC.to_owned(),
        index,
        path: path.to_owned(),
        kind,
        name: action_name(kind).map(str::to_owned),
        scope: scope_name(action.scope()).to_owned(),
        json: action.to_json(),
        raw: hex::encode(action.encode()),
        protocol_valid: true,
        auditability: auditability.as_str().to_owned(),
        audit_notes: notes,
        transfer,
        children,
    }
}

/// Collect nested control-flow children (AstIf → cond/if/else AstSelects,
/// AstSelect → flat action list) with the protocol depth cap. Depth overflow
/// is reported as an audit note, never as a decode failure.
fn collect_children(
    action: &ActionRef,
    depth: usize,
    notes: &mut Vec<String>,
) -> Option<Vec<ActionDesc>> {
    use protocol::action_std::{AstIf, AstSelect};
    if depth >= AST_DEPTH_MAX {
        notes.push(format!(
            "nested AST depth exceeds protocol maximum {}",
            AST_DEPTH_MAX
        ));
        return None;
    }
    let mut list = Vec::new();
    if let Some(ast) = action.as_any().downcast_ref::<AstIf>() {
        for (branch_idx, select) in [&ast.cond, &ast.br_if, &ast.br_else]
            .into_iter()
            .enumerate()
        {
            for (idx, nested) in select.actions.as_list().iter().enumerate() {
                let path = format!("{}/{}", branch_idx, idx);
                list.push(describe_action(nested, idx, &path, depth + 1));
            }
        }
        return Some(list);
    }
    if let Some(select) = action.as_any().downcast_ref::<AstSelect>() {
        for (idx, nested) in select.actions.as_list().iter().enumerate() {
            list.push(describe_action(nested, idx, &idx.to_string(), depth + 1));
        }
        return Some(list);
    }
    None
}

/// sha3-256 digest of the canonical unsigned body (signature set removed).
/// Domain-frozen at ABI major 2; stable across SDK releases (doc 14 §6.2).
pub fn unsigned_body_hash(body_hex: &str) -> Result<String, SdkError> {
    let body = hex::decode(body_hex)
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "body hex invalid"))?;
    let tx = crate::inspect::decode_tx(&body)?;
    let unsigned = crate::inspect::encode_without_signs(tx.as_ref())?;
    let mut data = Vec::with_capacity(DOMAIN_UNSIGNED_BODY.len() + unsigned.len());
    data.extend_from_slice(DOMAIN_UNSIGNED_BODY);
    data.extend_from_slice(&unsigned);
    Ok(hex::encode(sys::calculate_hash(data)))
}

/// sha3-256 over domain + unsigned_body_hash + signer + sign_hash +
/// codec_profile_hash + inspect context + canonical review digest. The review
/// digest excludes `review_binding` itself and non-deterministic display text.
pub fn compute_review_binding(
    unsigned_body_hash: &str,
    signer: Option<&str>,
    sign_hash: Option<&str>,
    codec_profile_hash: &str,
    context_json: &str,
    review_digest: &str,
) -> String {
    let mut data = Vec::new();
    data.extend_from_slice(DOMAIN_REVIEW_BINDING);
    data.extend_from_slice(unsigned_body_hash.as_bytes());
    data.push(0);
    data.extend_from_slice(signer.unwrap_or("").as_bytes());
    data.push(0);
    data.extend_from_slice(sign_hash.unwrap_or("").as_bytes());
    data.push(0);
    data.extend_from_slice(codec_profile_hash.as_bytes());
    data.push(0);
    data.extend_from_slice(context_json.as_bytes());
    data.push(0);
    data.extend_from_slice(review_digest.as_bytes());
    hex::encode(sys::calculate_hash(data))
}

/// Canonical digest of a review payload: the review JSON with
/// `review_binding` removed (deterministic for identical review content).
pub fn canonical_review_digest(review: &serde_json::Value) -> String {
    let mut copy = review.clone();
    if let Some(obj) = copy.as_object_mut() {
        obj.remove("review_binding");
    }
    let canonical = serde_json::to_string(&copy).unwrap_or_default();
    hex::encode(sys::calculate_hash(canonical))
}
