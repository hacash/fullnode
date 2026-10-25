//! Hacash consensus action definitions + consensus state types, shared by mint, SDK, and app.
//! No tokio, compiles to wasm32; consensus `exec/` bodies and x16rs are gated by the `execute` feature.

// Policy: the wasm SDK must not link x16rs (the execute body is the only thing
// that pulls it in, and x16rs's 16 sphlib tables would bloat the wasm blob),
// so `execute` stays a native-fullnode-only feature — enforced here, not just
// by convention.
#[cfg(all(target_arch = "wasm32", feature = "execute"))]
compile_error!(
    "the `execute` feature pulls in x16rs and must not be enabled for wasm32 \
     (wasm SDK builds keep it off to avoid binary bloat)"
);

pub mod action;
pub mod inscription;
pub mod state;
mod wire;

#[cfg(feature = "execute")]
pub(crate) mod exec;
#[cfg(feature = "execute")]
pub mod interest;
#[cfg(feature = "execute")]
mod setup;

#[cfg(feature = "execute")]
pub use setup::register_exec;
pub use wire::{ACTION_CODECS, STRUCT_SCHEMAS, register_wire};
