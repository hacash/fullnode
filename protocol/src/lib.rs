//! `protocol` —— standard Hacash codecs, state helpers, and Registry setup.
//!
//! Depends only on sys → field → base (WASM goal #2).
//! Concrete chains register via `register_standard` / `standard_registry`.
//!
//! # Module map
//!
//! - `codec/`   action / tx / block standard codecs
//! - `exec/`    ContextInst / tex (operate helpers live in `base`)
//! - `params`   standard Hacash protocol rules
//! - `setup`    register_standard (+ VM host capability metadata)
//!
//! Public nested paths (`action_std`, `tx_std`, …) are aliases for
//! compatibility with existing external crates.

pub(crate) mod codec;
pub(crate) mod exec;
pub(crate) mod level;
pub(crate) mod params;
pub(crate) mod setup;
pub mod upgrade;

// ---- public nested path aliases (external crates keep using these) ----
pub use codec::action as action_std;
pub use codec::block as block_std;
pub use codec::tx as tx_std;

// ---- crate-root re-exports ----
pub use params::{PROTOCOL_PARAMS, ProtocolParams, execution_params};
pub use setup::register_standard;
