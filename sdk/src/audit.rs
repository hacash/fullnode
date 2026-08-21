//! Action descriptors and review bindings (Unified SDK 2.0, doc 14 §5/§6).
//! Auditability classes are schema-declared at each action's definition site, so the SDK never keeps a separate grading table.

use base::Action;
use field::Decode;

use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::profile::AST_DEPTH_MAX;
pub use crate::schema::{
    DOMAIN_REVIEW_BINDING, DOMAIN_UNSIGNED_BODY, SCHEMA_ACTION_DESC, SCHEMA_TRANSFER_DESC,
};

/// Tagged asset payload (doc 14 §4.7): HAC/SAT/HACD keep their distinct wire
/// shapes; only protocol Asset collapses to serial + atoms.
#[derive(Debug, Clone, PartialEq)]
pub enum PayloadDesc {
    Hac { amount: String },
    Satoshi { atoms: String },
    Hacd { count: u32, names: Vec<String> },
    Asset { serial: String, atoms: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransferDesc {
    pub schema: String,
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

/// Graded action facts (by schema — the stable wire identity); classes and the blob flag are
/// schema-declared, and `classify_auditability` fails closed for any unregistered name.
fn schema_of(name: &str) -> Option<&'static base::ActionSchema> {
    crate::selection::action_schema_named(name)
}

/// Classify an action kind by its schema-declared class; unregistered kinds
/// default to opaque with a note (fail-closed).
pub fn classify_auditability(name: &str) -> (Auditability, Option<&'static str>) {
    match schema_of(name).map(|schema| schema.audit_class) {
        Some(base::AuditClass::Full) => (Auditability::Full, None),
        Some(base::AuditClass::Structured) => (Auditability::Structured, None),
        Some(base::AuditClass::Branching) => (Auditability::Branching, None),
        Some(base::AuditClass::Opaque) => (Auditability::Opaque, None),
        _ => (
            Auditability::Opaque,
            Some("action kind is not graded by the SDK audit table; treat as opaque"),
        ),
    }
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

/// Diamond names ride the wire packed; decode for display. Unreadable payloads
/// degrade to an empty list rather than failing the descriptor.
fn readable_diamond_names(names: &[u8]) -> Vec<String> {
    if let Ok((list, used)) = <field::DiamondNameListMax200 as Decode>::decode(names) {
        if used == names.len() {
            return list
                .as_list()
                .iter()
                .map(|name| name.to_readable())
                .collect();
        }
    }
    Vec::new()
}

/// One action in the review tree. `path` is the nested index path ("0", "1/2").
/// AST control-flow children are collected into `children` when present.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionDesc {
    pub schema: String,
    pub index: usize,
    pub path: String,
    pub kind: u16,
    pub name: Option<String>,
    pub scope: String,
    pub raw: String,
    pub protocol_valid: bool,
    pub auditability: String,
    pub audit_notes: Vec<String>,
    pub transfer: Option<TransferDesc>,
    pub children: Option<Vec<ActionDesc>>,
    /// Schema-declared blob fact (opaque byte-carrier action such as
    /// `tx_message`/`tx_blob`); `policy.deny_blob` reads it directly.
    pub blob: bool,
}

pub fn describe_action(action: &dyn Action, index: usize, path: &str, depth: usize) -> ActionDesc {
    let kind = action.kind();
    let mut notes = Vec::new();
    let children = collect_children(action, depth, &mut notes);
    let name = crate::selection::action_schema(kind).map(|schema| schema.name);
    let (auditability, grade_note) = classify_auditability(name.unwrap_or(""));
    if let Some(note) = grade_note {
        notes.push(note.to_owned());
    }
    let transfer = action.as_transfer_like().map(|transfer| TransferDesc {
        schema: SCHEMA_TRANSFER_DESC.to_owned(),
        from: transfer.transfer_from().and_then(|from| match from {
            base::AddrOrPtr::Addr(addr) => Some(addr.to_readable()),
            // Address-table pointers resolve against the tx addrlist; a
            // pointer without the list is left unresolved in M1.
            base::AddrOrPtr::Ptr(_) => None,
        }),
        to: transfer.transfer_to().to_readable(),
        payload: payload_desc(action).unwrap_or(PayloadDesc::Hac {
            amount: transfer.transfer_amount().to_fin_string(),
        }),
    });
    ActionDesc {
        schema: SCHEMA_ACTION_DESC.to_owned(),
        index,
        path: path.to_owned(),
        kind,
        name: name.map(str::to_owned),
        scope: scope_name(action.scope()).to_owned(),
        raw: hex::encode(action.encode()),
        protocol_valid: true,
        auditability: auditability.as_str().to_owned(),
        audit_notes: notes,
        transfer,
        children,
        blob: crate::selection::action_schema(kind)
            .map(|schema| schema.blob)
            .unwrap_or(false),
    }
}

/// Collect nested control-flow children via `Action::nested_actions`; depth overflow or a
/// schema-declared `branching` action without a walker is an audit note (fail-closed), never a decode failure.
fn collect_children(
    action: &dyn Action,
    depth: usize,
    notes: &mut Vec<String>,
) -> Option<Vec<ActionDesc>> {
    match action.nested_actions() {
        Some(nested) => {
            let next_depth = match depth.checked_add(nested.depth_inc) {
                Some(d) => d,
                None => {
                    notes.push("ast tree depth overflow".to_owned());
                    return None;
                }
            };
            if next_depth > AST_DEPTH_MAX {
                notes.push(format!(
                    "nested AST depth exceeds protocol maximum {}",
                    AST_DEPTH_MAX
                ));
                return None;
            }
            let multi = nested.branches.len() > 1;
            let mut list = Vec::new();
            for (branch_idx, branch) in nested.branches.iter().enumerate() {
                for (idx, child) in branch.iter().enumerate() {
                    let path = if multi {
                        format!("{branch_idx}/{idx}")
                    } else {
                        idx.to_string()
                    };
                    list.push(describe_action(*child, idx, &path, next_depth));
                }
            }
            Some(list)
        }
        None => {
            if crate::selection::action_schema(action.kind()).map(|s| s.audit_class)
                == Some(base::AuditClass::Branching)
            {
                notes.push(format!(
                    "branching action kind {} has no nested_actions walker",
                    action.kind()
                ));
            }
            None
        }
    }
}

/// sha3-256 digest of the canonical unsigned body (signature set removed).
/// Domain-frozen at ABI major 2 (doc 14 §6.2); the single computation every path funnels through.
pub fn unsigned_body_hash_bytes(body: &[u8]) -> Result<String, SdkError> {
    let tx = crate::inspect::decode_tx(body)?;
    let unsigned = crate::inspect::encode_without_signs(tx.as_ref())?;
    let mut data = Vec::with_capacity(DOMAIN_UNSIGNED_BODY.len() + unsigned.len());
    data.extend_from_slice(DOMAIN_UNSIGNED_BODY);
    data.extend_from_slice(&unsigned);
    Ok(hex::encode(sys::calculate_hash(data)))
}

/// sha3-256 digest of the canonical unsigned body (hex body input; the `0x`
/// prefix is tolerated like `decode_body_hex`).
pub fn unsigned_body_hash(body_hex: &str) -> Result<String, SdkError> {
    let body = hex::decode(body_hex.trim_start_matches("0x").trim_start_matches("0X"))
        .map_err(|_| SdkError::new(SdkErrorCode::ParseFailed, "body hex invalid"))?;
    unsigned_body_hash_bytes(&body)
}

/// sha3-256 over domain + unsigned_body_hash + signer + sign_hash + codec_profile_hash +
/// inspect context + canonical review digest (excludes `review_binding` and non-deterministic display text).
pub fn compute_review_binding(
    unsigned_body_hash: &str,
    signer: Option<&str>,
    sign_hash: Option<&str>,
    codec_profile_hash: &str,
    context: &[u8],
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
    data.extend_from_slice(context);
    data.push(0);
    data.extend_from_slice(review_digest.as_bytes());
    hex::encode(sys::calculate_hash(data))
}

/// Canonical digest of a review payload: the review JSON with `review_binding`
/// removed; hand-written serialization (field order + skip None), no serde_json.
pub fn canonical_review_digest(review: &crate::inspect::Review) -> String {
    let mut copy = review.clone();
    copy.review_binding.clear();
    let canonical = copy.to_json_string();
    hex::encode(sys::calculate_hash(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registered action kind must grade through its schema-declared
    /// class, so none falls back to the ungraded note.
    #[test]
    fn every_registered_kind_has_a_schema_declared_audit_class() {
        let names: Vec<&str> = crate::selection::action_schemas()
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names.len(), 35, "SDK capability profile changed");
        for name in &names {
            let (_, note) = classify_auditability(name);
            assert!(
                note.is_none(),
                "action {name} has no schema-declared audit class"
            );
        }
    }

    /// The schema-declared blob flag must be exactly the two opaque
    /// byte-carrier actions; anything else fails here.
    #[test]
    fn blob_class_is_exactly_tx_message_and_tx_blob() {
        let schemas = crate::selection::action_schemas();
        let blobs: Vec<&str> = schemas.iter().filter(|s| s.blob).map(|s| s.name).collect();
        assert_eq!(blobs, vec!["tx_message", "tx_blob"]);
    }

    #[test]
    fn known_grades_are_stable() {
        assert_eq!(
            classify_auditability("transfer_hac_to"),
            (Auditability::Full, None)
        );
        assert_eq!(
            classify_auditability("ast_select"),
            (Auditability::Branching, None)
        );
        assert_eq!(
            classify_auditability("contract_deploy"),
            (Auditability::Structured, None)
        );
        assert_eq!(
            classify_auditability("contract_main_call"),
            (Auditability::Opaque, None)
        );
        assert!(classify_auditability("brand_new_kind").0 == Auditability::Opaque);
    }

    /// The AST control-flow walkers must stay complete over the registered branching actions
    /// (`ast_select`/`ast_if`); a missing walker is a fail-closed note, not a silent child drop.
    #[test]
    fn ast_control_flow_kinds_collect_children() {
        use protocol::action_std::{AstIf, AstSelect, HacToTrs};
        let transfer = std::sync::Arc::new(HacToTrs::new(
            field::Address::from(*sys::Account::create_by("123456").unwrap().address()),
            field::Amount::from("1:244").unwrap(),
        ));
        let select = AstSelect::create_by(1, 1, vec![transfer.clone()]).unwrap();
        let select: base::ActionRef = std::sync::Arc::new(select);
        let desc = describe_action(select.as_ref(), 0, "0", 0);
        let children = desc.children.expect("ast_select collects children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].path, "0");
        assert_eq!(children[0].name.as_deref(), Some("transfer_hac_to"));

        let ast_if = AstIf::create_by(
            AstSelect::create_by(1, 1, vec![transfer.clone()]).unwrap(),
            AstSelect::create_by(1, 1, vec![transfer.clone()]).unwrap(),
            AstSelect::create_by(1, 1, vec![transfer.clone()]).unwrap(),
        );
        let ast_if: base::ActionRef = std::sync::Arc::new(ast_if);
        let desc = describe_action(ast_if.as_ref(), 0, "0", 0);
        let children = desc.children.expect("ast_if collects children");
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].path, "0/0");
        assert_eq!(children[2].path, "2/0");
    }

    /// `ActScope::AST` is shared by non-control-flow actions; the fail-closed
    /// note must key off the `branching` class, never the scope constant.
    #[test]
    fn non_branching_ast_scope_actions_have_no_missing_walker_note() {
        let maincall: base::ActionRef = std::sync::Arc::new(vm::action::ContractMainCall::new());
        let desc = describe_action(maincall.as_ref(), 0, "0", 0);
        assert!(desc.children.is_none());
        assert!(
            desc.audit_notes
                .iter()
                .all(|n| !n.contains("nested_actions") && !n.contains("children collection")),
            "spurious walker note on a non-branching action: {:?}",
            desc.audit_notes
        );
    }
}
