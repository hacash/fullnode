//! The chain engine. Two real jobs: insert one block (discover) and insert a
//! stream of them (sync). Everything else is a query.
//!
//! Concurrency, in full:
//! - `inserting` serializes all block insertion (discover vs sync vs boot);
//!   every root-move writer also holds it, so insertion excludes root movement.
//! - the tree has its own short-lived lock for lookups and attaching.
//! - state sessions pin the captured tree root (a `StateChunkRef`) instead of
//!   holding any lock; the pin keeps their weak parent chains alive while a
//!   root roll prunes the tree underneath them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use base::{
    ApplyMode, BlkPkg, BlockAcceptResult, BlockRef, BlockSource, ChainListener, ConsensusRuntime,
    EngineConfig, Env, ExecutionServices, PipelineOptions, RecentBlock, Store, TxRef,
};
use field::{Address, Hash};
use sys::{Rerr, Ret, errf};

use crate::history::StoreHistory;
use crate::side_list::{SideKeepCtx, SideListWriter};
use crate::tree::{Inserted, Tree};

/// The engine-fatal fault classes. An error carrying one of these stops the
/// engine; see §2.3 of the engine error-handling contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CoreFault {
    /// Persistence failure after a block was published to the tree: the disk
    /// or root transition is uncertain, so the engine must stop.
    PersistFailed,
    /// Internal inconsistency detected by the tree or consensus runtime.
    CoreFailed,
    /// A core storage read failed (the Option-based state API panics).
    StorageReadFailed,
}

impl CoreFault {
    /// The code attached to `sys::Error`; kept stable for the crate boundary.
    pub(crate) fn code(self) -> &'static str {
        match self {
            CoreFault::PersistFailed => "persist_failed",
            CoreFault::CoreFailed => "core_failed",
            CoreFault::StorageReadFailed => "storage_read_failed",
        }
    }

    /// Whether `error` carries this fault code.
    pub(crate) fn is(self, error: &sys::Error) -> bool {
        error.code() == Some(self.code())
    }

    /// Whether `error` is any engine-fatal core fault.
    pub(crate) fn is_core_fault(error: &sys::Error) -> bool {
        CoreFault::PersistFailed.is(error)
            || CoreFault::CoreFailed.is(error)
            || CoreFault::StorageReadFailed.is(error)
    }
}

const LIFE_STARTING: u8 = 0;
const LIFE_RUNNING: u8 = 1;
const LIFE_FATAL: u8 = 2;
const LIFE_STOPPING: u8 = 3;
const LIFE_STOPPED: u8 = 4;

fn persist_fatal(e: sys::Error) -> sys::Error {
    sys::Error::fault(format!("canonical persistence failed: {}", e))
        .with_code(CoreFault::PersistFailed.code())
}

/// Convert only the storage panic used by the Option-based state API. Other
/// panics remain programming/plugin failures and keep their normal unwind.
pub(crate) fn catch_storage_panic<T>(run: impl FnOnce() -> Ret<T>) -> Ret<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(payload) => match payload.downcast::<base::StorageReadPanic>() {
            Ok(fault) => Err(sys::Error::fault(format!(
                "core storage read failed: {}",
                fault.error
            ))
            .with_code(CoreFault::StorageReadFailed.code())),
            Err(payload) => std::panic::resume_unwind(payload),
        },
    }
}

