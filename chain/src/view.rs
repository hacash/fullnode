//! `ChainView` / `Engine` implementations: the query and admission surface.
//! Immutable, self-consistent snapshots let ordinary queries read without locks (§3.3).

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

use crate::engine::{ChainEngine, Phase};

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

    fn optimistic_canonical(&self) -> Result<Option<OptimisticState>, sys::Error> {
        if self.is_stopping() || self.is_fatal() {
            return Err(unavailable(
                "optimistic_canonical",
                "engine is stopping or fatal",
            ));
        }
        let hold = Some(
            self.waiter
                .try_hold()
                .ok_or_else(|| unavailable("optimistic_canonical", "engine is stopping"))?,
        );
        let (head_hash, head_height, epoch, view, root_pin) = self.tree.head_snapshot();
        Ok(Some(OptimisticState::new(
            view,
            root_pin,
            head_hash,
            head_height,
            epoch,
            hold,
        )))
    }

    /// Optimistic consumers need a consistent head: valid when the root is
    /// available and the canonical epoch is unchanged.
    fn validate_optimistic(&self, start_epoch: u64) -> bool {
        self.is_root_available() && self.tree.epoch() == start_epoch
    }

    fn validate_state_view(&self, tip_hash: &Hash) -> bool {
        self.is_root_available() && self.tree.contains(tip_hash)
    }

    fn state_canonical(&self) -> Result<Option<StateReadSession>, sys::Error> {
        if self.is_stopping() || self.is_fatal() {
            return Err(unavailable(
                "state_canonical",
                "engine is stopping or fatal",
            ));
        }
        let hold = self
            .waiter
            .try_hold()
            .ok_or_else(|| unavailable("state_canonical", "engine is stopping"))?;
        // Busy states are transient: the caller retries. Only fatal/stopping
        // is unavailable.
        if self.syncing.load(Ordering::Acquire) {
            return Ok(None);
        }
        if !self.is_root_available() {
            return Ok(None);
        }
        // A strict insert publishes its tree node before persist_one writes
        // the body; packing must not capture that intermediate state (the miner retries).
        let Some(_inserting) = self.inserting.try_lock().ok() else {
            return Ok(None);
        };
        let (head_hash, head_height, epoch, view, root_pin) = self.tree.head_snapshot();
        Ok(Some(StateReadSession::new(
            view,
            root_pin,
            head_hash,
            head_height,
            epoch,
            hold,
        )))
    }

    fn state_at_session(
        &self,
        branch_tip: &Hash,
    ) -> Result<Option<StateSnapSession<'_>>, sys::Error> {
        if self.is_stopping() || self.is_fatal() {
            return Err(unavailable(
                "state_at_session",
                "engine is stopping or fatal",
            ));
        }
        let hold = Some(
            self.waiter
                .try_hold()
                .ok_or_else(|| unavailable("state_at_session", "engine is stopping"))?,
        );
        let Some((view, root_pin, tip_height)) = self.tree.snapshot_at(branch_tip) else {
            return Ok(None);
        };
        Ok(Some(StateSnapSession::new(
            view,
            root_pin,
            *branch_tip,
            tip_height,
            hold,
        )))
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
        self.syncing.load(Ordering::Acquire) || self.waiter.is_shutdown() || self.is_fatal()
    }

    fn try_execute_tx(&self, tx: TxRef) -> Rerr {
        // Busy (`Ok(None)`) skips this round; fatal/stopping (`Err`) propagates
        // and is never flattened into a busy skip (§5).
        let snapshot = match self.optimistic_canonical() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };
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
        match result {
            // A core state read failure surfaced during execution is
            // engine-fatal: record it at the single engine boundary (§4.1).
            Err(e) if e.is_abort() => self
                .handle_error(
                    Phase::PreAttach,
                    "try_execute_tx",
                    None,
                    None,
                    Err::<(), _>(e),
                )
                .map(|_| ()),
            r => r,
        }
    }

    fn try_execute_batch(&self, txs: Vec<TxRef>, pending_height: u64) -> Ret<Vec<Hash>> {
        // `Ok(None)` (busy) keeps skip-this-round semantics; a fatal/stopping
        // engine (`Err`) propagates and is never flattened (§5).
        let Some(snapshot) = (match self.optimistic_canonical() {
            Ok(snapshot) => snapshot,
            Err(e) => return Err(e),
        }) else {
            return Ok(vec![]);
        };
        let root = snapshot.begin_block_draft(pending_height);
        let author = self.block_producer().external_exec_author();
        let mut failed = Vec::new();
        for tx in &txs {
            match self.execute_candidate(&root, tx, pending_height, author) {
                Ok(()) => {}
                // A core state read failure must not judge any transaction
                // invalid; it propagates to the engine fatal boundary.
                Err(e) if e.is_abort() => return Err(e),
                Err(_) => {
                    if self.tx_policy().failed_revalidation_can_remove(tx.as_ref()) {
                        failed.push(tx.hash());
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(failed)
    }

    fn try_pick_pending_txs_on_session(
        &self,
        session: &StateReadSession,
        candidates: Vec<TxRef>,
        pending_height: u64,
        author: Address,
        base_tx_size: usize,
        max_txs: usize,
        max_block_size: usize,
    ) -> Ret<Vec<TxRef>> {
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
            // A core state read failure is engine-fatal and propagates through
            // the single boundary (§4.1), not a packing-judged transaction; ordinary errors skip the tx.
            match execution.execute_tx(self.registry.clone(), env, tx.clone()) {
                Ok(()) => {
                    total += size;
                    picked.push(tx);
                }
                Err(e) if e.is_abort() => {
                    return self.handle_error(
                        Phase::PreAttach,
                        "try_pick_pending_txs_on_session",
                        None,
                        None,
                        Err::<Vec<TxRef>, _>(e),
                    );
                }
                Err(_) => {}
            }
        }
        Ok(picked)
    }

    fn handle_engine_error(&self, operation: &'static str, err: sys::Error) -> Rerr {
        self.handle_error(Phase::PreAttach, operation, None, None, Err::<(), _>(err))
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

/// The unavailable query error: `EngineUnavailable` with the operation as
/// context (§5; the old `QueryUnavailable` fields ride in the message).
fn unavailable(operation: &'static str, cause: &'static str) -> sys::Error {
    sys::Error::abort(cause)
        .with_code("engine_unavailable")
        .context(operation)
}
