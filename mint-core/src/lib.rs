//! Hacash consensus action definitions + consensus state types, shared by mint, SDK, and app.
//! No tokio, compiles to wasm32; consensus `exec/` bodies and x16rs are gated by the `execute` feature.

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
