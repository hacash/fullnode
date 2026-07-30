//! Generic chain HTTP API services (status / chain / pool / account).
//!
//! # Three-layer API story
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Transport | `server` | `HttpServer` only - listen, route table, no chain types |
//! | Generic chain | `api` (this crate) | Status / Chain / Pool / Account - TRULY generic, depends only on `base`+`field`+`sys` |
//! | Domain | `mint::api`, `vm::api` | Consensus / miner / diamond / VM + Hacash transfer routes |
//!
//! `app` assembles all three into one service list for `HttpServer::open`.
//! Domain crates do not depend on `server`; `server` does not depend on them.
//! Hacash-specific transfer-build (`/create/coin/transfer`) and transfer-scan
//! (`/query/coin/transfer`) live in `mint::api`, since they encode Hacash
//! transfer semantics and need `protocol`. This crate stays protocol-free.

mod account;
mod chain;
mod pool;
mod status;
mod util;

pub use account::AccountApi;
pub use chain::ChainApi;
pub use pool::PoolApi;
pub use status::StatusApi;
