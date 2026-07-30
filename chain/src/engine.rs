//! The chain engine. Two real jobs: insert one block (discover) and insert a
//! stream of them (sync). Everything else is a query.
//!
//! Concurrency, in full:
//! - `inserting` serializes all block insertion (discover vs sync vs boot).
//! - the tree has its own short-lived lock for lookups and attaching.
//! - critical state readers hold `root_move` shared; optimistic readers pin a
//!   Tree root and validate after use.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

use base::{
    ApplyMode, BlkPkg, BlockAcceptResult, BlockRef, BlockSource, ChainListener, ConsensusRuntime,
    EngineConfig, Env, ExecutionServices, PipelineOptions, RecentBlock, Store, TxRef,
};
use field::{Address, Hash};
use sys::{Rerr, Ret, errf};

use crate::history::StoreHistory;
use crate::insert::Insert;
use crate::tree::Tree;

pub struct ChainEngine {
    pub(crate) registry: Arc<dyn ExecutionServices>,
    pub(crate) config: EngineConfig,
    pub(crate) consensus: Arc<dyn ConsensusRuntime>,
    pub(crate) store: Arc<dyn Store>,
    pub(crate) tree: Tree,
    pub(crate) block_history: Arc<StoreHistory>,
    pub(crate) genesis: BlockRef,
    pub(crate) waiter: sys::Waiter,
    pub(crate) listeners: Mutex<Vec<Arc<dyn ChainListener>>>,
    /// Held for the whole of any block insertion, single block or stream.
    pub(crate) inserting: Mutex<()>,
    /// Critical state execution holds a read guard. Root persistence and tree
    /// reset hold the write guard across both the disk and tree transitions.
    pub(crate) root_move: RwLock<()>,
    /// Even while stable, odd while a root writer is between disk and tree.
    pub(crate) root_version: AtomicU64,
    /// False while moving or recovering the root, and after any failed root
    /// write until recovery explicitly publishes a valid state again.
    pub(crate) root_available: AtomicBool,
    /// Set while a sync stream owns `inserting`, so discover and miner packing
    /// can back off without blocking on the mutex.
    pub(crate) syncing: AtomicBool,
    mempool_min_fee_purity: u64,
    recent: RwLock<VecDeque<RecentBlock>>,
    avgfees: Mutex<VecDeque<u64>>,
    background: Mutex<Vec<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>>,
    sync_cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    shutdown_lock: Mutex<()>,
    shutdown_complete: AtomicBool,
    next_sync_id: AtomicU64,
}

pub(crate) struct SyncCancelRegistration<'a> {
    engine: &'a ChainEngine,
    id: u64,
}

impl Drop for SyncCancelRegistration<'_> {
    fn drop(&mut self) {
        self.engine.sync_cancels.lock().unwrap().remove(&self.id);
    }
}

pub(crate) enum PreparedBlock {
    Accepted(PersistJob),
    Duplicate(Hash),
    Orphan(Hash),
}

pub(crate) struct PersistJob {
    pub pkg: BlkPkg,
    pub height: u64,
    pub confirmed_txs: Vec<Hash>,
    pub reverted_txs: Vec<Hash>,
    pub is_head: bool,
    pub reorg: bool,
    pub roll: Option<crate::tree::RollJob>,
    pub persist_body: bool,
}

pub(crate) struct PersistOutcome {
    pub result: BlockAcceptResult,
    pub rolled: u64,
    pub events: u64,
}

fn recent_block(blk: &dyn base::Block, arrive: u64) -> RecentBlock {
    let prelude = blk.prelude_transaction().ok();
    let miner = prelude
        .and_then(|tx| tx.author())
        .unwrap_or_else(|| prelude.map(|tx| tx.main()).unwrap_or_default());
    let reward = prelude
        .and_then(|tx| tx.block_reward())
        .cloned()
        .unwrap_or_default();
    let message = prelude
        .and_then(|tx| tx.block_message())
        .map(|msg| sys::left_readable_string(msg.as_ref()))
        .unwrap_or_default();
    let timestamp = blk.timestamp();

    RecentBlock {
        height: blk.height(),
        hash: blk.hash(),
        prev: blk.prev_hash(),
        txs: blk.transaction_count(),
        miner,
        message,
        reward,
        timestamp,
        arrive,
    }
}