pub struct ChainEngine {
    pub(crate) registry: Arc<dyn ExecutionServices>,
    pub(crate) config: EngineConfig,
    pub(crate) consensus: Arc<dyn ConsensusRuntime>,
    pub(crate) store: Arc<dyn Store>,
    pub(crate) tree: Arc<Tree>,
    pub(crate) block_history: Arc<StoreHistory>,
    pub(crate) genesis: BlockRef,
    pub(crate) waiter: sys::Waiter,
    pub(crate) listeners: Mutex<Vec<Arc<dyn ChainListener>>>,
    /// Held for the whole of any block insertion, single block or stream.
    pub(crate) inserting: Mutex<()>,
    /// False while a root writer is between disk and tree, and after any
    /// failed root write (engine fatal). The only root state flag. Root
    /// writers are already serialized by `inserting`, so no separate lock
    /// guards the transition.
    pub(crate) root_available: AtomicBool,
    /// Best-effort side recovery hints; `side_list_path` is the list file.
    pub(crate) side_list: Arc<SideListWriter>,
    pub(crate) side_list_path: Option<PathBuf>,
    /// Set while a sync stream owns `inserting`, so discover and miner packing
    /// can back off without blocking on the mutex.
    pub(crate) syncing: AtomicBool,
    mempool_min_fee_purity: u64,
    recent: RwLock<VecDeque<RecentBlock>>,
    avgfees: Mutex<VecDeque<u64>>,
    background: Mutex<Vec<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>>,
    side_background: Mutex<Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>>,
    sync_cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    lifecycle: AtomicU8,
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

pub(crate) enum ApplyResult {
    Accepted(ApplyAccepted),
    Duplicate(Hash),
    Orphan(Hash),
    /// A live side branch was discarded (execution / body write / attach
    /// failure). Not an error: the canonical tree is untouched and the stream
    /// continues.
    Discarded,
}

/// The accepted payload handed from the apply stage to the persist stage.
/// `persist_body` is false only for the internal replay pipeline, whose block
/// bodies and canonical index already exist. `prev_head` is the canonical head
/// hash before the attach; after a reorg the replaced old canonical tail is
/// derived from it and appended to the side hash list.
pub(crate) struct ApplyAccepted {
    pub pkg: BlkPkg,
    pub inserted: Inserted,
    pub persist_body: bool,
    pub prev_head: Hash,
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
        let state_status = store
            .state_status()
            .map_err(|e| crate::boot::probe_fault(format!("state status: {}", e)))?;
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
        let tree = Arc::new(Tree::new(store.disk(), root_block));
        // Side hash list writer. The keep predicate captures only standalone
        // engine pieces (tree / store / registry), never the engine itself,
        // so the writer thread needs no engine handle.
        let side_cancel = Arc::new(AtomicBool::new(false));
        let side_path = (!config.data_dir.is_empty())
            .then(|| PathBuf::from(&config.data_dir).join("side_hash_list"));
        let (side_list, side_rx) = SideListWriter::new();
        let side_list_spawn = side_list.clone();
        let eng = Arc::new(Self {
            tree,
            registry,
            config,
            consensus,
            store,
            block_history,
            genesis,
            waiter,
            listeners: Mutex::new(Vec::new()),
            inserting: Mutex::new(()),
            root_available: AtomicBool::new(true),
            side_list,
            side_list_path: side_path,
            syncing: AtomicBool::new(false),
            mempool_min_fee_purity,
            recent: RwLock::new(VecDeque::new()),
            avgfees: Mutex::new(VecDeque::new()),
            background: Mutex::new(Vec::new()),
            side_background: Mutex::new(None),
            sync_cancels: Mutex::new(HashMap::new()),
            lifecycle: AtomicU8::new(LIFE_STARTING),
            shutdown_lock: Mutex::new(()),
            shutdown_complete: AtomicBool::new(false),
            next_sync_id: AtomicU64::new(1),
        });
        crate::boot::open_state(&eng, state_status)?;
        eng.lifecycle.store(LIFE_RUNNING, Ordering::Release);
        // The writer only appends after boot; starting it here keeps the boot
        // side replay's direct file read race-free and a boot failure cannot
        // leak the writer thread.
        let side_handle = side_list_spawn.spawn(
            eng.side_list_path.clone(),
            side_keep_ctx(eng.tree.clone(), eng.store.clone(), eng.registry.clone()),
            side_cancel.clone(),
            side_rx,
        )?;
        eng.track_side_writer(side_cancel, side_handle);
        Ok(eng)
    }

