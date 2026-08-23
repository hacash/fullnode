//! Consensus: HacashConsensus, difficulty, genesis, bidding, mint state.

pub mod bidding;
pub mod block_check;
pub mod coinbase;
pub(crate) mod config;
pub mod difficulty;
pub mod genesis;
pub(crate) mod initialize;
pub mod minter;

pub use bidding::{DiamondBidding, LOW_BID_CACHE_FULL_ERR, LOW_BID_PENDING_ERR};
pub use config::MintConf;
pub use minter::{
    DIAMOND_FORM_STATE_KEY, HacashConsensus, MinerConf, MinerNoticeGuard, MinerPendingWork,
    block_hasher,
};

use field::{Address, Hash};
use sys::Ret;

/// Consensus surface consumed by the miner/mint HTTP API (`MintApi`) and the
/// app-layer miner-notice long-poll. Implemented by `HacashConsensus` (and by
/// side/test-chain consensus implementations outside this crate), so the HTTP
/// layer stays consensus-agnostic.
pub trait ConsensusApi: Send + Sync {
    fn mint_params(&self) -> base::MintParams;
    fn pending_replay_count(&self) -> usize;
    fn miner_enabled(&self) -> bool;
    fn diamond_miner_enabled(&self) -> bool;
    fn diamond_miner_bid_address(&self) -> Address;
    fn diamond_miner_reward_address(&self) -> Address;
    fn miner_notice_count(&self) -> u64;
    fn begin_miner_notice(&self) -> MinerNoticeGuard;
    fn miner_pending_work(
        &self,
        engine: &dyn base::Engine,
        txpool: &dyn base::TxPool,
    ) -> Ret<MinerPendingWork>;
    fn miner_success_block(
        &self,
        reg: &dyn base::BinaryCodecs,
        height: u64,
        block_nonce: u32,
        coinbase_nonce: Hash,
    ) -> Ret<base::BlkPkg>;
    fn miner_mark_block_submitted(&self, height: u64);
    fn diamond_miner_success_tx(
        &self,
        reg: &dyn base::BinaryCodecs,
        view: &dyn base::ChainView,
        txpool: &dyn base::TxPool,
        node: &dyn base::Node,
        action_body: Vec<u8>,
    ) -> Ret<base::TxPkg>;
}

impl ConsensusApi for HacashConsensus {
    fn mint_params(&self) -> base::MintParams {
        self.mint_params()
    }
    fn pending_replay_count(&self) -> usize {
        self.pending_replay_count()
    }
    fn miner_enabled(&self) -> bool {
        self.miner_enabled()
    }
    fn diamond_miner_enabled(&self) -> bool {
        self.diamond_miner_enabled()
    }
    fn diamond_miner_bid_address(&self) -> Address {
        self.diamond_miner_bid_address()
    }
    fn diamond_miner_reward_address(&self) -> Address {
        self.diamond_miner_reward_address()
    }
    fn miner_notice_count(&self) -> u64 {
        self.miner_notice_count()
    }
    fn begin_miner_notice(&self) -> MinerNoticeGuard {
        self.begin_miner_notice()
    }
    fn miner_pending_work(
        &self,
        engine: &dyn base::Engine,
        txpool: &dyn base::TxPool,
    ) -> Ret<MinerPendingWork> {
        self.miner_pending_work(engine, txpool)
    }
    fn miner_success_block(
        &self,
        reg: &dyn base::BinaryCodecs,
        height: u64,
        block_nonce: u32,
        coinbase_nonce: Hash,
    ) -> Ret<base::BlkPkg> {
        self.miner_success_block(reg, height, block_nonce, coinbase_nonce)
    }
    fn miner_mark_block_submitted(&self, height: u64) {
        self.miner_mark_block_submitted(height)
    }
    fn diamond_miner_success_tx(
        &self,
        reg: &dyn base::BinaryCodecs,
        view: &dyn base::ChainView,
        txpool: &dyn base::TxPool,
        node: &dyn base::Node,
        action_body: Vec<u8>,
    ) -> Ret<base::TxPkg> {
        self.diamond_miner_success_tx(reg, view, txpool, node, action_body)
    }
}