impl ChainEngine {
    pub fn open(
        registry: Arc<dyn ExecutionServices>,
        config: EngineConfig,
        consensus: Arc<dyn ConsensusRuntime>,
        store: Arc<dyn Store>,
        waiter: sys::Waiter,
        mempool_min_fee_purity: u64,
    ) -> Ret<Arc<Self>> {
        let state_status = store.state_status()?;
        let genesis = consensus.genesis_block();
        let block_history = Arc::new(crate::history::StoreHistory::new(
            store.clone(),
            registry.clone(),
            genesis.clone(),
        ));
        let root_block = match &state_status {
            base::StateStatus::Uninitialized => genesis.clone(),
            base::StateStatus::Ready(status) => {
                load_persisted_root_block(registry.as_ref(), store.as_ref(), &genesis, status)?
            }
        };
        let eng = Arc::new(Self {
            tree: Tree::new(store.disk(), root_block),
            registry,
            config,
            consensus,
            store,
            block_history,
            genesis,
            waiter,
            listeners: Mutex::new(Vec::new()),
            inserting: Mutex::new(()),
            root_move: RwLock::new(()),
            root_version: AtomicU64::new(0),
            root_available: AtomicBool::new(true),
            syncing: AtomicBool::new(false),
            mempool_min_fee_purity,
            recent: RwLock::new(VecDeque::new()),
            avgfees: Mutex::new(VecDeque::new()),
            background: Mutex::new(Vec::new()),
            sync_cancels: Mutex::new(HashMap::new()),
            shutdown_lock: Mutex::new(()),
            shutdown_complete: AtomicBool::new(false),
            next_sync_id: AtomicU64::new(1),
        });
        crate::boot::open_state(&eng, state_status)?;
        Ok(eng)
    }

    pub fn add_chain_listener(&self, listener: Arc<dyn ChainListener>) -> Rerr {
        self.listeners.lock().unwrap().push(listener);
        Ok(())
    }