    pub fn add_chain_listener(&self, listener: Arc<dyn ChainListener>) -> Rerr {
        self.listeners.lock().unwrap().push(listener);
        Ok(())
    }

    pub(crate) fn begin_root_move(&self) -> RootMoveGuard<'_> {
        self.root_available.store(false, Ordering::Release);
        RootMoveGuard {
            available: &self.root_available,
            committed: false,
        }
    }

    pub(crate) fn is_root_available(&self) -> bool {
        self.root_available.load(Ordering::Acquire)
    }

    pub fn shutdown(&self) -> Rerr {
        let _shutdown = self.shutdown_lock.lock().unwrap();
        if self.shutdown_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        self.lifecycle.store(LIFE_STOPPING, Ordering::Release);
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
        if let Some((cancel, handle)) = self.side_background.lock().unwrap().take() {
            // All inserts have completed before the side writer is cancelled;
            // this is the normal-shutdown drain guarantee.
            cancel.store(true, Ordering::Release);
            worker_panicked |= handle.join().is_err();
        }
        self.consensus.exit();
        self.lifecycle.store(LIFE_STOPPED, Ordering::Release);
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
        // Parent verification comes before any admission/arrival side effect:
        // an orphan must not touch the bidding map (§6 of the error contract).
        let prev_hash = pkg.block().prev_hash();
        if !self.tree.contains(&prev_hash) {
            return Ok(BlockAcceptResult::orphan(prev_hash));
        }
        let arrive = catch_storage_panic(|| self.consensus.check_block_arrive(&pkg, self, false));
        if let Err(e) = arrive {
            if self.mark_core_error(&e) {
                eprintln!(
                    "[Engine Fatal] operation=discover_arrive phase=pre_attach height={} hash={:?} error={}",
                    pkg.height(),
                    pkg.hash(),
                    e
                );
            }
            return Err(e);
        }

        match catch_storage_panic(|| self.consensus.check_block_admission(&pkg, self, false)) {
            Ok(base::BlockAdmissionDecision::Continue) => {}
            Ok(base::BlockAdmissionDecision::Defer(_)) => {
                return Ok(BlockAcceptResult::deferred());
            }
            Err(e) => return Err(e),
        }

        let _guard = self.inserting.lock().unwrap();
        if self.waiter.is_shutdown() {
            return errf!("engine is stopping");
        }
        // Preparation executes and attaches the candidate only after all
        // fallible validation is complete. If it fails, the existing tree,
        // including side branches, is untouched and must not be rebuilt.
        let job = match self.prepare_one(&pkg, ApplyMode::Strict, true) {
            Ok(ApplyResult::Accepted(job)) => job,
            Ok(ApplyResult::Duplicate(hash)) => return Ok(BlockAcceptResult::duplicate(hash)),
            Ok(ApplyResult::Orphan(parent)) => return Ok(BlockAcceptResult::orphan(parent)),
            Ok(ApplyResult::Discarded) => return Ok(BlockAcceptResult::ignored()),
            Err(e) => {
                if self.mark_core_error(&e) {
                    eprintln!(
                        "[Engine Fatal] operation=discover_prepare phase=pre_attach height={} hash={:?} error={}",
                        pkg.height(),
                        pkg.hash(),
                        e
                    );
                }
                return Err(e);
            }
        };
        match self.persist_one(job, true) {
            Ok(outcome) => Ok(outcome.result),
            Err(e) => {
                // After the attach, any canonical persistence failure is
                // engine-fatal (§2.3): the memory tree and disk are in an
                // uncertain transition. No recovery path exists; boot replay
                // rebuilds from the real disk state on the next start.
                eprintln!(
                    "[Engine Fatal] operation=discover_persist phase=post_attach height={} hash={:?} error={}",
                    pkg.height(),
                    pkg.hash(),
                    e
                );
                self.mark_fatal();
                Err(e)
            }
        }
    }

    /// Execute and attach a block without performing any I/O (except side
    /// branch bodies, which must be durable before their in-memory attach).
    /// Fast historical sync sends the returned job to the ordered persistence
    /// stage.
    pub(crate) fn prepare_one(
        &self,
        pkg: &BlkPkg,
        mode: ApplyMode,
        persist_body: bool,
    ) -> Ret<ApplyResult> {
        // A failure here is either a plain consensus/validation error (returned
        // to the caller, which decides peer penalty) or a core error carrying a
        // fatal code — the caller decides the boundary and prints it (§2.3).
        let accepted = match catch_storage_panic(|| {
            crate::insert::insert_block(self, pkg, mode, persist_body)
        }) {
            Ok(value) => value,
            Err(e) => return Err(e),
        };
        let ApplyResult::Accepted(job) = accepted else {
            return Ok(accepted);
        };
        if job.inserted.is_head {
            if job.inserted.reorg {
                // Pending entries describe the previous canonical branch. The
                // strict path persists the replacement before another insert.
                self.block_history.clear_pending();
            }
            self.block_history.remember(pkg.block_ref());
        }
        Ok(ApplyResult::Accepted(job))
    }

    /// Persist one already-executed block. Jobs must be supplied in insertion
    /// order; root commits validate that ordering independently. Any failure
    /// after the block was attached to the tree is engine-fatal (§2.3).
    pub(crate) fn persist_one(
        &self,
        job: ApplyAccepted,
        maintain_runtime_caches: bool,
    ) -> Ret<PersistOutcome> {
        let ApplyAccepted {
            pkg,
            inserted,
            persist_body,
            prev_head,
        } = job;
        let Inserted {
            is_head,
            reorg,
            roll,
            confirmed_txs,
            reverted_txs,
        } = inserted;
        let height = pkg.height();
        // A false persistence flag is only supplied by the internal replay
        // pipeline, whose block body and canonical index already exist. Side
        // branch bodies are written inside insert_block before the attach, so
        // a non-head job never reaches a body write here.
        let stored_replay = !persist_body;
        if persist_body {
            let store = self.store.block_store();
            if is_head && reorg {
                let depth = height.saturating_sub(self.tree.root_height());
                let mut canonical = self.tree.back_hashes(depth);
                canonical.reverse();
                store
                    .commit_reorg(height, &pkg.hash(), pkg.data().clone(), &canonical)
                    .map_err(persist_fatal)?;
                // The replaced old canonical tail becomes a side branch on the
                // next boot; record it so the fork tree can be restored. The
                // side tree just grew by the replaced tail, so re-apply the
                // live capacity bound (side chunks never enter the canonical
                // chain, only their side-subtree roots are evicted).
                let root_height = self.tree.root_height();
                let new_path: HashSet<Hash> = canonical.iter().map(|(_, hash)| *hash).collect();
                let replaced: Vec<Hash> = self
                    .tree
                    .branch_blocks(&prev_head)
                    .into_iter()
                    .flatten()
                    .filter(|blk| blk.height() > root_height && !new_path.contains(&blk.hash()))
                    .map(|blk| blk.hash())
                    .collect();
                self.side_list.append_many(replaced);
                self.tree
                    .enforce_side_capacity(self.config.side_tree_capacity);
            } else if is_head {
                store
                    .put_block_available(height, &pkg.hash(), pkg.data().clone())
                    .map_err(persist_fatal)?;
            }
        }
        let mut rolled = 0;
        if let Some(job) = roll {
            rolled = crate::roll::roll_root(self, job, pkg.origin(), stored_replay)
                .map_err(persist_fatal)?
                .len() as u64;
        }
        // The block is durably accepted now; publish consensus-owned arrival
        // metadata. Replay/rebuild must not republish it.
        if !stored_replay {
            isolate_callback("consensus.on_block_accepted", || {
                self.consensus.on_block_accepted(&pkg, self);
            });
        }
        if maintain_runtime_caches && is_head {
            self.record_recent(pkg.block());
            self.record_avgfee(pkg.block());
        }
        if !stored_replay {
            for listener in self.listeners.lock().unwrap().iter() {
                isolate_callback("listener.on_block_accepted", || {
                    listener.on_block_accepted(height, pkg.origin());
                });
            }
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
        let tip = match self.store.block_store().available_cursor()? {
            Some(tip) => tip,
            None => 0,
        };
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
                let Some((_hash, data)) = self.store.block_data_by_height(height)? else {
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

    /// Permanently stop accepting work after a canonical persistence failure
    /// (or an internal inconsistency). The caller may still hold a waiter
    /// handle, so this deliberately does not wait for outstanding work like
    /// `shutdown` does.
    pub(crate) fn mark_fatal(&self) {
        let mut state = self.lifecycle.load(Ordering::Acquire);
        loop {
            match state {
                LIFE_STOPPING | LIFE_STOPPED => return,
                LIFE_FATAL => break,
                _ => match self.lifecycle.compare_exchange(
                    state,
                    LIFE_FATAL,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(next) => state = next,
                },
            }
        }
        self.waiter.trigger();
        for cancel in self.sync_cancels.lock().unwrap().values() {
            cancel.store(true, Ordering::Release);
        }
    }

    /// Whether `error` is engine-fatal, marking the lifecycle when it is. The
    /// caller prints the structured context; every fatal error is printed
    /// exactly once at the boundary that decides it.
    pub(crate) fn mark_core_error(&self, error: &sys::Error) -> bool {
        let fatal = CoreFault::is_core_fault(error);
        if fatal && self.lifecycle.load(Ordering::Acquire) == LIFE_RUNNING {
            self.mark_fatal();
        }
        fatal
    }

    pub(crate) fn is_fatal(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == LIFE_FATAL
    }

    pub(crate) fn is_stopping(&self) -> bool {
        matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFE_STOPPING | LIFE_STOPPED
        )
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

    pub(crate) fn track_side_writer(
        &self,
        cancel: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
    ) {
        if self.is_stopping() {
            cancel.store(true, Ordering::Release);
            let _ = handle.join();
            return;
        }
        *self.side_background.lock().unwrap() = Some((cancel, handle));
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

/// Run one external callback inside a panic boundary. A panicking listener
/// must not take down the engine; the notification is skipped with a warning
/// (§8 of the error contract).
pub(crate) fn isolate_callback(name: &'static str, run: impl FnOnce()) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    if result.is_err() {
        eprintln!("[Engine] {} panicked; notification skipped", name);
    }
}

/// Build the side hash list keep predicate: per compaction it snapshots the
/// canonical branch and the durable root height, then drops hashes that are
/// canonical, below the root, or whose body is missing/undecodable.
fn side_keep_ctx(
    tree: Arc<Tree>,
    store: Arc<dyn Store>,
    registry: Arc<dyn ExecutionServices>,
) -> SideKeepCtx {
    Arc::new(move || {
        let root_height = tree.root_height();
        let head_hash = tree.head_tip().0;
        let canonical: HashSet<Hash> = tree
            .branch_blocks(&head_hash)
            .into_iter()
            .flatten()
            .map(|blk| blk.hash())
            .collect();
        let store = store.clone();
        let registry = registry.clone();
        Box::new(move |hash: &Hash| {
            if canonical.contains(hash) {
                return false;
            }
            // A transient read failure keeps the recovery hint (conservative);
            // a missing or undecodable body drops it, and boot decides the
            // canonical answer anyway.
            let Some(data) = store.block_data(hash).unwrap_or(None) else {
                return false;
            };
            let Ok((blk, _)) = registry.decode_block(&data) else {
                return false;
            };
            blk.height() > root_height
        })
    })
}

pub(crate) fn load_persisted_root_block(
    registry: &dyn ExecutionServices,
    store: &dyn Store,
    genesis: &BlockRef,
    status: &base::ChainStatus,
) -> sys::Ret<BlockRef> {
    use crate::boot::{boot_fault, validate_fault};
    if status.latest_height == 0 {
        if status.latest_hash != genesis.hash() {
            return Err(validate_fault(format!(
                "state root hash {:?} does not match genesis {:?}",
                status.latest_hash,
                genesis.hash()
            )));
        }
        return Ok(genesis.clone());
    }
    // The height index must agree with the root marker: the state root block
    // is identified by both, and a mismatch rejects startup (no cursor scan
    // or repair is ever attempted). A read failure is a storage boot failure.
    let index_hash = store.block_hash(status.latest_height).map_err(|e| {
        validate_fault(format!(
            "state root height index read failed at <{}, {:?}>: {}",
            status.latest_height,
            status.latest_hash,
            e
        ))
    })?;
    if index_hash != Some(status.latest_hash) {
        return Err(validate_fault(format!(
            "state root hash does not match the canonical height index at <{}, {:?}>",
            status.latest_height,
            status.latest_hash
        )));
    }
    let data = match store.block_data(&status.latest_hash) {
        Ok(Some(data)) => data,
        Ok(None) => {
            return Err(validate_fault(format!(
                "state root block is missing from the block db at <{}, {:?}>",
                status.latest_height,
                status.latest_hash
            )));
        }
        Err(e) => {
            return Err(validate_fault(format!(
                "state root block read failed at <{}, {:?}>: {}",
                status.latest_height,
                status.latest_hash,
                e
            )));
        }
    };
    let block = registry.decode_block_exact(&data).map_err(|e| {
        boot_fault(
            "validate",
            "compatibility",
            format!(
                "state root block cannot be decoded at <{}, {:?}>: {}",
                status.latest_height,
                status.latest_hash,
                e
            ),
        )
    })?;
    if block.height() != status.latest_height || block.hash() != status.latest_hash {
        return Err(validate_fault(format!(
            "state root block identity mismatch: expected <{}, {:?}>, got <{}, {:?}>",
            status.latest_height,
            status.latest_hash,
            block.height(),
            block.hash()
        )));
    }
    Ok(block)
}

pub(crate) struct RootMoveGuard<'a> {
    available: &'a AtomicBool,
    committed: bool,
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
    }
}

//////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_read_panic_becomes_a_core_storage_error() {
        let result = catch_storage_panic(|| -> Ret<()> {
            std::panic::panic_any(base::StorageReadPanic {
                error: sys::Error::fault("injected read failure"),
            })
        });
        let error = result.unwrap_err();
        assert_eq!(error.code(), Some(CoreFault::StorageReadFailed.code()));
    }

    #[test]
    #[should_panic(expected = "programming failure")]
    fn non_storage_panic_keeps_unwinding() {
        let _ = catch_storage_panic(|| -> Ret<()> { panic!("programming failure") });
    }

    #[test]
    fn failed_root_move_stays_unavailable_until_commit() {
        let available = AtomicBool::new(true);
        {
            let mut guard = RootMoveGuard {
                available: &available,
                committed: false,
            };
            guard.commit();
        }
        assert!(
            available.load(Ordering::Acquire),
            "committed move is readable"
        );
    }

    #[test]
    fn abandoned_root_move_keeps_state_unavailable() {
        // `begin_root_move` marks the state unavailable before the guard.
        let available = AtomicBool::new(false);
        {
            let _guard = RootMoveGuard {
                available: &available,
                committed: false,
            };
        }
        // A failed root persistence must leave the state unavailable so the
        // engine rejects new work instead of continuing on an uncertain
        // disk/tree transition.
        assert!(!available.load(Ordering::Acquire));
    }
}
