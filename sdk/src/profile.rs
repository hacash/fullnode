//! Codec profile, capabilities and SDK version (Unified SDK 2.0, doc 14 §4.1/§4.7/§6.2).
//! `profile_hash` pins the codec identity: any protocol or registry change rotates it, invalidating outstanding review bindings.

use crate::json::SdkJsonTo;
use crate::schema::{DOMAIN_CODEC_PROFILE, SCHEMA_CAPABILITIES, SCHEMA_CODEC_PROFILE};

pub const SDK_VERSION: &str = "0.2.1";
pub const ABI_MAJOR: u32 = 2;
/// v2.5: the codec profile reports and hashes transaction-type membership;
/// the wallet surface admits only current Type-2/3 envelopes.
pub const ABI_MINOR: u32 = 5;
/// Fullnode source commit this SDK's protocol/codec behavior corresponds to.
/// Injected from git by `build.rs`, never hand-maintained.
pub const FULLNODE_COMMIT: &str = env!("SDK_FULLNODE_COMMIT");

/// Protocol-level hard limits. All values are single-sourced from the crates
/// that own them (`hacash-params`, `field`); the SDK re-declares none.
pub use hacash_params::{MAX_TX_SIZE, TX_ACTIONS_MAX};
pub const AST_DEPTH_MAX: usize = hacash_params::MAINNET_PARAMS.protocol.ast_tree_depth_max;
pub const HACD_WIRE_MAX: usize = field::DIAMOND_LIST_MAX;

#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolParamsProfile {
    pub ast_tree_depth_max: usize,
    pub max_type3_signers: usize,
    pub fee_purity_floor: u64,
    pub diamond_form_flag: u64,
    /// Height-gated floor reductions `(activation_height, next_floor)`; with
    /// `fee_purity_floor` they form the chain's gas-billing schedule.
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
    /// Version of the single Hacash parameter profile that produced limits
    /// and execution facts exposed by this SDK.
    pub params_version: u32,
    pub protocol_params: ProtocolParamsProfile,
    pub limits: LimitsProfile,
    pub registered_kinds: Vec<u16>,
    /// Wallet transaction envelope types admitted by this codec profile.
    pub registered_tx_types: Vec<u16>,
    /// Hash of the sorted schema set (`base::schema_set_hash`): field-shape
    /// changes also rotate profile_hash, not just kind-set changes.
    pub schema_hash: String,
    /// Hash of the ordered transaction-type and action-kind sets; reports
    /// registry membership independently of field shape (unlike `schema_hash`).
    pub registry_hash: String,
    pub profile_hash: String,
}

impl CodecProfile {
    /// Build the profile from the SDK codec registry (protocol + VM actions).
    pub fn standard() -> Self {
        let (tx_types, kinds) = crate::codec::standard_codecs()
            .map(|codecs| {
                (
                    codecs
                        .registered_tx_types()
                        .into_iter()
                        .map(u16::from)
                        .collect(),
                    codecs.registered_kinds(),
                )
            })
            .unwrap_or_default();
        Self::build(tx_types, kinds)
    }

    pub fn build(mut registered_tx_types: Vec<u16>, registered_kinds: Vec<u16>) -> Self {
        registered_tx_types.sort_unstable();
        registered_tx_types.dedup();
        let mut kinds = registered_kinds;
        kinds.sort_unstable();
        kinds.dedup();
        let params = hacash_params::MAINNET_PARAMS.protocol;
        let registry_hash = Self::registry_hash_hex(&registered_tx_types, &kinds);
        let mut profile = CodecProfile {
            schema: SCHEMA_CODEC_PROFILE.to_owned(),
            sdk_version: SDK_VERSION.to_owned(),
            fullnode_commit: FULLNODE_COMMIT.to_owned(),
            params_version: hacash_params::MAINNET_PARAMS.version,
            protocol_params: ProtocolParamsProfile {
                ast_tree_depth_max: params.ast_tree_depth_max,
                max_type3_signers: params.max_type3_signers,
                fee_purity_floor: params.vm.initial_fee_purity_floor,
                diamond_form_flag: params.diamond_form_flag,
                fee_purity_reductions: params.vm.fee_purity_reductions.to_vec(),
            },
            limits: LimitsProfile {
                max_tx_size: hacash_params::MAINNET_PARAMS.mint.max_tx_size,
                tx_actions_max: params.tx_actions_max,
                hacd_wire_max: HACD_WIRE_MAX,
            },
            registered_kinds: kinds,
            registered_tx_types,
            schema_hash: Self::schema_set_hash_hex(),
            registry_hash,
            profile_hash: String::new(),
        };
        let hash = profile.compute_hash();
        profile.profile_hash = hash;
        profile
    }

