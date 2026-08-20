//! Unified SDK 2.0 (doc 14). The raw WASM transport is
//! `sdk_invoke_binary`/`sdk_transport_version`; the wasm core is JSON-free
//! (binary `bjson` field streams in, binary envelope out — all JSON lives in
//! the JS facade). Private keys never cross the boundary.

#![cfg_attr(all(target_arch = "wasm32", not(test)), no_main)]

mod bjson;
mod codec;
mod json;
mod names;
mod spec_codec;

/// Declarative friendly↔wire action mapping (`ACTION_SPECS`): single source
/// for the generated Rust decoder, the generated JS adapter (`sdk_codegen`)
/// and the golden-vector/validation tests.
pub mod actionspec;

/// Generators for the JS artifacts (`sdk_codegen` bin): `actionspec.mjs/.d.ts`
/// and `op_tables.mjs`, all derived from Rust single sources.
pub mod codegen;

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
    SignatureProof, SigningRequest, attach_signature, prepare_signature, signature_report,
    verify_signatures,
};
pub use audit::{ActionDesc, TransferDesc};
pub use build::{ActionSpec, TransactionSpec, build_transaction};
pub use error::{SdkError, SdkErrorCode};
pub use inspect::{Review, inspect, inspect_report};
pub use policy::{Policy, PolicyDecision, evaluate_policy};
pub use profile::{CodecProfile, SDK_VERSION, capabilities};
pub use schema::ResultEnvelope;
pub use spec_codec::decode_transaction_spec_binary;

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

/// Binary WASM transport (§5): `sdk_invoke_binary(operation_id, payload)`
/// → binary envelope (see `service`). `operation_id` is a `service::OP_*`
/// constant; the payload/result layout is in each operation's `route` branch.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_invoke_binary(operation_id: u16, payload: &[u8]) -> Vec<u8> {
    service::invoke_binary(operation_id, payload)
}

/// Binary transport version (§5): bumped when the envelope/payload semantics change.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn sdk_transport_version() -> u32 {
    service::TRANSPORT_VERSION
}
