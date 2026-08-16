//! Generic application policy (Unified SDK 2.0, doc 14 §4.8). `evaluate`
//! consumes a Review and a caller-provided Policy and produces a
//! PolicyDecision; it never changes `protocol_valid`/`signability`.

use serde::{Deserialize, Serialize};

use crate::error::SdkError;
use crate::inspect::Review;
use crate::schema::{DOMAIN_POLICY_DECISION, SCHEMA_POLICY, SCHEMA_POLICY_DECISION};

/// Caller-provided policy, frozen schema `hacash.sdk/policy@1` (doc 14 §4.8).
/// Absent fields take protocol defaults; nothing here is a product constant.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Policy {
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub deny_kinds: Option<Vec<u16>>,
    #[serde(default)]
    pub deny_blob: Option<bool>,
    #[serde(default)]
    pub max_diamond_names: Option<u32>,
    #[serde(default)]
    pub confirm_auditability: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let json = serde_json::to_string(policy).unwrap_or_default();
    hex::encode(sys::calculate_hash(json))
}

fn count_diamond_names(review: &Review) -> u32 {
    let mut total = 0u32;
    for action in &review.actions {
        if let Some(transfer) = &action.transfer {
            if let crate::audit::PayloadDesc::Hacd { count, .. } = &transfer.payload {
                total = total.saturating_add(*count);
            }
        }
    }
    total
}

/// `policy.evaluate`: apply the policy to a review. Default (empty) policy
/// allows; AST/TEX/opaque maincall never auto-deny (doc 14 §4.8).
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
        for action in &review.actions {
            if deny_kinds.contains(&action.kind) {
                decision = "deny";
                findings.push(format!("action kind {} denied by policy", action.kind));
            }
        }
    }
    if policy.deny_blob.unwrap_or(false) {
        use protocol::action_std::{TxBlob, TxMessage};
        for action in &review.actions {
            if action.kind == TxBlob::KIND || action.kind == TxMessage::KIND {
                decision = "deny";
                findings.push(format!("blob action {} denied by policy", action.kind));
            }
        }
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
    let json = serde_json::to_string(&decision_obj).unwrap_or_default();
    let mut data = Vec::with_capacity(DOMAIN_POLICY_DECISION.len() + json.len());
    data.extend_from_slice(DOMAIN_POLICY_DECISION);
    data.extend_from_slice(json.as_bytes());
    decision_obj.policy_binding = hex::encode(sys::calculate_hash(data));
    Ok(decision_obj)
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
            protocol_valid: true,
            signability: "signable".to_owned(),
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
        }];
        let decision = evaluate_policy(&review, &policy).unwrap();
        assert_eq!(decision.decision, "deny");
    }
}