    pub(crate) fn begin_root_move(&self) -> RootMoveGuard<'_> {
        let lock = self.root_move.write().unwrap();
        let previous = self.root_version.fetch_add(1, Ordering::AcqRel);
        assert!(previous.is_multiple_of(2), "nested root movement");
        self.root_available.store(false, Ordering::Release);
        RootMoveGuard {
            version: &self.root_version,
            available: &self.root_available,
            committed: false,
            _lock: lock,
        }
    }

    /// Run a non-blocking optimistic validation against a stable root phase.
    /// A root move before, during, or immediately after `check` makes the
    /// result false; compatible completed moves may be accepted by `check`.
    pub(crate) fn validate_stable_root(&self, check: impl FnOnce() -> bool) -> bool {
        validate_root_state(&self.root_available, &self.root_version, check)
    }

    pub(crate) fn is_root_available(&self) -> bool {
        self.root_available.load(Ordering::Acquire)
    }

    /// Ordered fast-sync execution may overlap an active compatible root
    /// writer. An even unavailable version means that writer failed and the
    /// pipeline must stop for recovery.
    pub(crate) fn is_root_readable_for_fast_sync(&self) -> bool {
        let version = self.root_version.load(Ordering::Acquire);
        !version.is_multiple_of(2) || self.root_available.load(Ordering::Acquire)
    }

    pub fn shutdown(&self) -> Rerr {
        let _shutdown = self.shutdown_lock.lock().unwrap();
        if self.shutdown_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        self.waiter.trigger();
        let cancels: Vec<_> = self
            .sync_cancels
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        for cancel in cancels {
            cancel.store(true, Ordering::Release);
        }
        let tasks: Vec<_> = std::mem::take(&mut *self.background.lock().unwrap());
        let mut worker_panicked = false;
        for (cancel, handle) in tasks {
            cancel.store(true, Ordering::Release);
            worker_panicked |= handle.join().is_err();
        }
        self.waiter.wait_complete();
        self.consensus.exit();
        self.shutdown_complete.store(true, Ordering::Release);
        println!("[Engine] exit.");
        if worker_panicked {
            return errf!("chain background sync worker panicked during shutdown");
        }
        Ok(())
    }

    /// Insert one block that arrived from the network or a local producer.
    pub fn discover(&self, pkg: BlkPkg) -> Ret<BlockAcceptResult> {
        let Some(_hold) = self.waiter.try_hold() else {
            return errf!("engine is stopping");
        };
        if self.syncing.load(Ordering::Acquire) {
            return Err(
                sys::Error::fault("chain is syncing; block deferred").with_code("deferred_sync")
            );
        }
        crate::verify::check_intrinsic(self, &pkg)?;
        self.consensus.check_block_arrive(&pkg, self)?;

        let _guard = self.inserting.lock().unwrap();
        if self.waiter.is_shutdown() {
            return errf!("engine is stopping");
        }
        match self.apply_one(&pkg, ApplyMode::Strict, true) {
            Ok(result) => Ok(result),
            Err(e) => {
                if let Err(re) = crate::boot::recover(self) {
                    return errf!("block insert failed: {}; recovery failed: {}", e, re);
                }
                Err(e)
            }
        }
    }

    /// The shared tail of discover and sync: insert, persist, report.
    /// Caller holds `inserting`.
    pub(crate) fn apply_one(
        &self,
        pkg: &BlkPkg,
        mode: ApplyMode,
        persist_body: bool,
    ) -> Ret<BlockAcceptResult> {
        match self.prepare_one(pkg, mode, persist_body)? {
            PreparedBlock::Accepted(job) => Ok(self.persist_one(job, true)?.result),
            PreparedBlock::Duplicate(hash) => Ok(BlockAcceptResult::duplicate(hash)),
            PreparedBlock::Orphan(parent) => Ok(BlockAcceptResult::orphan(parent)),
        }
    }

    /// Execute and attach a block without performing any I/O. Fast historical
    /// sync sends the returned job to the ordered persistence stage.
    pub(crate) fn prepare_one(
        &self,
        pkg: &BlkPkg,
        mode: ApplyMode,
        persist_body: bool,
    ) -> Ret<PreparedBlock> {
        let accepted = match crate::insert::insert_block(self, pkg, mode)? {
            Insert::Accepted {
                height,
                confirmed_txs,
                reverted_txs,
                is_head,
                reorg,
                roll,
            } => (height, confirmed_txs, reverted_txs, is_head, reorg, roll),
            Insert::Duplicate => return Ok(PreparedBlock::Duplicate(pkg.hash())),
            Insert::Orphan(parent) => return Ok(PreparedBlock::Orphan(parent)),
        };
        let (height, confirmed_txs, reverted_txs, is_head, reorg, roll) = accepted;
        if is_head {
            if reorg {
                // Pending entries describe the previous canonical branch. The
                // strict path persists the replacement before another insert.
                self.block_history.clear_pending();
            }
            self.block_history.remember(pkg.block_ref());
        }
        Ok(PreparedBlock::Accepted(PersistJob {
            pkg: pkg.clone(),
            height,
            confirmed_txs,
            reverted_txs,
            is_head,
            reorg,
            roll,
            persist_body,
        }))
    }

    /// Persist one already-executed block. Jobs must be supplied in insertion
    /// order; root commits validate that ordering independently.
    pub(crate) fn persist_one(
        &self,
        job: PersistJob,
        maintain_runtime_caches: bool,
    ) -> Ret<PersistOutcome> {
        let PersistJob {
            pkg,
            height,
            confirmed_txs,
            reverted_txs,
            is_head,
            reorg,
            roll,
            persist_body,
        } = job;
        // A false persistence flag is only supplied by the internal replay
        // pipeline, whose block body and canonical index already exist.
        let stored_replay = !persist_body;
        if persist_body {
            let store = self.store.block_store();
            if is_head && reorg {
                let depth = height.saturating_sub(self.tree.root_height());
                let mut canonical = self.tree.back_hashes(depth);
                canonical.reverse();
                store.commit_reorg(height, &pkg.hash(), pkg.data().clone(), &canonical)?;
            } else if is_head {
                store.put_block_available(height, &pkg.hash(), pkg.data().clone())?;
            } else {
                store.put_block(height, &pkg.hash(), pkg.data().clone())?;
            }
        }
        let mut rolled = 0;
        if let Some(job) = roll {
            rolled = crate::roll::roll_root(self, job, pkg.origin(), stored_replay)?.len() as u64;
        }
        if maintain_runtime_caches && is_head {
            self.record_recent(pkg.block());
            self.record_avgfee(pkg.block());
        }
        for listener in self.listeners.lock().unwrap().iter() {
            listener.on_block_accepted(height, pkg.origin());
        }
        Ok(PersistOutcome {
            result: BlockAcceptResult::accepted(height, confirmed_txs, reverted_txs),
            rolled,
            events: 1 + rolled,
        })
    }

    fn record_recent(&self, blk: &dyn base::Block) {
        if !self.config.recent_blocks {
            return;
        }
        let keep_above = self
            .tree
            .root_height()
            .saturating_sub(self.config.unstable_block);
        let mut recent = self.recent.write().unwrap();
        recent.retain(|item| item.height > keep_above);
        recent.push_front(recent_block(blk, sys::curtimes()));
    }

    /// Mid-third fee purity sample, matching mainnet's wallet fee API.
    fn record_avgfee(&self, blk: &dyn base::Block) {
        if !self.config.average_fee_purity {
            return;
        }
        let sample = self.fee_sample(blk);
        let mut fees = self.avgfees.lock().unwrap();
        fees.push_front(sample);
        while fees.len() > 8 {
            fees.pop_back();
        }
    }

    fn fee_sample(&self, blk: &dyn base::Block) -> u64 {
        let txs = blk.transactions();
        let mut sample = self.mempool_min_fee_purity;
        if txs.len() >= 30 {
            let third = txs.len() / 3;
            let total: u128 = txs[third..third * 2]
                .iter()
                .map(|tx| tx.fee_purity() as u128)
                .sum();
            sample = (total / third as u128) as u64;
        }
        sample
    }

    /// Rebuild runtime-only historical caches after a sync run. Both new
    /// collections are complete before either live cache is replaced.
    pub(crate) fn rebuild_runtime_caches(&self) -> Rerr {
        let tip = self.store.block_store().available_cursor().unwrap_or(0);
        let root_height = self.tree.root_height();
        let recent_start = root_height
            .saturating_sub(self.config.unstable_block)
            .saturating_add(1)
            .max(1);
        let fee_start = tip.saturating_sub(7).max(1);
        let decode_start = match (self.config.recent_blocks, self.config.average_fee_purity) {
            (true, true) => recent_start.min(fee_start),
            (true, false) => recent_start,
            (false, true) => fee_start,
            (false, false) => tip.saturating_add(1),
        };

        let mut recent = VecDeque::new();
        let mut fees = VecDeque::new();
        if decode_start <= tip {
            for height in decode_start..=tip {
                let Some((_hash, data)) = self.store.block_data_by_height(height) else {
                    return errf!("cannot rebuild runtime caches: block {} is missing", height);
                };
                let (block, _) = self
                    .registry
                    .decode_block(&data)
                    .map_err(|e| format!("cannot rebuild runtime caches at {}: {}", height, e))?;
                if self.config.recent_blocks && height >= recent_start {
                    recent.push_front(recent_block(block.as_ref(), 0));
                }
                if self.config.average_fee_purity && height >= fee_start {
                    fees.push_front(self.fee_sample(block.as_ref()));
                    while fees.len() > 8 {
                        fees.pop_back();
                    }
                }
            }
        }

        *self.recent.write().unwrap() = recent;
        *self.avgfees.lock().unwrap() = fees;
        Ok(())
    }

    pub(crate) fn build_tx_env(
        &self,
        height: u64,
        author: Address,
        tx: &dyn base::Transaction,
    ) -> Env {
        let mut env = Env::default();
        env.chain.id = self.consensus.chain_id();
        env.chain.consensus_flags = self.consensus.chain_flags(height);
        env.block.height = height;
        env.block.author = author;
        env.tx = base::TxInfo {
            ty: tx.ty(),
            main: tx.main(),
            addrs: tx.addrs(),
            fee: tx.fee().clone(),
        };
        env
    }

    pub(crate) fn register_sync_cancel(
        &self,
        cancel: Arc<AtomicBool>,
    ) -> SyncCancelRegistration<'_> {
        let id = self.next_sync_id.fetch_add(1, Ordering::Relaxed);
        let mut cancels = self.sync_cancels.lock().unwrap();
        if self.waiter.is_shutdown() {
            cancel.store(true, Ordering::Release);
        } else {
            cancels.insert(id, cancel);
        }
        drop(cancels);
        SyncCancelRegistration { engine: self, id }
    }

    pub(crate) fn track_background(
        &self,
        cancel: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
    ) {
        let mut background = self.background.lock().unwrap();
        if self.waiter.is_shutdown() {
            cancel.store(true, Ordering::Release);
            drop(background);
            let _ = handle.join();
        } else {
            background.push((cancel, handle));
        }
    }

    pub(crate) fn recent_snapshot(&self) -> Vec<RecentBlock> {
        self.recent.read().unwrap().iter().cloned().collect()
    }

    pub(crate) fn avgfee(&self) -> u64 {
        let fees = self.avgfees.lock().unwrap();
        if fees.is_empty() {
            return self.mempool_min_fee_purity;
        }
        let total: u128 = fees.iter().map(|v| *v as u128).sum();
        (total / fees.len() as u128) as u64
    }

    /// Limits a transaction must satisfy before it may be executed against
    /// pending state, whether for admission or for block packing.
    pub(crate) fn check_pending(&self, tx: &dyn base::Transaction) -> Rerr {
        if tx.is_block_prelude() {
            return errf!("a prelude transaction cannot be executed as a user transaction");
        }
        let params = self.consensus.mint_params();
        if params.max_tx_size > 0 && tx.size() > params.max_tx_size {
            return errf!("tx size {} exceeds max {}", tx.size(), params.max_tx_size);
        }
        if tx.action_count() != tx.actions().len() {
            return errf!("tx action count does not match its decoded actions");
        }
        if tx.timestamp().value() > sys::curtimes() {
            return errf!("tx timestamp {} is in the future", tx.timestamp().value());
        }
        Ok(())
    }

    /// Executes one candidate into `root`, leaving `root` untouched on failure.
    pub(crate) fn execute_candidate(
        &self,
        root: &base::StateChunkRef,
        tx: &TxRef,
        pending_height: u64,
        author: Address,
    ) -> Rerr {
        self.check_pending(tx.as_ref())?;
        let env = self.build_tx_env(pending_height, author, tx.as_ref());
        let child = root.spawn_tx_child(tx.hash())?;
        let mut ctx = self
            .registry
            .clone()
            .create_context(env, child, tx.clone())?;
        match tx.execute(ctx.as_mut()) {
            Ok(()) => {
                let child = ctx.release_chunk()?;
                let parent = child.commit_to_parent()?;
                debug_assert!(parent.ptr_eq(root));
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Run a sync stream on its own thread.
    pub(crate) fn spawn_sync(
        engine: Arc<Self>,
        mut src: Box<dyn BlockSource>,
        mode: ApplyMode,
        mut opts: PipelineOptions,
    ) -> Ret<base::BackgroundSyncHandle> {
        let cancel = opts
            .cancel
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        opts.cancel = Some(cancel.clone());
        src.set_cancel(Some(cancel.clone()));
        let progress = opts
            .progress
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(base::PipelineReport::default())));
        opts.progress = Some(progress.clone());

        let worker = engine.clone();
        let handle = std::thread::Builder::new()
            .name("chain-sync".to_owned())
            .spawn(move || {
                if let Err(e) = crate::sync::run(&worker, src, mode, opts) {
                    eprintln!("[Block Sync Warning] {}", e);
                }
            })
            .map_err(|e| sys::Error::fault(format!("cannot spawn sync thread: {}", e)))?;
        engine.track_background(cancel.clone(), handle);
        Ok(base::BackgroundSyncHandle::new(cancel, progress))
    }
}

