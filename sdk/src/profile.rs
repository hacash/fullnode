//! Codec profile, capabilities and SDK version (Unified SDK 2.0, doc 14
//! §4.1/§4.7/§6.2). `profile_hash` pins the codec identity: any protocol or
//! registry change rotates it, invalidating outstanding review bindings.


use crate::error::{SdkError, SdkErrorCode};
use crate::schema::{DOMAIN_CODEC_PROFILE, SCHEMA_CAPABILITIES, SCHEMA_CODEC_PROFILE};

pub const SDK_VERSION: &str = "0.2.1";
pub const ABI_MAJOR: u32 = 2;
/// v2.1: `tx.inspect` no longer denies reviews for height/chain guards; the
/// strict-mode context is evaluated into facts (`expired_height`/
/// `wrong_chain`, review schema v2) and the upper layer decides.
pub const ABI_MINOR: u32 = 1;
/// Fullnode source commit this SDK's protocol/codec behavior corresponds to.
/// Injected from git by `build.rs` (never hand-maintained), so the profile
/// identity can never lag behind a protocol/registry-affecting change.
pub const FULLNODE_COMMIT: &str = env!("SDK_FULLNODE_COMMIT");

/// Protocol-level hard limits. All values are single-sourced from the crates
/// that own them (`base` for the wire/consensus caps, `protocol::PROTOCOL_PARAMS`
/// for the AST depth, `field` for the diamond-list cap) — the SDK re-declares
/// none of these numbers.
pub use base::MAX_TX_SIZE;
pub use base::TX_ACTIONS_MAX;
pub const AST_DEPTH_MAX: usize = protocol::PROTOCOL_PARAMS.ast_tree_depth_max;
pub const HACD_WIRE_MAX: usize = field::DIAMOND_LIST_MAX;

#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolParamsProfile {
    pub ast_tree_depth_max: usize,
    pub max_type3_signers: usize,
    pub fee_purity_floor: u64,
    pub diamond_form_flag: u64,
    /// Height-gated floor reductions `(activation_height, next_floor)`;
    /// together with `fee_purity_floor` they form the same schedule the
    /// chain bills gas by (single `base` computation).
    pub fee_purity_reductions: Vec<(u64, u64)>,
}

impl ProtocolParamsProfile {
    /// Effective fee purity floor at `height` — delegated to the single
    /// `base` schedule implementation, never re-derived here.
    pub fn fee_purity_floor_at(&self, height: u64) -> u64 {
        base::fee_purity_floor_at(self.fee_purity_floor, &self.fee_purity_reductions, height)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitsProfile {
    pub max_tx_size: usize,
    pub tx_actions_max: usize,
    pub hacd_wire_max: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodecProfile {
    pub schema: String,
    pub sdk_version: String,
    pub fullnode_commit: String,
    pub protocol_params: ProtocolParamsProfile,
    pub limits: LimitsProfile,
    pub registered_kinds: Vec<u16>,
    /// Hash of the sorted schema set (`base::schema_set_hash`): field-shape
    /// changes also rotate profile_hash, not just kind-set changes.
    pub schema_hash: String,
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
                fee_purity_reductions: params.vm.fee_purity_reductions.to_vec(),
            },
            limits: LimitsProfile {
                max_tx_size: MAX_TX_SIZE,
                tx_actions_max: TX_ACTIONS_MAX,
                hacd_wire_max: HACD_WIRE_MAX,
            },
            registered_kinds: kinds,
            schema_hash: Self::schema_set_hash_hex(),
            profile_hash: String::new(),
        };
        let hash = profile.compute_hash();
        profile.profile_hash = hash;
        profile
    }

