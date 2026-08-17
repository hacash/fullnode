//! Unified SDK 2.0 (doc 14). The raw WASM transport is a single
//! `sdk_invoke`/`sdk_transport_version` pair; all operations are JSON
//! request/response through the dispatcher. Private keys never cross the
//! boundary.

#![cfg_attr(all(target_arch = "wasm32", not(test)), no_main)]

mod codec;
mod names;

pub mod account;
pub mod amount;
pub mod attach;
pub mod audit;
pub mod build;
pub mod error;
pub mod inspect;
pub mod message;
pub mod policy;
pub mod profile;
pub mod schema;
pub mod service;

pub use account::{address_from_public_key, verify_address};
pub use amount::{format_protocol, parse_protocol};
pub use attach::{
    attach_signature, prepare_signature, signature_report, verify_signatures, SignatureProof,
    SigningRequest,
};
pub use audit::{ActionDesc, TransferDesc};
pub use build::{build_transaction, ActionSpec, TransactionSpec};
pub use error::{SdkError, SdkErrorCode};
pub use inspect::{inspect, inspect_report, Review};
pub use policy::{evaluate_policy, Policy, PolicyDecision};
pub use profile::{capabilities, CodecProfile, SDK_VERSION};
pub use schema::ResultEnvelope;

/// Current UNIX time in seconds. Native builds use `sys::curtimes`; on wasm32
/// `SystemTime::now()` is not implemented, so the host clock is reached
/// through JS (`Date.now`) instead. The JS facade also injects an explicit
/// `timestamp` when `tx.build` is called without one, so this is only a
/// fallback for raw `sdk_invoke` callers and the request-expiry checks.
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

/// Raw WASM transport: one JSON request in, one envelope JSON out.
/// `{ operation, payload }` → `{ ok: true, value } | { ok: false, error }`.
/// Every input object rejects unknown fields (`unknown_field`); the
/// `operation` names are the `OPERATIONS` registry (see `profile`).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_invoke(request_json: &str) -> String {
    service::invoke(request_json)
}

/// Raw WASM transport version (doc 14 §9). Bumping this means the JSON
/// transport semantics changed, not that operations were added.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_transport_version() -> u32 {
    service::TRANSPORT_VERSION
}
