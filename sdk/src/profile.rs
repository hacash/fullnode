//! Codec profile, capabilities and SDK version (Unified SDK 2.0, doc 14
//! §4.1/§4.7/§6.2). `profile_hash` pins the codec identity: any protocol or
//! registry change rotates it, invalidating outstanding review bindings.

use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};
use crate::schema::{DOMAIN_CODEC_PROFILE, SCHEMA_CAPABILITIES, SCHEMA_CODEC_PROFILE};

pub const SDK_VERSION: &str = "0.2.0";
pub const ABI_MAJOR: u32 = 2;
pub const ABI_MINOR: u32 = 0;
/// Fullnode source commit this SDK's protocol/codec behavior corresponds to.
/// It pins the *protocol* identity, not the SDK release (the release is pinned
/// by `SDK_VERSION`, which also rotates `profile_hash`): bump it only when a
/// protocol/registry-affecting change lands, then release the SDK.
pub const FULLNODE_COMMIT: &str = "644a6b51d8a11fac804d13ae5423c7277a6ec5d2";

/// Protocol-level hard limits (doc 14 §4.7, plan 13 §4.5).
pub const MAX_TX_SIZE: usize = 16 * 1024;
pub const TX_ACTIONS_MAX: usize = 200;
pub const HACD_WIRE_MAX: usize = 200;
pub const AST_DEPTH_MAX: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolParamsProfile {
    pub ast_tree_depth_max: usize,
    pub max_type3_signers: usize,
    pub fee_purity_floor: u64,
    pub diamond_form_flag: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsProfile {
    pub max_tx_size: usize,
    pub tx_actions_max: usize,
    pub hacd_wire_max: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecProfile {
    pub schema: String,
    pub sdk_version: String,
    pub fullnode_commit: String,
    pub protocol_params: ProtocolParamsProfile,
    pub limits: LimitsProfile,
    pub registered_kinds: Vec<u16>,
    pub profile_hash: String,
}

impl CodecProfile {
    /// Build the profile from the SDK codec registry (protocol + VM actions).
    pub fn standard() -> Self {
        let kinds = crate::codec::standard_codecs()
            .map(|codecs| codecs.registered_kinds())
            .unwrap_or_default();
        Self::build(kinds)
    }

    pub fn build(registered_kinds: Vec<u16>) -> Self {
        let mut kinds = registered_kinds;
        kinds.sort_unstable();
        kinds.dedup();
        let params = protocol::PROTOCOL_PARAMS;
        let mut profile = CodecProfile {
            schema: SCHEMA_CODEC_PROFILE.to_owned(),
            sdk_version: SDK_VERSION.to_owned(),
            fullnode_commit: FULLNODE_COMMIT.to_owned(),
            protocol_params: ProtocolParamsProfile {
                ast_tree_depth_max: params.ast_tree_depth_max,
                max_type3_signers: params.max_type3_signers,
                fee_purity_floor: params.vm.initial_fee_purity_floor,
                diamond_form_flag: params.diamond_form_flag,
            },
            limits: LimitsProfile {
                max_tx_size: MAX_TX_SIZE,
                tx_actions_max: TX_ACTIONS_MAX,
                hacd_wire_max: HACD_WIRE_MAX,
            },
            registered_kinds: kinds,
            profile_hash: String::new(),
        };
        let hash = profile.compute_hash();
        profile.profile_hash = hash;
        profile
    }

    /// sha3-256 over the domain prefix and the canonical profile JSON
    /// (excluding `profile_hash` itself, which is empty at this point).
    fn compute_hash(&self) -> String {
        let mut copy = self.clone();
        copy.profile_hash.clear();
        let json = serde_json::to_string(&copy).expect("codec profile is serializable");
        let mut data = Vec::with_capacity(DOMAIN_CODEC_PROFILE.len() + json.len());
        data.extend_from_slice(DOMAIN_CODEC_PROFILE);
        data.extend_from_slice(json.as_bytes());
        hex::encode(sys::calculate_hash(data))
    }

    pub fn profile_hash(&self) -> &str {
        &self.profile_hash
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureItem {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub schema: String,
    pub package_version: String,
    pub abi: AbiVersion,
    pub codec_profile_hash: String,
    pub features: Vec<FeatureItem>,
}

/// Every operation the dispatcher routes. Single registry: `capabilities()`
/// derives the feature list from it, and a dispatcher test asserts every
/// entry routes, so the capability surface and the routing surface cannot
/// drift.
pub const OPERATIONS: &[&str] = &[
    "system.capabilities",
    "system.sdk_version",
    "system.codec_profile",
    "tx.build",
    "tx.inspect_report",
    "tx.inspect",
    "tx.prepare_signature",
    "tx.attach_signature",
    "tx.attach_signature_unbound",
    "tx.verify",
    "tx.signature_report",
    "tx.decode",
    "tx.encode",
    "account.verify_address",
    "account.address_from_public_key",
    "amount.parse_protocol",
    "amount.format_protocol",
    "message.prepare_signature",
    "message.verify",
    "policy.evaluate",
];

/// Frozen feature baseline of Unified SDK 2.0 — exactly the routed operation
/// set, each at version 1. New operations extend `OPERATIONS`; changing an
/// existing input/output semantic requires a schema major instead of reusing
/// a feature name (doc 14 §4.1).
pub fn capabilities(profile: &CodecProfile) -> Capabilities {
    let features = OPERATIONS
        .iter()
        .map(|id| FeatureItem {
            id: (*id).to_owned(),
            version: 1,
        })
        .collect();
    Capabilities {
        schema: SCHEMA_CAPABILITIES.to_owned(),
        package_version: SDK_VERSION.to_owned(),
        abi: AbiVersion {
            major: ABI_MAJOR,
            minor: ABI_MINOR,
        },
        codec_profile_hash: profile.profile_hash.clone(),
        features,
    }
}

/// Validate that a caller-declared profile hash matches the current one.
pub fn check_profile_hash(expected: &str, actual: &str) -> Result<(), SdkError> {
    if expected != actual {
        return Err(SdkError::with_detail(
            SdkErrorCode::CodecProfileMismatch,
            "codec profile hash mismatch",
            serde_json::json!({ "expected": expected, "actual": actual }),
        ));
    }
    Ok(())
}