    /// Deterministic hash of the captured schema set (actions + structs), hex-encoded.
    fn schema_set_hash_hex() -> String {
        let schemas: Vec<base::ActionSchema> = crate::codec::standard_codecs()
            .map(|codecs| codecs.action_schemas().to_vec())
            .unwrap_or_default();
        if schemas.is_empty() {
            return String::new();
        }
        // The chain struct set comes from the single chain-codec aggregation
        // (same list `codec-schema-gen` and the spec codec use).
        let structs = chain_codec::struct_schemas();
        let hash = base::schema_set_hash(&schemas, &structs);
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// sha3-256 over the domain prefix and the canonical profile JSON
    /// (excluding `profile_hash` itself, which is empty at this point).
    fn compute_hash(&self) -> String {
        let mut copy = self.clone();
        copy.profile_hash.clear();
        let body = copy.to_binary_body();
        let mut data = Vec::with_capacity(DOMAIN_CODEC_PROFILE.len() + body.len());
        data.extend_from_slice(DOMAIN_CODEC_PROFILE);
        data.extend_from_slice(&body);
        hex::encode(sys::calculate_hash(data))
    }

    pub fn profile_hash(&self) -> &str {
        &self.profile_hash
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AbiVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureItem {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    pub schema: String,
    pub package_version: String,
    pub abi: AbiVersion,
    pub codec_profile_hash: String,
    pub features: Vec<FeatureItem>,
}

/// One field of a binary request layout. The JS facade packs these before
/// `invoke`; they mirror the reads in `service::parse_request` (both sides are
/// driven by this same table, so a layout edit can never desync them). The arg
/// name is the JS parameter name; a dotted name (`options.review`) reads from
/// the trailing options object (`options?.review ?? null`).
#[derive(Debug, Clone, Copy)]
pub enum OpRequestField {
    W2Str(&'static str),
    OptW2Str(&'static str),
    /// Complex object as a `bjson` field stream (W4 len + body).
    W4Bin(&'static str),
    OptW4Bin(&'static str),
    OptU64(&'static str),
    U8(&'static str),
    /// Inspect context `{current_height, expected_chain_id}`: marker 0/1,
    /// then u64 + u32 when present.
    OptInspectContext(&'static str),
}

impl OpRequestField {
    /// The field's arg name (the key the Rust parse result is addressed by).
    pub fn arg_name(&self) -> &'static str {
        match self {
            OpRequestField::W2Str(a)
            | OpRequestField::OptW2Str(a)
            | OpRequestField::W4Bin(a)
            | OpRequestField::OptW4Bin(a)
            | OpRequestField::OptU64(a)
            | OpRequestField::U8(a)
            | OpRequestField::OptInspectContext(a) => *a,
        }
    }
}

/// One routed operation's JS facade surface, emitted from the same list as
/// `OPERATIONS`/`OP_*` (single source). `special` operations (tx.build only)
/// keep hand-written bodies in `hacashsdk.mjs`; every other operation's
/// request layout is packed by the generated facade and parsed by
/// `service::parse_request` from this same table.
#[derive(Debug, Clone, Copy)]
pub struct OpDef {
    pub group: &'static str,
    pub method: &'static str,
    pub special: bool,
    pub request: &'static [OpRequestField],
}

macro_rules! op_req {
    (w2_str($a:literal)) => { OpRequestField::W2Str($a) };
    (opt_w2_str($a:literal)) => { OpRequestField::OptW2Str($a) };
    (w4_bin($a:literal)) => { OpRequestField::W4Bin($a) };
    (opt_w4_bin($a:literal)) => { OpRequestField::OptW4Bin($a) };
    (opt_u64($a:literal)) => { OpRequestField::OptU64($a) };
    (u8($a:literal)) => { OpRequestField::U8($a) };
    (opt_ctx($a:literal)) => { OpRequestField::OptInspectContext($a) };
}

/// Every operation the dispatcher routes. Single registry: `capabilities()`
/// derives the feature list from it, a dispatcher test asserts every entry
/// routes, and the JS facade methods are generated from the request layouts —
/// so the capability surface, the routing surface and the JS operation surface
/// all share one source of truth. `define_operations!` also emits the `OP_*`
/// id constants from the same list (positional, index + 1).
macro_rules! define_operations {
    ($(($id:ident, $name:literal, $group:literal, $method:ident, $special:literal, [$($req:ident($($arg:literal),*)),* $(,)?])),+ $(,)?) => {
        pub const OPERATIONS: &[&str] = &[$($name),+];
        define_operations!(@ops 1; $($id),+);
        /// JS facade surface per routed operation (see `OpDef`); single source
        /// for the generated `operations.mjs`, same list as `OPERATIONS`.
        pub const OP_DEFS: &[OpDef] = &[
            $(OpDef {
                group: $group,
                method: stringify!($method),
                special: $special,
                request: &[$(op_req!($req($($arg),*))),*],
            }),+
        ];
    };
    (@ops $n:expr;) => {};
    (@ops $n:expr; $head:ident $(, $tail:ident)*) => {
        pub(crate) const $head: u16 = $n;
        define_operations!(@ops $n + 1; $($tail),*);
    };
}

define_operations! {
    (OP_SYSTEM_CAPABILITIES, "system.capabilities", "system", capabilities, false, []),
    (OP_SYSTEM_SDK_VERSION, "system.sdk_version", "system", sdk_version, false, []),
    (OP_SYSTEM_CODEC_PROFILE, "system.codec_profile", "system", codec_profile, false, []),
    (OP_TX_BUILD, "tx.build", "tx", build, true, []),
    (OP_TX_INSPECT_REPORT, "tx.inspect_report", "tx", inspect_report, false, [w2_str("body"), opt_w2_str("signer_address")]),
    (OP_TX_INSPECT, "tx.inspect", "tx", inspect, false, [w2_str("body"), opt_w2_str("signer_address"), opt_ctx("context")]),
    (OP_TX_PREPARE_SIGNATURE, "tx.prepare_signature", "tx", prepare_signature, false, [w2_str("body"), w2_str("signer_address"), opt_w4_bin("options.review"), opt_w4_bin("options.policy"), opt_w2_str("options.origin"), opt_u64("options.expires_at")]),
    (OP_TX_ATTACH_SIGNATURE, "tx.attach_signature", "tx", attach_signature, false, [w2_str("body"), w4_bin("proof"), w4_bin("review"), w4_bin("request")]),
    (OP_TX_ATTACH_SIGNATURE_UNBOUND, "tx.attach_signature_unbound", "tx", attach_signature_unbound, false, [w2_str("body"), w4_bin("proof")]),
    (OP_TX_VERIFY, "tx.verify", "tx", verify, false, [w2_str("body")]),
    (OP_TX_SIGNATURE_REPORT, "tx.signature_report", "tx", signature_report, false, [w2_str("body")]),
    (OP_TX_DECODE, "tx.decode", "tx", decode, false, [w2_str("body")]),
    (OP_TX_ENCODE, "tx.encode", "tx", encode, false, [w4_bin("transaction"), opt_w4_bin("review")]),
    (OP_ACCOUNT_VERIFY_ADDRESS, "account.verify_address", "account", verify_address, false, [w2_str("address")]),
    (OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY, "account.address_from_public_key", "account", address_from_public_key, false, [w2_str("public_key")]),
    (OP_AMOUNT_PARSE_PROTOCOL, "amount.parse_protocol", "amount", parse_protocol, false, [w2_str("value")]),
    (OP_AMOUNT_FORMAT_PROTOCOL, "amount.format_protocol", "amount", format_protocol, false, [w2_str("value"), u8("unit")]),
    (OP_MESSAGE_PREPARE_SIGNATURE, "message.prepare_signature", "message", prepare_signature, false, [w4_bin("params")]),
    (OP_MESSAGE_VERIFY, "message.verify", "message", verify, false, [w4_bin("request"), w4_bin("proof")]),
    (OP_POLICY_EVALUATE, "policy.evaluate", "policy", evaluate, false, [w4_bin("review"), opt_w4_bin("policy")]),
}

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
            crate::json::obj(vec![crate::json::kv("expected", crate::json::q(&expected)), crate::json::kv("actual", crate::json::q(&actual))]),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated `OP_*` ids are positional over `OPERATIONS` (index + 1);
    /// both come from the single `define_operations!` invocation, but this
    /// test pins the contract in case the macro is ever edited carelessly.
    #[test]
    fn operation_ids_are_positional_over_operations() {
        let ids = [
            OP_SYSTEM_CAPABILITIES,
            OP_SYSTEM_SDK_VERSION,
            OP_SYSTEM_CODEC_PROFILE,
            OP_TX_BUILD,
            OP_TX_INSPECT_REPORT,
            OP_TX_INSPECT,
            OP_TX_PREPARE_SIGNATURE,
            OP_TX_ATTACH_SIGNATURE,
            OP_TX_ATTACH_SIGNATURE_UNBOUND,
            OP_TX_VERIFY,
            OP_TX_SIGNATURE_REPORT,
            OP_TX_DECODE,
            OP_TX_ENCODE,
            OP_ACCOUNT_VERIFY_ADDRESS,
            OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY,
            OP_AMOUNT_PARSE_PROTOCOL,
            OP_AMOUNT_FORMAT_PROTOCOL,
            OP_MESSAGE_PREPARE_SIGNATURE,
            OP_MESSAGE_VERIFY,
            OP_POLICY_EVALUATE,
        ];
        assert_eq!(
            ids.len(),
            OPERATIONS.len(),
            "every operation needs an OP_* id"
        );
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                *id,
                i as u16 + 1,
                "OP_* id must be the operation index + 1"
            );
        }
    }

    /// `OP_DEFS` must reconstruct the operation names (`group.method`) in the
    /// same order as `OPERATIONS`; the generated `operations.mjs` derives the
    /// `OP.*` const from that reconstruction.
    #[test]
    fn op_defs_reconstruct_operation_names_in_order() {
        assert_eq!(OP_DEFS.len(), OPERATIONS.len());
        for (def, name) in OP_DEFS.iter().zip(OPERATIONS) {
            assert_eq!(
                format!("{}.{}", def.group, def.method),
                *name,
                "OP_DEFS entry must reconstruct the operation name"
            );
        }
    }
}
