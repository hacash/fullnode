//! Frozen schema ids and the unified result envelope (Unified SDK 2.0, doc 14).
//! Schema ids are the public data contract, additive-only — changing an existing id's semantics requires a new schema major.

/// Stable schema ids (frozen at ABI major 2).
pub const SCHEMA_ERROR: &str = "hacash.sdk/error@1";
pub const SCHEMA_SDK_VERSION: &str = "hacash.sdk/sdk-version@1";
pub const SCHEMA_CAPABILITIES: &str = "hacash.sdk/capabilities@1";
pub const SCHEMA_CODEC_PROFILE: &str = "hacash.sdk/codec-profile@2";
/// v4: adds `guard_violations` / `schedule_violations` / signer valid+invalid
/// and `signature_errors`; `protocol_valid` is the conjunction of empty fact buckets (never a gate).
pub const SCHEMA_REVIEW: &str = "hacash.sdk/review@4";
pub const SCHEMA_ACTION_DESC: &str = "hacash.sdk/action-desc@1";
pub const SCHEMA_TRANSFER_DESC: &str = "hacash.sdk/transfer-desc@1";
pub const SCHEMA_TRANSACTION_SPEC: &str = "hacash.sdk/transaction-spec@1";
pub const SCHEMA_TRANSACTION_JSON: &str = "hacash.sdk/transaction-json@1";
pub const SCHEMA_BUILT_TRANSACTION: &str = "hacash.sdk/built-transaction@1";
pub const SCHEMA_SIGNING_REQUEST: &str = "hacash.sdk/signing-request@1";
pub const SCHEMA_SIGNATURE_PROOF: &str = "hacash.sdk/signature-proof@1";
pub const SCHEMA_SIGNATURE_REPORT: &str = "hacash.sdk/signature-report@1";
pub const SCHEMA_ATTACH_RESULT: &str = "hacash.sdk/attach-result@2";
pub const SCHEMA_VERIFY_RESULT: &str = "hacash.sdk/verify-result@1";
pub const SCHEMA_POLICY: &str = "hacash.sdk/policy@1";
pub const SCHEMA_POLICY_DECISION: &str = "hacash.sdk/policy-decision@1";

/// Hash domain prefixes (frozen at ABI major 2). All SDK bindings are
/// sha3-256 over an explicit domain prefix, never over undecorated JSON.
pub const DOMAIN_UNSIGNED_BODY: &[u8] = b"hacash.sdk/unsigned-body@1";
pub const DOMAIN_REVIEW_BINDING: &[u8] = b"hacash.sdk/review-binding@1";
pub const DOMAIN_CODEC_PROFILE: &[u8] = b"hacash.sdk/codec-profile@2";
pub const DOMAIN_SIGNING_REQUEST: &[u8] = b"hacash.sdk/signing-request@1";
pub const DOMAIN_POLICY_DECISION: &[u8] = b"hacash.sdk/policy-decision@1";
