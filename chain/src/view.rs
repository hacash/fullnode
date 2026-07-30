//! `ChainView` / `Engine` implementations: the query and admission surface.
//!
//! Every optimistic read pins the captured root so its parent chain stays
//! alive. Validators decide whether work from that view remains publishable.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use base::{
    ApplyMode, BackgroundSyncHandle, BlkPkg, BlockAcceptResult, BlockHistory, BlockProducer,
    BlockRef, BlockSource, ChainListener, ChainView, Consensus, ConsensusNodeHooks, Engine,
    EngineConfig, ExecutionServices, OptimisticState, PipelineOptions, RecentBlock,
    StateReadSession, StateSnapSession, Store, SyncHandle, TxPolicy, TxRef,
};
use field::{Address, Hash};
use sys::{Rerr, Ret};

use crate::engine::ChainEngine;

impl ChainView for ChainEngine {
    fn services(&self) -> &Arc<dyn ExecutionServices> {
        &self.registry
    }

    fn config(&self) -> &EngineConfig {
        &self.config
    }

    fn consensus(&self) -> &dyn Consensus {
        self.consensus.as_ref()
    }

    fn store(&self) -> Arc<dyn Store> {
        self.store.clone()
    }

    fn block_history(&self) -> Arc<dyn BlockHistory> {
        self.block_history.clone()
    }

    fn latest_height(&self) -> u64 {
        self.tree.head_height()
    }

    fn latest_block(&self) -> BlockRef {
        self.tree.head_block()
    }

    fn optimistic_canonical(&self) -> Option<OptimisticState> {
        let hold = self.waiter.try_hold()?;
        if self.waiter.is_shutdown() {
            return None;
        }
        let (head_hash, head_height, epoch, view, root_pin) = self.tree.head_snapshot();
        Some(OptimisticState::new(
            view,
            root_pin,
            head_hash,
            head_height,
            epoch,
            hold,
        ))
    }

    fn validate_optimistic(&self, start_epoch: u64) -> bool {
        self.validate_stable_root(|| self.tree.epoch() == start_epoch)
    }

    fn validate_state_view(&self, tip_hash: &Hash) -> bool {
        self.validate_stable_root(|| self.tree.contains(tip_hash))
    }

    fn state_canonical(&self) -> Option<StateReadSession<'_>> {
        let hold = self.waiter.try_hold()?;
        if self.waiter.is_shutdown() || self.syncing.load(Ordering::Acquire) {
            return None;
        }
        let root_move = self.root_move.try_read().ok()?;
        if !self.is_root_available() {
            return None;
        }
        // A strict insert publishes its tree node before persist_one writes the
        // block body. Packing must not capture that short intermediate state,
        // because difficulty lookup also needs the parent body. Do not wait:
        // the miner will retry on its next cycle.
        let _inserting = self.inserting.try_lock().ok()?;
        let (head_hash, head_height, epoch, view, _root_pin) = self.tree.head_snapshot();
        Some(StateReadSession::new(
            view,
            head_hash,
            head_height,
            epoch,
            hold,
            root_move,
        ))
    }

    fn state_at_session(&self, branch_tip: &Hash) -> Option<StateSnapSession<'_>> {
        let hold = self.waiter.try_hold()?;
        if self.waiter.is_shutdown() {
            return None;
        }
        let (view, root_pin, tip_height) = self.tree.snapshot_at(branch_tip)?;
        Some(StateSnapSession::new(
            view,
            root_pin,
            *branch_tip,
            tip_height,
            hold,
        ))
    }

    fn recent_blocks(&self) -> Vec<RecentBlock> {
        self.recent_snapshot()
    }

    fn average_fee_purity(&self) -> u64 {
        self.avgfee()
    }
}

impl Engine for ChainEngine {
    fn tx_policy(&self) -> &dyn TxPolicy {
        self.consensus.as_ref()
    }

    fn block_producer(&self) -> &dyn BlockProducer {
        self.consensus.as_ref()
    }

    fn node_hooks(&self) -> &dyn ConsensusNodeHooks {
        self.consensus.as_ref()
    }

    fn is_packing_inhibited(&self) -> bool {
        self.syncing.load(Ordering::Acquire) || self.waiter.is_shutdown()
    }

    fn try_execute_tx(&self, tx: TxRef) -> Rerr {
        let snapshot = self
            .optimistic_canonical()
            .ok_or_else(|| sys::Error::fault("chain state unavailable"))?;
        self.check_pending(tx.as_ref())?;
        let chunk = snapshot.begin_tx(tx.hash());
        let env = self.build_tx_env(
            snapshot.head_height + 1,
            self.block_producer().external_exec_author(),
            tx.as_ref(),
        );
        let mut ctx = self
            .registry
            .clone()
            .create_context(env, chunk, tx.clone())?;
        let result = tx.execute(ctx.as_mut());
        drop(ctx);
        result
    }

    fn try_execute_batch(&self, txs: Vec<TxRef>, pending_height: u64) -> Vec<Hash> {
        let Some(snapshot) = self.optimistic_canonical() else {
            return vec![];
        };
        let root = snapshot.begin_block_draft(pending_height);
        let author = self.block_producer().external_exec_author();
        let mut failed = Vec::new();
        for tx in &txs {
            if self
                .execute_candidate(&root, tx, pending_height, author)
                .is_err()
            {
                if self.tx_policy().failed_revalidation_can_remove(tx.as_ref()) {
                    failed.push(tx.hash());
                } else {
                    break;
                }
            }
        }
        failed
    }

    fn try_pick_pending_txs_on_session(
        &self,
        session: &StateReadSession<'_>,
        candidates: Vec<TxRef>,
        pending_height: u64,
        author: Address,
        base_tx_size: usize,
        max_txs: usize,
        max_block_size: usize,
    ) -> Vec<TxRef> {
        let mut execution = session.begin_execution();
        let mut picked = Vec::new();
        let mut total = base_tx_size;
        for tx in candidates {
            if max_txs > 0 && picked.len() + 1 >= max_txs {
                break;
            }
            let size = tx.size();
            if max_block_size > 0 && total + size > max_block_size {
                continue;
            }
            if self.check_pending(tx.as_ref()).is_err() {
                continue;
            }
            let env = self.build_tx_env(pending_height, author, tx.as_ref());
            if execution
                .execute_tx(self.registry.clone(), env, tx.clone())
                .is_ok()
            {
                total += size;
                picked.push(tx);
            }
        }
        picked
    }

    fn discover_block(&self, blk: BlkPkg) -> Ret<BlockAcceptResult> {
        ChainEngine::discover(self, blk)
    }

    fn run_sync(
        &self,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<SyncHandle> {
        crate::sync::run(self, src, mode, opts).map(SyncHandle::done)
    }

    fn run_sync_background(
        self: Arc<Self>,
        src: Box<dyn BlockSource>,
        mode: ApplyMode,
        opts: PipelineOptions,
    ) -> Ret<BackgroundSyncHandle> {
        ChainEngine::spawn_sync(self, src, mode, opts)
    }

    fn add_chain_listener(&self, listener: Arc<dyn ChainListener>) -> Rerr {
        ChainEngine::add_chain_listener(self, listener)
    }

    fn exit(&self) {
        let _ = self.shutdown();
    }
}
