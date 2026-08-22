//! Action descriptors and review bindings (Unified SDK 2.0, doc 14 §5/§6).
//! Auditability classes are schema-declared at each action's definition site, so the SDK never keeps a separate grading table.

use base::{Action, BinaryCodecs};
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

/// Per-action describe knobs (Unified SDK 2.0 §6.5). Each switch independently
/// controls one output facet so callers can trim payload and decompile load:
/// `description` is the schema-declared one-line text, `json` the canonical
/// field-level JSON (can be large for contract deploy/update), `code` the VM
/// code metadata of code-carrying actions (`contract_main_call`, `p2sh`).
/// All default on; `tx.inspect` / `tx.decode` / `action.describe` accept a
/// `describe` object with any subset of these keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescribeOptions {
    pub with_description: bool,
    pub with_json: bool,
    pub with_code: bool,
}

impl Default for DescribeOptions {
    fn default() -> Self {
        Self {
            with_description: true,
            with_json: true,
            with_code: true,
        }
    }
}

impl DescribeOptions {
    pub(crate) fn from_json_pairs(pairs: &[(&str, &str)]) -> Result<Self, SdkError> {
        let mut opts = Self::default();
        for (key, value) in pairs {
            let parsed = match value.trim() {
                "true" => true,
                "false" => false,
                _ => {
                    return Err(crate::jsonparse::parse_failed(format!(
                        "describe field {key} must be a boolean"
                    )))
                }
            };
            match *key {
                "description" => opts.with_description = parsed,
                "json" => opts.with_json = parsed,
                "code" => opts.with_code = parsed,
                _ => {
                    return Err(SdkError::new(
                        SdkErrorCode::UnknownField,
                        format!("describe field {key} is unknown"),
                    ))
                }
            }
        }
        Ok(opts)
    }
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
    /// Schema-declared one-line description (`base::Action::description`),
    /// e.g. "Run main codes with conf 0". Omitted when `with_description=false`.
    pub description: Option<String>,
    /// Canonical field-level JSON (registry `to_json` view), e.g. the full
    /// contract structure for deploy/update. Omitted when `with_json=false`.
    pub json: Option<String>,
    /// VM code metadata for code-carrying actions (single-code payloads such
    /// as `contract_main_call` / `p2sh`); multi-code payloads (deploy/update)
    /// expose their per-function code through `json`. Omitted when
    /// `with_code=false` or the action carries no single code payload.
    pub code: Option<ActionCodeDesc>,
    pub transfer: Option<TransferDesc>,
    pub children: Option<Vec<ActionDesc>>,
    /// Schema-declared blob fact (opaque byte-carrier action such as
    /// `tx_message`/`tx_blob`); `policy.deny_blob` reads it directly.
    pub blob: bool,
}

/// VM code metadata of a code-carrying action (schema `hacash.sdk/action-desc@2`
/// `code` field). `codeconf` low bits are the code type (0 = bytecode,
/// 1 = ir_node); `codes_preview` is the first up-to-64 bytes hex (identity
/// without echoing a large payload).
#[derive(Debug, Clone, PartialEq)]
pub struct ActionCodeDesc {
    pub codeconf: u8,
    pub code_type: u8,
    pub code_type_name: String,
    pub codes_len: usize,
    pub codes_hash: String,
    pub codes_preview: String,
}

/// `code_type` names for the `codeconf` low bits (mirror of `vm::rt::CodeType`;
/// the constants are re-declared because `vm::rt` is crate-private).
pub fn code_type_name(raw: u8) -> (&'static str, u8) {
    match raw & crate::vm::CODECONF_TYPE_MASK {
        0 => ("bytecode", 0),
        1 => ("ir_node", 1),
        _ => ("invalid", 2),
    }
}

/// Build the code metadata of a decoded action, or `None` when the action
/// carries no single code payload (multi-code contract deploy/update expose
/// their structure through the canonical `json` view instead).
fn action_code_desc(action: &dyn Action) -> Option<ActionCodeDesc> {
    let (codeconf, codes): (u8, Vec<u8>) = if let Some(call) = action
        .as_any()
        .downcast_ref::<vm::action::ContractMainCall>()
    {
        (call.codeconf.uint(), call.codes.as_vec().clone())
    } else if let Some(p2sh) = action.as_any().downcast_ref::<vm::action::P2SHScriptProve>()
    {
        (p2sh.codeconf.uint(), p2sh.lockbox.as_vec().clone())
    } else {
        return None;
    };
    let (type_name, type_id) = code_type_name(codeconf);
    let preview_len = codes.len().min(64);
    Some(ActionCodeDesc {
        codeconf,
        code_type: type_id,
        code_type_name: type_name.to_owned(),
        codes_len: codes.len(),
        codes_hash: hex::encode(sys::calculate_hash(&codes)),
        codes_preview: hex::encode(&codes[..preview_len]),
    })
}

pub fn describe_action(
    action: &dyn Action,
    index: usize,
    path: &str,
    depth: usize,
    options: &DescribeOptions,
) -> ActionDesc {
    let kind = action.kind();
    let mut notes = Vec::new();
    let children = collect_children(action, depth, options, &mut notes);
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
    let description = if options.with_description {
        Some(action.description())
    } else {
        None
    };
    let json = if options.with_json {
        crate::codec::standard_codecs()
            .ok()
            .and_then(|codecs| codecs.action_json_to(kind))
            .map(|render| render(action, &field::JSONFormater::default()))
    } else {
        None
    };
    let code = if options.with_code {
        crate::selection::action_schema(kind)
            .map(|schema| schema.has_code)
            .unwrap_or(false)
            .then(|| action_code_desc(action))
            .flatten()
    } else {
        None
    };
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
        description,
        json,
        code,
        transfer,
        children,
        blob: crate::selection::action_schema(kind)
            .map(|schema| schema.blob)
            .unwrap_or(false),
    }
}

