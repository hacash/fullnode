use std::sync::Arc;

use field::{Address, Hash};
use sys::{Rerr, Ret};

use crate::chain::ApplyMode;
use crate::chain::BlkPkg;
use crate::chain::{
    BlockAcceptResult, BlockHistory, BlockProducer, ChainListener, Consensus, ConsensusNodeHooks,
    EngineConfig, OptimisticState, RecentBlock, StateReadSession, StateSnapSession, TxPolicy,
};
use crate::registry::ExecutionServices;
use crate::store::Store;
use crate::sync::{BackgroundSyncHandle, BlockSource, PipelineOptions, SyncHandle};
use crate::{BlockRef, TxRef};

pub trait ChainView: Send + Sync {
    fn services(&self) -> &Arc<dyn ExecutionServices>;
    fn config(&self) -> &EngineConfig;
    fn consensus(&self) -> &dyn Consensus;

    fn store(&self) -> Arc<dyn Store>;
    fn block_history(&self) -> Arc<dyn BlockHistory>;

    fn latest_height(&self) -> u64 {
        self.latest_block().height()
    }
    fn latest_block(&self) -> BlockRef;

    /// Optimistic canonical snapshot. Captures head, state tip, root pin and
    /// epoch together under the Tree lock. API queries and VM sandbox validate
    /// at the end; transaction relay may use execution only as a best-effort
    /// filter and skip validation.
    fn optimistic_canonical(&self) -> Option<OptimisticState>;

    /// Exact canonical-head validation for work whose result is only useful on
    /// the same head, such as block-template construction.
    fn validate_optimistic(&self, start_epoch: u64) -> bool;

    /// Validate that a captured branch tip still belongs to the current
    /// durable-root subtree. This checks state-view consistency without
    /// requiring the canonical head to remain unchanged.
    fn validate_state_view(&self, tip_hash: &Hash) -> bool;

    /// Root-stable read session for miner packing. Root movement is excluded,
    /// while the epoch check still detects an ordinary head change.
    fn state_canonical(&self) -> Option<StateReadSession<'_>>;

    /// Optimistic branch snapshot for indexer reads. The complete read must be
    /// followed by `validate_state_view(&session.tip_hash())`.
    fn state_at_session(&self, branch_tip: &Hash) -> Option<StateSnapSession<'_>>;

    fn recent_blocks(&self) -> Vec<RecentBlock> {
        vec![]
    }
    fn average_fee_purity(&self) -> u64 {
        0
    }
}

pub trait Engine: ChainView {
    fn tx_policy(&self) -> &dyn TxPolicy;
    fn block_producer(&self) -> &dyn BlockProducer;
    fn node_hooks(&self) -> &dyn ConsensusNodeHooks;

    fn try_execute_tx(&self, tx: TxRef) -> Rerr;

    /// §8.1 step 1 / §10: whether miner packing should be inhibited right now.
    /// Returns `true` when the activity channel is owned by `Sync`,
    /// `Recovery`, `Stopping`, or when sync has been requested
    /// (`sync_waiting`).  The miner consults this BEFORE attempting the strict
    /// `state_canonical` session so it can return `None` (Busy) without
    /// contending for the StateGate read while a sync writer is committing.
    /// This query does NOT take the StateGate and does NOT enter the activity
    /// channel - it is a non-blocking ownership snapshot.
    fn is_packing_inhibited(&self) -> bool {
        false
    }

    fn try_execute_batch(&self, _txs: Vec<TxRef>, _pending_height: u64) -> Vec<Hash> {
        vec![]
    }

    fn try_pick_pending_txs(
        &self,
        candidates: Vec<TxRef>,
        _pending_height: u64,
        _author: Address,
        _base_tx_size: usize,
        _max_txs: usize,
        _max_block_size: usize,
    ) -> Vec<TxRef> {
        let mut picked = Vec::new();
        for tx in candidates {
            if self.try_execute_tx(tx.clone()).is_ok() {
                picked.push(tx);
            }
        }
        picked
    }

    #[allow(clippy::too_many_arguments)]
    fn try_pick_pending_txs_on_session(
        &self,
        session: &StateReadSession<'_>,
        candidates: Vec<TxRef>,
        pending_height: u64,
        author: Address,
        base_tx_size: usize,
        max_txs: usize,
        max_block_size: usize,
    ) -> Vec<TxRef>;

    /// try_execute_tx  sync/rebuild
    fn discover_block(&self, blk: BlkPkg) -> Ret<BlockAcceptResult>;

    /// `src`
    /// `mode` Strict  P2PFastSync
    fn run_sync(
        &self,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<SyncHandle>;

    /// block  + FastSync
    fn run_sync_background(
        self: Arc<Self>,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<BackgroundSyncHandle>;

    /// `open`
    fn add_chain_listener(&self, _listener: Arc<dyn ChainListener>) -> Rerr {
        sys::errf!("chain listener registration not supported")
    }

    fn exit(&self) {}
}
