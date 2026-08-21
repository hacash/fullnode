//! `mint` — Hacash consensus, mint actions, and related APIs: `action/` codecs,
//! `consensus/` (HacashConsensus, difficulty, genesis, bidding), `api` HTTP services, `wire` registration.

pub(crate) mod action;
pub mod api;
pub(crate) mod consensus;
pub mod diamond_mining;
pub mod opencl;
mod wire;

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
    LOW_BID_PENDING_ERR, MinerConf, MintConf, block_hasher,
};
pub use wire::{TX_CODECS, register_wire};

// crate-internal path aliases (keep existing `crate::foo` style inside mint)
pub(crate) use consensus::bidding;
pub(crate) use consensus::block_check;
pub(crate) use consensus::coinbase;
pub(crate) use consensus::initialize;
// Consensus state types moved to mint-core (pure relocation; storage layout unchanged).
pub(crate) use mint_core::state;