    /// Deterministic hash of the captured schema set (actions + structs), hex-encoded.
    fn schema_set_hash_hex() -> String {
        // The chain struct set is the SDK selection over crate-owned catalogs
        // (same list the spec codec uses).
        let schemas = crate::selection::action_schemas();
        let structs = crate::selection::struct_schemas();
        let hash = base::schema_set_hash(&schemas, &structs);
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn registry_hash_hex(tx_types: &[u16], kinds: &[u16]) -> String {
        let mut bytes = Vec::with_capacity(4 + (tx_types.len() + kinds.len()) * 2);
        bytes.extend_from_slice(&(tx_types.len() as u16).to_be_bytes());
        for ty in tx_types {
            bytes.extend_from_slice(&ty.to_be_bytes());
        }
        bytes.extend_from_slice(&(kinds.len() as u16).to_be_bytes());
        for kind in kinds {
            bytes.extend_from_slice(&kind.to_be_bytes());
        }
        hex::encode(sys::calculate_hash(bytes))
    }

    /// sha3-256 over the domain prefix and the canonical profile JSON
    /// (excluding `profile_hash` itself, which is empty at this point).
    fn compute_hash(&self) -> String {
        let mut copy = self.clone();
        copy.profile_hash.clear();
        let body = copy.to_json_string();
        let mut data = Vec::with_capacity(DOMAIN_CODEC_PROFILE.len() + body.len());
        data.extend_from_slice(DOMAIN_CODEC_PROFILE);
        data.extend_from_slice(body.as_bytes());
        hex::encode(sys::calculate_hash(data))
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

/// One field of a JSON request layout. A dotted name (`options.review`) is a
/// flattened key in the raw request object.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RequestField {
    String(&'static str),
    OptionalString(&'static str),
    /// Nested JSON object decoded by the typed boundary parser.
    Json(&'static str),
    /// TransactionSpec JSON object; action fields are resolved dynamically
    /// from the SDK action schema profile.
    TransactionSpec(&'static str),
    OptionalJson(&'static str),
    OptionalU64(&'static str),
    U8(&'static str),
    /// Inspect context `{current_height, expected_chain_id}`: marker 0/1,
    /// then u64 + u32 when present.
    OptionalInspectContext(&'static str),
}

impl RequestField {
    /// The field's arg name (the key the Rust parse result is addressed by).
    pub(crate) fn arg_name(&self) -> &'static str {
        match self {
            RequestField::String(a)
            | RequestField::OptionalString(a)
            | RequestField::Json(a)
            | RequestField::TransactionSpec(a)
            | RequestField::OptionalJson(a)
            | RequestField::OptionalU64(a)
            | RequestField::U8(a)
            | RequestField::OptionalInspectContext(a) => *a,
        }
    }
}

/// One SDK-local raw operation. This static table is the sole operation
/// registry: it assigns numeric ids, validates JSON requests, and reports
/// capabilities. It is neither exported nor mirrored in JavaScript.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Operation {
    pub(crate) name: &'static str,
    pub(crate) request: &'static [RequestField],
}

macro_rules! op_req {
    (string($a:literal)) => {
        RequestField::String($a)
    };
    (optional_string($a:literal)) => {
        RequestField::OptionalString($a)
    };
    (json($a:literal)) => {
        RequestField::Json($a)
    };
    (tx_spec($a:literal)) => {
        RequestField::TransactionSpec($a)
    };
    (optional_json($a:literal)) => {
        RequestField::OptionalJson($a)
    };
    (opt_u64($a:literal)) => {
        RequestField::OptionalU64($a)
    };
    (u8($a:literal)) => {
        RequestField::U8($a)
    };
    (opt_ctx($a:literal)) => {
        RequestField::OptionalInspectContext($a)
    };
}

/// Every operation the dispatcher routes. This SDK-local list is the source
/// for capabilities, raw operation ids, and request validation.
macro_rules! define_operations {
    ($(($id:ident, $name:literal, [$($req:ident($($arg:literal),*)),* $(,)?])),+ $(,)?) => {
        pub(crate) const OPERATIONS: &[Operation] = &[
            $(Operation {
                name: $name,
                request: &[$(op_req!($req($($arg),*))),*],
            }),+
        ];
        define_operations!(@ops 1; $($id),+);
    };
    (@ops $n:expr;) => {};
    (@ops $n:expr; $head:ident $(, $tail:ident)*) => {
        pub(crate) const $head: u16 = $n;
        define_operations!(@ops $n + 1; $($tail),*);
    };
}

define_operations! {
    (OP_SYSTEM_CAPABILITIES, "system.capabilities", []),
    (OP_SYSTEM_SDK_VERSION, "system.sdk_version", []),
    (OP_SYSTEM_CODEC_PROFILE, "system.codec_profile", []),
    (OP_TX_BUILD, "tx.build", [tx_spec("spec")]),
    (OP_TX_INSPECT_REPORT, "tx.inspect_report", [string("body"), optional_string("signer_address")]),
    (OP_TX_INSPECT, "tx.inspect", [string("body"), optional_string("signer_address"), opt_ctx("context")]),
    (OP_TX_PREPARE_SIGNATURE, "tx.prepare_signature", [string("body"), string("signer_address"), optional_json("options.review"), optional_json("options.policy"), optional_string("options.origin"), opt_u64("options.expires_at")]),
    (OP_TX_ATTACH_SIGNATURE, "tx.attach_signature", [string("body"), json("proof"), json("review"), json("request")]),
    (OP_TX_ATTACH_SIGNATURE_UNBOUND, "tx.attach_signature_unbound", [string("body"), json("proof")]),
    (OP_TX_VERIFY, "tx.verify", [string("body")]),
    (OP_TX_SIGNATURE_REPORT, "tx.signature_report", [string("body")]),
    (OP_TX_DECODE, "tx.decode", [string("body")]),
    (OP_TX_ENCODE, "tx.encode", [json("transaction"), optional_json("review")]),
    (OP_ACCOUNT_VERIFY_ADDRESS, "account.verify_address", [string("address")]),
    (OP_ACCOUNT_ADDRESS_FROM_PUBLIC_KEY, "account.address_from_public_key", [string("public_key")]),
    (OP_AMOUNT_PARSE_PROTOCOL, "amount.parse_protocol", [string("value")]),
    (OP_AMOUNT_FORMAT_PROTOCOL, "amount.format_protocol", [string("value"), u8("unit")]),
    (OP_MESSAGE_PREPARE_SIGNATURE, "message.prepare_signature", [json("params")]),
    (OP_MESSAGE_VERIFY, "message.verify", [json("request"), json("proof")]),
    (OP_POLICY_EVALUATE, "policy.evaluate", [json("review"), optional_json("policy")]),
}

/// Frozen feature baseline of Unified SDK 2.0 — the routed operation set, each
/// at version 1. New ops extend `OPERATIONS`; semantic changes need a schema major.
pub fn capabilities(profile: &CodecProfile) -> Capabilities {
    let features = OPERATIONS
        .iter()
        .map(|operation| FeatureItem {
            id: operation.name.to_owned(),
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
