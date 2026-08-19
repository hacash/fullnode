//! `mint-core` — Hacash consensus action definitions and the consensus state types their execution depends on.
//!
//! Split out of `mint` (the miner crate), it serves three consumers:
//! - `mint` (miner)     uses this crate to reuse the inscription/channel/asset/diamond-mint actions and
//!   the MintState/MintTotal state types;
//! - `sdk` (wallet WASM) uses this crate to directly register/build/review all consensus actions
//!   (32-36 inscription, 2/3 channel, 16 asset, 4 diamond mint); it no longer needs codec mirrors;
//!   this crate has no tokio and compiles to wasm32 (x16rs/protocol compile only under the `execute`
//!   feature);
//! - `app` (full node)    gets the same definitions indirectly via mint/sdk.
//!
//! Storage-layout compatibility: `MintTotal` field order, `TOTAL_KEY = b"_mint.total"` and the
//! channel storage keys all match the pre-split layout (pure relocation, no serialization change);
//! mainnet state is unaffected.
//!
//! This crate compiles in both shapes (fullnode with `execute`, SDK/wasm without).
//! The consensus execute helpers in `inscription` are always compiled and
//! dead-code-eliminated from the wasm artifact, so they are legitimately dead
//! code in execute-off builds (same convention as `vm`).

#![allow(dead_code)]

pub mod action;
pub mod inscription;
pub mod schema;
pub mod setup;
pub mod state;

#[cfg(feature = "execute")]
pub mod interest;
#[cfg(feature = "execute")]
pub mod reward;