pub(crate) fn load_persisted_root_block(
    registry: &dyn ExecutionServices,
    store: &dyn Store,
    genesis: &BlockRef,
    status: &base::ChainStatus,
) -> Ret<BlockRef> {
    if status.latest_height == 0 {
        if status.latest_hash != genesis.hash() {
            return errf!(
                "state root hash {:?} does not match genesis {:?}",
                status.latest_hash,
                genesis.hash()
            );
        }
        return Ok(genesis.clone());
    }
    let data = store.block_data(&status.latest_hash).ok_or_else(|| {
        format!(
            "state root block <{}, {:?}> is missing from block db",
            status.latest_height, status.latest_hash
        )
    })?;
    let block = registry.decode_block_exact(&data)?;
    if block.height() != status.latest_height || block.hash() != status.latest_hash {
        return errf!(
            "state root block identity mismatch: expected <{}, {:?}>, got <{}, {:?}>",
            status.latest_height,
            status.latest_hash,
            block.height(),
            block.hash()
        );
    }
    Ok(block)
}

fn validate_root_state(
    available: &AtomicBool,
    version: &AtomicU64,
    check: impl FnOnce() -> bool,
) -> bool {
    if !available.load(Ordering::Acquire) {
        return false;
    }
    let before = version.load(Ordering::Acquire);
    if !before.is_multiple_of(2) {
        return false;
    }
    let valid = check();
    let after = version.load(Ordering::Acquire);
    valid && before == after && available.load(Ordering::Acquire)
}

