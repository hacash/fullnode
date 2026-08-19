//! Structured SDK errors (Unified SDK 2.0, doc 14 §7).
//!
//! Codes are stable strings and additive-only: business logic classifies by
//! `code`, never by message text. `sys::Error` text is never parsed.

/// Declares the error-code surface in one place: the `SdkErrorCode` enum, its
/// snake_case `as_str()` mapping and the positional `ERROR_CODES` ABI table all
/// come from this list, so adding a code can never desync the enum from the
/// binary ids (the same pattern as `profile::define_operations!`).
macro_rules! define_error_codes {
    ($(($variant:ident, $name:literal)),+ $(,)?) => {
        /// Stable error codes, frozen at ABI major 2 (doc 14 §7).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum SdkErrorCode {
            $($variant,)+
        }

        impl SdkErrorCode {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(SdkErrorCode::$variant => $name,)+
                }
            }
        }

        /// Stable error codes in ABI id order (index + 1 = binary id; entry 0
        /// is reserved as "unknown", mirroring `ERROR_NAMES` on the JS side).
        /// Single source for `error_code_id` and the generated `op_tables.mjs`.
        pub const ERROR_CODES: &[&str] = &[$($name),+];
    };
}

define_error_codes! {
    (UnknownOperation, "unknown_operation"),
    (UnsupportedFeature, "unsupported_feature"),
    (UnsupportedSchema, "unsupported_schema"),
    (UnknownField, "unknown_field"),
    (UnknownAction, "unknown_action"),
    (TrailingBytes, "trailing_bytes"),
    (ParseFailed, "parse_failed"),
    (WrongChainId, "wrong_chain_id"),
    (ExpiredHeight, "expired_height"),
    (MissingInspectContext, "missing_inspect_context"),
    (UnsupportedTxType, "unsupported_tx_type"),
    (InvalidAddress, "invalid_address"),
    (InvalidPublicKey, "invalid_public_key"),
    (BadSignature, "bad_signature"),
    (ReviewBindingMismatch, "review_binding_mismatch"),
    (TransactionJsonMismatch, "transaction_json_mismatch"),
    (RequestExpired, "request_expired"),
    (InvalidSigningRequest, "invalid_signing_request"),
    (PolicyBindingMismatch, "policy_binding_mismatch"),
    (CodecProfileMismatch, "codec_profile_mismatch"),
}

/// `{ code, message, detail? }` — the single error shape across every
/// operation. `detail` carries `action_index`, `byte_offset`, `expected`,
/// `actual`, `path` etc. when available.
#[derive(Debug, Clone, PartialEq)]
pub struct SdkError {
    pub schema: String,
    pub code: String,
    pub message: String,
    /// Serialized JSON detail (a string under the binary ABI, no serde).
    pub detail: Option<String>,
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
        detail: impl Into<String>,
    ) -> Self {
        Self {
            schema: crate::schema::SCHEMA_ERROR.to_owned(),
            code: code.as_str().to_owned(),
            message: message.into(),
            detail: Some(detail.into()),
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

// Binary ABI: no serde_json, so the `From<serde_json::Error>` impl was dropped.
