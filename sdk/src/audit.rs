//! Action descriptors and review bindings (Unified SDK 2.0, doc 14 §5/§6).
//!
//! `ActionDesc` is the UI contract for one action: canonical json, raw bytes,
//! auditability and notes. The review binding is a sha3-256 over explicit
//! domain-prefixed fields; it never depends on localized text or the binding
//! itself.
//!
//! Auditability classes are declared at each action's definition site (the
//! `ActionSchemaProvider`/derive `audit_class` fact captured with the schema),
//! so the SDK's grading surface is the chain's definition surface: there is no
//! separate hand-written grading table here to drift from the action set.

use base::Action;
use field::Decode;

use crate::error::{SdkError, SdkErrorCode};
use crate::names::action_name;
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

/// Graded action facts (by schema — the stable wire identity). The classes and
/// the blob flag are declared at the action definition sites and captured with
/// the codec schemas (same registry as `names::action_name`), so a new action
/// carries its grade by construction; `classify_auditability` still fails
/// closed (opaque + note) for any name outside the registered set.
fn schema_of(name: &str) -> Option<&'static base::ActionSchema> {
    crate::codec::standard_codecs()
        .ok()?
        .action_schemas()
        .iter()
        .find(|schema| schema.name == name)
}

fn schema_of_kind(kind: u16) -> Option<&'static base::ActionSchema> {
    crate::codec::standard_codecs()
        .ok()?
        .action_schemas()
        .iter()
        .find(|schema| schema.kind == kind)
}

/// Classify an action kind into an auditability grade by its schema-declared
/// class. Defaults to opaque with an explanatory note (fail-closed): a kind
/// that has not been registered must not be presented as fully auditable.
pub fn classify_auditability(name: &str) -> (Auditability, Option<&'static str>) {
    match schema_of(name).map(|schema| schema.audit_class) {
        Some("full") => (Auditability::Full, None),
        Some("structured") => (Auditability::Structured, None),
        Some("branching") => (Auditability::Branching, None),
        Some("opaque") => (Auditability::Opaque, None),
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

/// Diamond names ride the wire in their packed form; decode them for display.
/// Unreadable payloads degrade to an empty list rather than failing the whole
/// descriptor (the raw bytes stay available in `ActionDesc.raw`).
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
    /// `tx_message`/`tx_blob`); `policy.deny_blob` reads this instead of a
    /// hand-written kind list.
    pub blob: bool,
}

pub fn describe_action(action: &dyn Action, index: usize, path: &str, depth: usize) -> ActionDesc {
    let kind = action.kind();
    let mut notes = Vec::new();
    let children = collect_children(action, depth, &mut notes);
    let name = action_name(kind);
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
        blob: schema_of_kind(kind)
            .map(|schema| schema.blob)
            .unwrap_or(false),
    }
}

/// Collect nested control-flow children through `Action::nested_actions` (the
/// same walker protocol topology analysis uses). Depth overflow is an audit
/// note, never a decode failure. A schema-declared `branching` action without
/// a walker is reported as a note too (fail-closed), so a new control-flow
/// kind never silently drops its children from the review. `ActScope::AST` is
/// not the signal: inscriptions and `contract_main_call` share that scope
/// without being control-flow.
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
            if schema_of_kind(action.kind()).map(|s| s.audit_class) == Some("branching") {
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

/// Canonical digest of a review payload: the review JSON with
/// `review_binding` removed (deterministic for identical review content).
/// Hand-written serialization (field declaration order + skip None), no serde_json.
pub fn canonical_review_digest(review: &crate::inspect::Review) -> String {
    let mut copy = review.clone();
    copy.review_binding.clear();
    let canonical = copy.to_binary_body();
    hex::encode(sys::calculate_hash(canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registered action kind must grade through the schema-declared
    /// class: a new action carries its class at the definition site, so no
    /// registered kind may fall back to the ungraded note.
    #[test]
    fn every_registered_kind_has_a_schema_declared_audit_class() {
        let names: Vec<&str> = chain_codec::collect_action_schemas()
            .iter()
            .map(|s| s.name)
            .collect();
        assert!(names.len() >= 40, "expected the full standard action set");
        for name in &names {
            let (_, note) = classify_auditability(name);
            assert!(
                note.is_none(),
                "action {name} has no schema-declared audit class"
            );
        }
    }

    /// The schema-declared blob flag must be exactly the two opaque
    /// byte-carrier actions: a new blob action without the flag (or a non-blob
    /// action with it) fails here.
    #[test]
    fn blob_class_is_exactly_tx_message_and_tx_blob() {
        let schemas = chain_codec::collect_action_schemas();
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

    /// The AST control-flow walkers must stay complete over the registered
    /// branching actions: `ast_select` and `ast_if` collect their nested
    /// children through `Action::nested_actions`. A new branching action
    /// without a walker is reported as a note (fail-closed) instead of
    /// silently dropping its children from the review.
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

    /// `ActScope::AST` is shared by inscriptions and `contract_main_call`,
    /// which are not control-flow. The fail-closed note must key off the
    /// schema-declared `branching` class (and a missing walker), never the
    /// scope constant.
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
