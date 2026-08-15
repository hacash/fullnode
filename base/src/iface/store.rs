//! Disk / block store / chain store facade.
//!
//! `BlockStore` / `DiskDB` / persist keys / `Store`

use std::collections::HashMap;
use std::sync::Arc;

use field::Hash;
use sys::Rerr;

use crate::state::{LogBackend, StateRead};
use crate::store::ChainStatus;

// =============================================================
// BlockStore
// =============================================================
//
// `DiskDB`state KVblock  state
// rebuild  block
//
// ****`put_block`  fsync  apply
// root marker canonical  root roll finalize
// block bytes  root marker  /
// canonical

pub trait BlockStore: Send + Sync {
    /// fsync
    ///
    /// BlockStore  forkgenesis
    /// stable root block  hash
    fn put_block(&self, height: u64, hash: &Hash, data: sys::Bytes) -> Rerr;

    /// Persist a block body and make it the next available canonical block.
    /// KV-backed stores override this to commit the body, height index and
    /// available cursor in one database batch.
    fn put_block_available(&self, height: u64, hash: &Hash, data: sys::Bytes) -> Rerr;

    /// Atomically persist the new reorg tip body, rewrite the supplied
    /// canonical height range, remove any canonical tail above `height`, and
    /// publish the available cursor. `canonical` is ascending and ends at the
    /// supplied `(height, hash)`.
    fn commit_reorg(
        &self,
        height: u64,
        hash: &Hash,
        data: sys::Bytes,
        canonical: &[(u64, Hash)],
    ) -> Rerr;

    /// canonical root roll finalize rebuild / open
    /// `height == 0` genesis
    ///
    /// `Ok(None)` is the only not-found answer. Read and decode failures are
    /// returned as errors: a corrupt stored block must never masquerade as a
    /// missing one on consensus or boot paths (§3.1 of the engine error
    /// contract).
    fn read_by_hash(&self, hash: &Hash) -> sys::Ret<Option<sys::Bytes>>;
    fn read_by_height(&self, height: u64) -> sys::Ret<Option<(Hash, sys::Bytes)>>;

    /// The canonical height index hash without the body read. Replay uses it
    /// to verify the decoded body identity without a second body fetch.
    fn hash_by_height(&self, height: u64) -> sys::Ret<Option<Hash>> {
        self.read_by_height(height)
            .map(|found| found.map(|(hash, _)| hash))
    }

    fn available_cursor(&self) -> sys::Ret<Option<u64>>;

    /// Whether the store holds any block records at all (bodies or height
    /// index). Boot uses this to distinguish a fresh store from a corrupted
    /// one whose available cursor is missing.
    fn has_records(&self) -> sys::Ret<bool>;
}

// =============================================================
// KV DB
// =============================================================

pub type MemMap = HashMap<Vec<u8>, Option<Vec<u8>>>;

pub trait MemDB: Send + Sync {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn for_each(&self, each: &mut dyn FnMut(&[u8], Option<&[u8]>));
}

#[derive(Default, Clone, Debug)]
pub struct MemKV {
    pub memry: MemMap,
}

impl MemKV {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.memry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.memry.is_empty()
    }

    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.memry.insert(key, Some(value));
    }

    pub fn del(&mut self, key: Vec<u8>) {
        self.memry.insert(key, None);
    }

    pub fn get(&self, key: &[u8]) -> Option<&Option<Vec<u8>>> {
        self.memry.get(key)
    }
}

impl MemDB for MemKV {
    fn len(&self) -> usize {
        self.memry.len()
    }

    fn for_each(&self, each: &mut dyn FnMut(&[u8], Option<&[u8]>)) {
        for (key, value) in &self.memry {
            each(key, value.as_deref());
        }
    }
}

pub trait DiskDB: Send + Sync {
    /// Read one key. Backends report I/O or corruption failures as errors:
    /// they must never be reported as a missing key, which would silently
    /// diverge state. `Ok(None)` is the only missing-key answer.
    fn read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>>;
    fn save(&self, key: &[u8], val: &[u8]);
    fn remove(&self, key: &[u8]);
    /// Recoverable boundary for consensus-critical persistence. Every backend
    /// must provide a native atomic batch implementation.
    fn try_write(&self, memkv: &dyn MemDB) -> Rerr;
    /// Read through the recoverable boundary. `Ok(None)` remains the only
    /// missing-key answer and must never be synthesized by callers.
    fn try_read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        self.read(key)
    }
    fn for_each(&self, _f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        Ok(())
    }
    /// Clear this database. Backends with a native truncate/drop primitive can
    /// override this; the default preserves the existing batch-delete logic.
    fn clear(&self) -> Rerr {
        let mut batch = MemKV::new();
        self.for_each(&mut |key, _| batch.del(key.to_vec()))?;
        if batch.is_empty() {
            return Ok(());
        }
        self.try_write(&batch)
    }
}

// =============================================================
// move_root
// =============================================================

/// stable root  hash `move_root`  KV  state
/// state batch write
pub const PERSIST_KEY_ROOT_HASH: &[u8] = b"_chain.root_hash";
/// stable root
pub const PERSIST_KEY_ROOT_HEIGHT: &[u8] = b"_chain.root_height";

/// Lifecycle state of the persisted canonical state.
///
/// A fresh state store is empty and has no root markers. A ready store has
/// both markers decoded successfully. Implementations must reject partial,
/// malformed, or rootless nonempty state rather than treating it as fresh.
#[derive(Clone, Debug)]
pub enum StateStatus {
    Uninitialized,
    Ready(ChainStatus),
}

// =============================================================
// Store
// =============================================================

pub trait Store: Send + Sync {
    /// The durable state root status. Read errors are returned, never
    /// silently replaced by a default status.
    fn status(&self) -> sys::Ret<ChainStatus>;

    /// Distinguish a fresh state store from a valid genesis state at height
    /// zero. Engine startup owns the resulting initialize/rebuild/replay
    /// decision.
    fn state_status(&self) -> sys::Ret<StateStatus>;

    fn state_get(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>>;
    fn stable_state(&self) -> Arc<dyn StateRead>;

    /// **State** disk — `move_root` / tree root KV (not the block DB).
    ///
    /// `store.disk()` is the state database. Block bytes live in `block_store()`;
    /// VM logs in `log_backend()`.
    fn disk(&self) -> Arc<dyn DiskDB>;

    /// ——  state KV
    /// apply state diff  root marker
    /// move-root `DiskDB::write` rebuild
    fn block_store(&self) -> Arc<dyn BlockStore>;

    fn log_backend(&self) -> Arc<dyn LogBackend>;

    /// state/root  block bytes
    /// stable state  state
    ///
    /// state KV  BlockStore  state
    /// column family /  KV  block
    fn clear_state_keep_blocks(&self) -> Rerr {
        sys::errf!("store does not support clear_state_keep_blocks")
    }

    /// Block body by hash; `Ok(None)` is the only not-found answer.
    fn block_data(&self, hash: &Hash) -> sys::Ret<Option<sys::Bytes>> {
        self.block_store().read_by_hash(hash)
    }
    fn block_hash(&self, height: u64) -> sys::Ret<Option<Hash>> {
        self.block_store()
            .read_by_height(height)
            .map(|found| found.map(|(h, _)| h))
    }
    fn block_data_by_height(&self, height: u64) -> sys::Ret<Option<(Hash, sys::Bytes)>> {
        self.block_store().read_by_height(height)
    }
}
