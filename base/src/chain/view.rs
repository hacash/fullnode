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

    /// Optimistic canonical snapshot: head, state tip, root pin, epoch captured together
    /// under the Tree lock. `Ok(None)` = engine busy (retry); `Err(EngineUnavailable)` = fatal/stopping (§5).
    fn optimistic_canonical(&self) -> Result<Option<OptimisticState>, sys::Error>;

    /// Exact canonical-head validation for work whose result is only useful on
    /// the same head, such as block-template construction.
    fn validate_optimistic(&self, start_epoch: u64) -> bool;

    /// Check a captured branch tip still belongs to the current durable-root
    /// subtree — state-view consistency without requiring the head to be unchanged.
    fn validate_state_view(&self, tip_hash: &Hash) -> bool;

    /// Root-pinned read session for miner packing: root rolls stay readable; the
    /// epoch check still detects an ordinary head change.
    fn state_canonical(&self) -> Result<Option<StateReadSession>, sys::Error>;

    /// Optimistic branch snapshot for indexer reads. Follow the read with
    /// `validate_state_view(&session.tip_hash())`; `Ok(None)` = tip not in the tree.
    fn state_at_session(
        &self,
        branch_tip: &Hash,
    ) -> Result<Option<StateSnapSession<'_>>, sys::Error>;

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

    /// §8.1 step 1 / §10: whether miner packing should be inhibited (activity channel
    /// owned by `Sync`/`Recovery`/`Stopping`). Consulted before `state_canonical`; non-blocking.
    fn is_packing_inhibited(&self) -> bool {
        false
    }

    /// Execute a batch against pending state; returns failed-tx hashes (removable from
    /// the txpool). An `Abort` returns as `Err` so it reaches the engine fatal boundary (§6.7).
    fn try_execute_batch(&self, _txs: Vec<TxRef>, _pending_height: u64) -> Ret<Vec<Hash>> {
        Ok(vec![])
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

    /// Best-effort packing filter on a root-pinned session. An `Abort` (core state read
    /// failure) returns as `Err` so it reaches the engine fatal boundary, not judged unsuitable (§5).
    #[allow(clippy::too_many_arguments)]
    fn try_pick_pending_txs_on_session(
        &self,
        session: &StateReadSession,
        candidates: Vec<TxRef>,
        pending_height: u64,
        author: Address,
        base_tx_size: usize,
        max_txs: usize,
        max_block_size: usize,
    ) -> Ret<Vec<TxRef>>;

    /// Route an error observed outside the engine into the single engine boundary (§4.1):
    /// `Abort` marks the engine fatal (recorded once), other errors only warn. Default passes through.
    fn handle_engine_error(&self, _operation: &'static str, err: sys::Error) -> Rerr {
        Err(err)
    }

    /// Insert a single block (discover path); sync/rebuild use the stream APIs.
    fn discover_block(&self, blk: BlkPkg) -> Ret<BlockAcceptResult>;

    /// Insert a block stream from `src`. `mode` is `Strict` (P2P) or
    /// `P2pFastSync`.
    fn run_sync(
        &self,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<SyncHandle>;

    /// Run `run_sync` on a background task (including `P2pFastSync`).
    fn run_sync_background(
        self: Arc<Self>,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<BackgroundSyncHandle>;

    /// Register a chain listener.
    fn add_chain_listener(&self, _listener: Arc<dyn ChainListener>) -> Rerr {
        sys::errf!("chain listener registration not supported")
    }

    fn exit(&self) {}
}
