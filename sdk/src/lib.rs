//! Unified SDK 2.0 (doc 14). Raw WASM transport `sdk_invoke_json`/`sdk_transport_version`; the
//! boundary is JSON strings via the hand-written engine (serde_json test-oracle only). Private keys never cross the boundary.

#![cfg_attr(all(target_arch = "wasm32", not(test)), no_main)]

mod codec;
mod json;
mod jsonparse;
mod selection;
mod spec_codec;

pub mod account;
pub mod amount;
pub mod attach;
pub mod audit;
pub mod build;
pub mod diamond;
pub mod error;
pub mod fee;
pub mod inspect;
pub mod message;
pub mod policy;
pub mod profile;
pub mod schema;
pub mod service;
pub mod vm;

pub use account::{address_from_public_key, verify_address, verify_signature};
pub use amount::{format, parse};
pub use attach::{
    attach_signature, prepare_signature, signature_report, verify_signatures, SignatureProof,
    SigningRequest,
};
pub use audit::{ActionCodeDesc, ActionDesc, DescribeOptions, TransferDesc, describe_single};
pub use build::{build_transaction, ActionSpec, TransactionSpec};
pub use diamond::lookup;
pub use error::{SdkError, SdkErrorCode};
pub use fee::estimate_fee;
pub use inspect::{inspect, inspect_report, Review};
pub use policy::{evaluate_policy, Policy, PolicyDecision};
pub use profile::{CodecProfile, SDK_VERSION};
pub use spec_codec::{decode_transaction_spec_json, WireValue};
pub use vm::{code, decode_call};

/// Current UNIX time in seconds (`sys::curtimes` natively; `Date.now` on wasm32, where
/// `SystemTime::now()` is unavailable). Fallback for raw `sdk_invoke` callers and expiry checks.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Date)]
    fn now() -> f64;
}

pub(crate) fn now_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        sys::curtimes()
    }
}

/// JSON WASM transport (§5): `sdk_invoke_json(operation_id, payload)` with a
/// UTF-8 JSON request → JSON envelope string; `operation_id` is an `OP_*` constant.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_invoke_json(operation_id: u16, payload: &[u8]) -> String {
    service::invoke_json(operation_id, payload)
}

/// Transport version (§5): bumped when the envelope/payload semantics change.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_transport_version() -> u32 {
    service::TRANSPORT_VERSION
}