pub(crate) struct RootMoveGuard<'a> {
    version: &'a AtomicU64,
    available: &'a AtomicBool,
    committed: bool,
    _lock: RwLockWriteGuard<'a, ()>,
}

impl RootMoveGuard<'_> {
    pub fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for RootMoveGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            self.available.store(true, Ordering::Release);
        }
        let previous = self.version.fetch_add(1, Ordering::Release);
        debug_assert!(!previous.is_multiple_of(2));
    }
}

//////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_root_move_remains_unavailable_until_recovery() {
        let lock = RwLock::new(());
        let version = AtomicU64::new(1);
        let available = AtomicBool::new(false);
        {
            let _guard = RootMoveGuard {
                version: &version,
                available: &available,
                committed: false,
                _lock: lock.write().unwrap(),
            };
        }
        assert_eq!(version.load(Ordering::Acquire), 2);
        assert!(!available.load(Ordering::Acquire));

        version.store(3, Ordering::Release);
        {
            let mut recovery = RootMoveGuard {
                version: &version,
                available: &available,
                committed: false,
                _lock: lock.write().unwrap(),
            };
            recovery.commit();
        }
        assert_eq!(version.load(Ordering::Acquire), 4);
        assert!(available.load(Ordering::Acquire));
    }

    #[test]
    fn optimistic_validation_rejects_every_root_transition_window() {
        let available = AtomicBool::new(true);
        let version = AtomicU64::new(0);
        assert!(validate_root_state(&available, &version, || true));

        version.store(1, Ordering::Release);
        assert!(!validate_root_state(&available, &version, || true));

        version.store(2, Ordering::Release);
        assert!(!validate_root_state(&available, &version, || {
            version.store(4, Ordering::Release);
            true
        }));

        assert!(!validate_root_state(&available, &version, || {
            available.store(false, Ordering::Release);
            true
        }));
    }
}
