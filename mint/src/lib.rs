//! `mint` —— Hacash consensus, mint actions, and related APIs.
//!
//! # Module map
//!
//! - `action/`     diamond / asset / channel / coinbase tx (inscription actions split out to `mint-core`)
//! - `consensus/`  HacashConsensus, difficulty, genesis, bidding
//! - `api`         mint HTTP services
//! - `setup`       Registry registration
//!
//! Diamond auto-bidding is a standard full-node runtime service in `app`.
//!
//! Public nested paths (`action_diamond`, `tx_coinbase`, …) are aliases.

pub(crate) mod action;
pub mod api;
pub(crate) mod consensus;
pub mod diamond_mining;
pub mod opencl;
pub mod setup;

// ---- public nested path aliases ----
pub use action::asset as action_asset;
pub use action::channel as action_channel;
pub use action::coinbase_tx as tx_coinbase;
pub use action::diamond as action_diamond;

pub use base::MintParams;
pub use consensus::difficulty;
pub use consensus::genesis;
pub use consensus::minter;

pub use consensus::{
    DIAMOND_FORM_STATE_KEY, DiamondBidding, HacashConsensus, LOW_BID_CACHE_FULL_ERR,
    LOW_BID_PENDING_ERR, MINT_PARAMS, MinerConf, MintConf, block_hasher,
};
pub use setup::register;

// crate-internal path aliases (keep existing `crate::foo` style inside mint)
pub(crate) use consensus::bidding;
pub(crate) use consensus::block_check;
pub(crate) use consensus::coinbase;
pub(crate) use consensus::initialize;
// Consensus state types moved to mint-core (pure relocation; storage layout unchanged).
pub(crate) use mint_core::state;