/// `action.describe`: describe a single raw action wire independently of any
/// transaction body — the on-demand detail entry for signature pages and
/// long-code viewers. Same `DescribeOptions` knobs as `tx.inspect`/`tx.decode`.
pub fn describe_single(action_hex: &str, options: &DescribeOptions) -> Result<ActionDesc, SdkError> {
    let wire = crate::inspect::decode_body_hex(action_hex)?;
    let codecs = crate::codec::standard_codecs().map_err(SdkError::from)?;
    let action = codecs
        .decode_action_exact(&wire)
        .map_err(SdkError::from)?;
    Ok(describe_action(action.as_ref(), 0, "0", 0, options))
}

/// Collect nested control-flow children via `Action::nested_actions`; depth overflow or a
/// schema-declared `branching` action without a walker is an audit note (fail-closed), never a decode failure.
fn collect_children(
    action: &dyn Action,
    depth: usize,
    options: &DescribeOptions,
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
                    list.push(describe_action(*child, idx, &path, next_depth, options));
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
        let desc = describe_action(select.as_ref(), 0, "0", 0, &DescribeOptions::default());
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
        let desc = describe_action(ast_if.as_ref(), 0, "0", 0, &DescribeOptions::default());
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
        let desc = describe_action(maincall.as_ref(), 0, "0", 0, &DescribeOptions::default());
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

// ================================ describe options / action.describe tests ================================

#[cfg(test)]
fn maincall_body() -> String {
    // Minimal bytecode: push 1, return (END). The codec-only SDK build has no
    // fitsh compiler; the code payload just needs to be wire-legal.
    let codes = vec![0x25u8, 0xef];
    let built = crate::build::build_transaction(&crate::build::TransactionSpec {
        schema: None,
        tx_type: 3,
        main: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9".to_owned(),
        fee: "1:244".to_owned(),
        timestamp: Some(1_755_223_764),
        gas_max: None,
        actions: vec![crate::build::ActionSpec::new(
            "contract_main_call",
            vec![
                ("marks".to_owned(), crate::spec_codec::WireValue::Hex(vec![0, 0, 0])),
                ("codeconf".to_owned(), crate::spec_codec::WireValue::Num(0)),
                ("codes".to_owned(), crate::spec_codec::WireValue::Hex(codes)),
            ],
        )],
    })
    .unwrap();
    let decoded =
        crate::inspect::decode_transaction_json(&built.body, &DescribeOptions::default()).unwrap();
    decoded.actions[0].raw.clone()
}

#[test]
fn describe_single_carries_description_json_and_code() {
    let raw = maincall_body();
    let desc = describe_single(&raw, &DescribeOptions::default()).unwrap();
    assert_eq!(desc.name.as_deref(), Some("contract_main_call"));
    assert!(!desc.scope.is_empty());
    // description: schema-declared one-liner.
    let text = desc.description.as_deref().unwrap();
    assert!(text.contains("Run main codes"), "description: {text}");
    // json: canonical field-level view.
    let json = desc.json.as_deref().unwrap();
    assert!(json.contains("\"kind\"") && json.contains("\"codes\""), "json: {json}");
    // code: single-code payload metadata (bytecode type 0).
    let code = desc.code.as_ref().expect("maincall has code metadata");
    assert_eq!(code.code_type_name, "bytecode");
    assert!(code.codes_len > 0);
    assert_eq!(code.codes_hash.len(), 64);
}

#[test]
fn describe_options_independently_trim_facets() {
    let raw = maincall_body();
    let bare = describe_single(&raw, &DescribeOptions {
        with_description: false,
        with_json: false,
        with_code: false,
    })
    .unwrap();
    assert!(bare.description.is_none());
    assert!(bare.json.is_none());
    assert!(bare.code.is_none());

    let code_only = describe_single(&raw, &DescribeOptions {
        with_description: false,
        with_json: false,
        with_code: true,
    })
    .unwrap();
    assert!(code_only.code.is_some());
    assert!(code_only.json.is_none());
}

#[test]
fn describe_options_reject_unknown_and_non_boolean() {
    let ok = DescribeOptions::from_json_pairs(&[("code", "false")]).unwrap();
    assert!(!ok.with_code);
    assert!(ok.with_description);
    let err = DescribeOptions::from_json_pairs(&[("bogus", "true")]).unwrap_err();
    assert_eq!(err.code, "unknown_field");
    let err = DescribeOptions::from_json_pairs(&[("json", "maybe")]).unwrap_err();
    assert_eq!(err.code, "parse_failed");
}

#[test]
fn action_desc_json_roundtrips_through_tx_encode() {
    // The tx.encode path parses ActionDesc@2 strictly; the new facets must
    // survive (optional fields default to None on the from side).
    let raw = maincall_body();
    let desc = describe_single(&raw, &DescribeOptions::default()).unwrap();
    let encoded = crate::json::SdkJsonTo::to_json_string(&desc);
    let back = <ActionDesc as crate::json::SdkJsonFrom>::from_json_str(&encoded).unwrap();
    assert_eq!(back.kind, desc.kind);
    assert_eq!(back.description, desc.description);
    assert_eq!(back.json, desc.json);
    assert_eq!(back.code, desc.code);
}
