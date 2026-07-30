//! Consensus: HacashConsensus, difficulty, genesis, bidding, mint state.

pub(crate) mod bidding;
pub(crate) mod block_check;
pub(crate) mod coinbase;
pub(crate) mod config;
pub mod difficulty;
pub mod genesis;
pub(crate) mod initialize;
pub mod minter;
pub(crate) mod params;
pub(crate) mod state;

pub use bidding::{DiamondBidding, LOW_BID_CACHE_FULL_ERR, LOW_BID_PENDING_ERR};
pub use config::MintConf;
pub use minter::{DIAMOND_FORM_STATE_KEY, HacashConsensus, MinerConf, block_hasher};
pub use params::MINT_PARAMS;
