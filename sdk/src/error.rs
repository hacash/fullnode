//! Structured SDK errors (Unified SDK 2.0, doc 14 §7).
//!
//! Codes are stable strings and additive-only: business logic classifies by
//! `code`, never by message text. `sys::Error` text is never parsed.

use serde::{Deserialize, Serialize};

/// Stable error codes, frozen at ABI major 2 (doc 14 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkErrorCode {
    UnknownOperation,
    UnsupportedFeature,
    UnsupportedSchema,
    UnknownField,
    UnknownAction,
    TrailingBytes,
    ParseFailed,
    LimitExceeded,
    WrongChainId,
    ExpiredHeight,
    MissingInspectContext,
    UnsupportedTxType,
    InvalidAddress,
    InvalidPublicKey,
    BadSignature,
    NotRequiredSigner,
    DuplicateSigner,
    ReviewBindingMismatch,
    TransactionJsonMismatch,
    RequestExpired,
    InvalidSigningRequest,
    PolicyBindingMismatch,
    PolicyDenied,
    CodecProfileMismatch,
    IncompleteSignatures,
}

impl SdkErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SdkErrorCode::UnknownOperation => "unknown_operation",
            SdkErrorCode::UnsupportedFeature => "unsupported_feature",
            SdkErrorCode::UnsupportedSchema => "unsupported_schema",
            SdkErrorCode::UnknownField => "unknown_field",
            SdkErrorCode::UnknownAction => "unknown_action",
            SdkErrorCode::TrailingBytes => "trailing_bytes",
            SdkErrorCode::ParseFailed => "parse_failed",
            SdkErrorCode::LimitExceeded => "limit_exceeded",
            SdkErrorCode::WrongChainId => "wrong_chain_id",
            SdkErrorCode::ExpiredHeight => "expired_height",
            SdkErrorCode::MissingInspectContext => "missing_inspect_context",
            SdkErrorCode::UnsupportedTxType => "unsupported_tx_type",
            SdkErrorCode::InvalidAddress => "invalid_address",
            SdkErrorCode::InvalidPublicKey => "invalid_public_key",
            SdkErrorCode::BadSignature => "bad_signature",
            SdkErrorCode::NotRequiredSigner => "not_required_signer",
            SdkErrorCode::DuplicateSigner => "duplicate_signer",
            SdkErrorCode::ReviewBindingMismatch => "review_binding_mismatch",
            SdkErrorCode::TransactionJsonMismatch => "transaction_json_mismatch",
            SdkErrorCode::RequestExpired => "request_expired",
            SdkErrorCode::InvalidSigningRequest => "invalid_signing_request",
            SdkErrorCode::PolicyBindingMismatch => "policy_binding_mismatch",
            SdkErrorCode::PolicyDenied => "policy_denied",
            SdkErrorCode::CodecProfileMismatch => "codec_profile_mismatch",
            SdkErrorCode::IncompleteSignatures => "incomplete_signatures",
        }
    }
}

/// `{ code, message, detail? }` — the single error shape across every
/// operation. `detail` carries `action_index`, `byte_offset`, `expected`,
/// `actual`, `path` etc. when available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdkError {
    pub schema: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl SdkError {
    pub fn new(code: SdkErrorCode, message: impl Into<String>) -> Self {
        Self {
            schema: crate::schema::SCHEMA_ERROR.to_owned(),
            code: code.as_str().to_owned(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(
        code: SdkErrorCode,
        message: impl Into<String>,
        detail: serde_json::Value,
    ) -> Self {
        Self {
            schema: crate::schema::SCHEMA_ERROR.to_owned(),
            code: code.as_str().to_owned(),
            message: message.into(),
            detail: Some(detail),
        }
    }
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SdkError {}

impl From<sys::Error> for SdkError {
    fn from(error: sys::Error) -> Self {
        // The codec registry reports unknown kinds/length mismatches as plain
        // sys::Error text; the SDK decode path detects those conditions itself
        // (see inspect::decode_tx) and only falls back here for unexpected
        // internal failures.
        SdkError::new(SdkErrorCode::ParseFailed, error.to_string())
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(error: serde_json::Error) -> Self {
        // The registry mirror pins serde_json < 1.0.167, which predates
        // `Category::UnknownField`, so unknown-field/variant reports are
        // detected from serde's stable message shape. Should the shape ever
        // change, this degrades to `parse_failed` (the SDK classifies by
        // code, never by message text).
        let message = error.to_string();
        if message.contains("unknown field") {
            return SdkError::with_detail(
                SdkErrorCode::UnknownField,
                "request contains an unknown field (typo or newer schema?)",
                serde_json::json!({ "message": message }),
            );
        }
        if message.contains("unknown variant") {
            return SdkError::with_detail(
                SdkErrorCode::UnknownAction,
                "request contains an unknown action kind or variant",
                serde_json::json!({ "message": message }),
            );
        }
        SdkError::new(SdkErrorCode::ParseFailed, format!("request json invalid: {error}"))
    }
}
