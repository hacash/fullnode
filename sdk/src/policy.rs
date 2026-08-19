//! Generic application policy (Unified SDK 2.0, doc 14 §4.8). `evaluate`
//! consumes a Review and a caller-provided Policy and produces a
//! PolicyDecision; it never changes `protocol_valid`/`signability`.


use crate::error::SdkError;
use crate::inspect::Review;
use crate::schema::{DOMAIN_POLICY_DECISION, SCHEMA_POLICY, SCHEMA_POLICY_DECISION};

/// Caller-provided policy, frozen schema `hacash.sdk/policy@1` (doc 14 §4.8).
/// Absent fields take protocol defaults; nothing here is a product constant.
#[derive(Debug, Clone, Default)]
pub struct Policy {
        pub schema: Option<String>,
        pub deny_kinds: Option<Vec<u16>>,
        pub deny_blob: Option<bool>,
        pub max_diamond_names: Option<u32>,
        pub confirm_auditability: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDecision {
    pub schema: String,
    pub policy_id: String,
    pub policy_hash: String,
    pub review_binding: String,
    pub decision: String,
    pub findings: Vec<String>,
    pub policy_binding: String,
}

fn policy_hash(policy: &Policy) -> String {
    let json = policy.to_json_string();
    hex::encode(sys::calculate_hash(json))
}

/// Depth-first walk of the action review tree (children of AST control-flow
/// actions included): a policy that only inspects the top level would let
/// `ast_select`/`ast_if` wrap denied content around it.
fn walk_actions<'a>(actions: &'a [crate::audit::ActionDesc], visit: &mut impl FnMut(&'a crate::audit::ActionDesc)) {
    for action in actions {
        visit(action);
        if let Some(children) = &action.children {
            walk_actions(children, visit);
        }
    }
}

fn count_diamond_names(review: &Review) -> u32 {
    let mut total = 0u32;
    walk_actions(&review.actions, &mut |action| {
        if let Some(transfer) = &action.transfer {
            if let crate::audit::PayloadDesc::Hacd { count, .. } = &transfer.payload {
                total = total.saturating_add(*count);
            }
        }
    });
    total
}

/// `policy.evaluate`: apply the policy to a review. Default (empty) policy
/// allows; AST/TEX/opaque maincall never auto-deny (doc 14 §4.8). All checks
/// walk the action tree (children included), so nested actions are never a
/// bypass.
pub fn evaluate_policy(review: &Review, policy: &Policy) -> Result<PolicyDecision, SdkError> {
    if let Some(schema) = &policy.schema {
        if schema != SCHEMA_POLICY {
            return Err(SdkError::new(
                crate::error::SdkErrorCode::UnsupportedSchema,
                format!("unsupported policy schema {schema:?}"),
            ));
        }
    }
    let mut findings = Vec::new();
    let mut decision = "allow";

    if let Some(deny_kinds) = &policy.deny_kinds {
        walk_actions(&review.actions, &mut |action| {
            if deny_kinds.contains(&action.kind) {
                decision = "deny";
                findings.push(format!(
                    "action kind {} denied by policy (path {})",
                    action.kind, action.path
                ));
            }
        });
    }
    if policy.deny_blob.unwrap_or(false) {
        walk_actions(&review.actions, &mut |action| {
            if action.blob {
                decision = "deny";
                findings.push(format!(
                    "blob action {} denied by policy (path {})",
                    action.kind, action.path
                ));
            }
        });
    }
    if let Some(max_names) = policy.max_diamond_names {
        let count = count_diamond_names(review);
        if count > max_names {
            decision = "deny";
            findings.push(format!(
                "diamond count {count} exceeds policy maximum {max_names}"
            ));
        }
    }
    if let Some(confirm_grades) = &policy.confirm_auditability {
        if confirm_grades.contains(&review.auditability) {
            if decision == "allow" {
                decision = "confirm";
            }
            findings.push(format!(
                "auditability {} requires confirmation",
                review.auditability
            ));
        }
    }
    if decision == "allow" {
        findings.push("no policy restrictions matched".to_owned());
    }

    let hash = policy_hash(policy);
    let mut decision_obj = PolicyDecision {
        schema: SCHEMA_POLICY_DECISION.to_owned(),
        policy_id: hash.clone(),
        policy_hash: hash,
        review_binding: review.review_binding.clone(),
        decision: decision.to_owned(),
        findings,
        policy_binding: String::new(),
    };
    decision_obj.policy_binding = policy_binding_of(&decision_obj);
    Ok(decision_obj)
}

/// Canonical policy-decision binding (sha3-256 over the domain prefix and the
/// decision JSON minus `policy_binding` itself). Shared by `evaluate_policy`
/// and the verification side, so a decision whose fields (decision, findings,
/// review binding, policy hash) were edited after evaluation never
/// re-verifies.
pub fn policy_binding_of(decision: &PolicyDecision) -> String {
    let mut copy = decision.clone();
    copy.policy_binding.clear();
    let json = copy.to_json_string();
    let mut data = Vec::with_capacity(DOMAIN_POLICY_DECISION.len() + json.len());
    data.extend_from_slice(DOMAIN_POLICY_DECISION);
    data.extend_from_slice(json.as_bytes());
    hex::encode(sys::calculate_hash(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_review() -> Review {
        // A minimal review only needs the policy-relevant fields.
        Review {
            schema: crate::schema::SCHEMA_REVIEW.to_owned(),
            codec_profile_hash: "p".to_owned(),
            tx_type: 2,
            timestamp: 0,
            main: String::new(),
            fee: String::new(),
            gas_max: None,
            tx_hash: String::new(),
            hash_with_fee: String::new(),
            unsigned_body_hash: String::new(),
            review_binding: "rb".to_owned(),
            signer_address: None,
            inspect_context: None,
            protocol_valid: true,
            signability: "signable".to_owned(),
            limits_violations: vec![],
            auditability: "opaque".to_owned(),
            requires_user_confirmation: true,
            required_signers: vec![],
            present_signers: vec![],
            missing_signers: vec![],
            chain_ids_allowed: None,
            valid_height_range: None,
            fee_purity: None,
            fee_purity_ok: None,
            actions: vec![],
            asset_serials: vec![],
        }
    }

    #[test]
    fn default_policy_allows() {
        let decision = evaluate_policy(&sample_review(), &Policy::default()).unwrap();
        assert_eq!(decision.decision, "allow");
        assert!(!decision.policy_binding.is_empty());
    }

    #[test]
    fn confirm_auditability_lifts_to_confirm() {
        let policy = Policy {
            confirm_auditability: Some(vec!["opaque".to_owned()]),
            ..Default::default()
        };
        let decision = evaluate_policy(&sample_review(), &policy).unwrap();
        assert_eq!(decision.decision, "confirm");
    }

    #[test]
    fn deny_kinds_denies() {
        let policy = Policy {
            deny_kinds: Some(vec![44]),
            ..Default::default()
        };
        let mut review = sample_review();
        review.actions = vec![crate::audit::ActionDesc {
            schema: crate::schema::SCHEMA_ACTION_DESC.to_owned(),
            index: 0,
            path: "0".to_owned(),
            kind: 44,
            name: None,
            scope: "main".to_owned(),
            json: String::new(),
            raw: String::new(),
            protocol_valid: true,
            auditability: "opaque".to_owned(),
            audit_notes: vec![],
            transfer: None,
            children: None,
            blob: false,
        }];
        let decision = evaluate_policy(&review, &policy).unwrap();
        assert_eq!(decision.decision, "deny");
    }

    fn desc(kind: u16, path: &str, children: Option<Vec<crate::audit::ActionDesc>>) -> crate::audit::ActionDesc {
        crate::audit::ActionDesc {
            schema: crate::schema::SCHEMA_ACTION_DESC.to_owned(),
            index: 0,
            path: path.to_owned(),
            kind,
            name: None,
            scope: "main".to_owned(),
            json: String::new(),
            raw: String::new(),
            protocol_valid: true,
            auditability: "opaque".to_owned(),
            audit_notes: vec![],
            transfer: None,
            children,
            blob: false,
        }
    }

    fn hacd_desc(path: &str, count: u32) -> crate::audit::ActionDesc {
        let mut d = desc(7, path, None);
        d.transfer = Some(crate::audit::TransferDesc {
            schema: crate::schema::SCHEMA_TRANSFER_DESC.to_owned(),
            from: None,
            to: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9".to_owned(),
            payload: crate::audit::PayloadDesc::Hacd {
                count,
                names: vec![],
            },
        });
        d
    }

    /// A denied kind wrapped inside an AST control-flow action must be denied
    /// too: the policy walks the review tree, so nesting is never a bypass.
    #[test]
    fn deny_kinds_recurse_into_ast_children() {
        let policy = Policy {
            deny_kinds: Some(vec![44]),
            ..Default::default()
        };
        let mut review = sample_review();
        review.actions = vec![desc(
            25, // ast_select
            "0",
            Some(vec![desc(44, "0/0", None)]),
        )];
        let decision = evaluate_policy(&review, &policy).unwrap();
        assert_eq!(decision.decision, "deny");
        assert!(
            decision.findings.iter().any(|f| f.contains("0/0")),
            "the finding must name the nested path"
        );
    }

    /// `deny_blob` reads the schema-declared blob fact, including nested.
    #[test]
    fn deny_blob_uses_the_schema_blob_fact() {
        let policy = Policy {
            deny_blob: Some(true),
            ..Default::default()
        };
        let mut review = sample_review();
        let mut blob = desc(0x0401, "0/0", None); // tx_message kind
        blob.blob = true;
        review.actions = vec![desc(25, "0", Some(vec![blob]))];
        let decision = evaluate_policy(&review, &policy).unwrap();
        assert_eq!(decision.decision, "deny");
    }

    /// The diamond-name cap counts nested transfers too.
    #[test]
    fn max_diamond_names_counts_nested_transfers() {
        let policy = Policy {
            max_diamond_names: Some(2),
            ..Default::default()
        };
        let mut review = sample_review();
        review.actions = vec![desc(
            25,
            "0",
            Some(vec![hacd_desc("0/0", 3)]),
        )];
        let decision = evaluate_policy(&review, &policy).unwrap();
        assert_eq!(decision.decision, "deny");
    }
}
