//! Triple-directory chain store: `block/` + app-selected `state/` + `vmlog/`.
//!
//! - **block**: content-addressed bodies + available height index (kept across rebuild)
//! - **state**: business KV + root markers (wipe to rebuild locally)
//! - **vmlog**: optional VM logs (cleared with state rebuild)
//!
//! `Store::disk()` returns the **state** DB (move_root WriteBatch target).
//! Block-body storage lives in [`crate::block_store`]; VM logs in [`crate::log_backend`].

//! # Persistent key map
//!
//! ## state DB
//! | Key pattern | Writer | Meaning |
//! |---|---|---|
//! | `_chain.root_hash` | `build_move_root_batch` / `reset_state_to_genesis` | stable root hash (32 bytes) |
//! | `_chain.root_height` | same | stable root height (be u64) |
//! | `_consensus.diamond_form` | mint genesis initialization | immutable diamond ownership form (one byte, `0`/`1`) |
//! | *(business KV)* | `move_root` batch write | state tree flattened deltas |
//!
//! Business namespaces are one binary byte beginning at `0x01`; `0x00` is
//! reserved and `0x5f` is skipped because it is the ASCII `_` text-key prefix.
//!
//! ## block DB (`block/`)
//! | Key pattern | Writer | Meaning |
//! |---|---|---|
//! | `0x01 || hash` | `BlockStore::put_block` / `put_block_available` | raw block body (content-addressed) |
//! | `0x02 || height_be_u64` | `BlockStore::put_block_available` / `commit_reorg` | recoverable head height -> hash |
//! | `_block.cursor` | same | highest contiguous available height |
//!
//! ## vmlog DB (`vmlog/`)
//! | Key pattern | Writer | Meaning |
//! |---|---|---|
//! | `0x01 || height_be_u64` | `LogBackend::append_block_logs` | log count for block (be u64) |
//! | `0x02 || height_be_u64 || index_be_u64` | same | individual log entry (encoded) |

use std::sync::Arc;

use base::{
    ChainStatus, DiskDB, LogBackend, PERSIST_KEY_ROOT_HASH, PERSIST_KEY_ROOT_HEIGHT, StateRead,
    StateStatus, Store,
};
use field::Hash;
use sys::Ret;

use crate::block_store::KvBlockStore;
use crate::log_backend::KvLogBackend;

/// Read the first 8 bytes as a big-endian `u64`.
pub(crate) fn read_u64_be_prefix(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    Some(u64::from_be_bytes(buf))
}

/// Triple-DB chain store (`block` / `state` / `vmlog`).
pub struct StoreInst {
    block: Arc<dyn DiskDB>,
    state: Arc<dyn DiskDB>,
    log: Arc<dyn DiskDB>,
    block_store: Arc<KvBlockStore>,
    log_backend: Arc<KvLogBackend>,
}

impl StoreInst {
    /// Three independent in-memory DBs (tests / no data_dir).
    pub fn new() -> Self {
        let block = crate::mem::MemDiskDB::new() as Arc<dyn DiskDB>;
        let state = crate::mem::MemDiskDB::new() as Arc<dyn DiskDB>;
        let log = crate::mem::MemDiskDB::new() as Arc<dyn DiskDB>;
        Self::from_disks(block, state, log).expect("in-memory store")
    }

    /// Open the three directories selected by the application composition root.
    pub fn open(
        block_dir: &std::path::Path,
        state_dir: &std::path::Path,
        log_dir: &std::path::Path,
    ) -> Ret<Self> {
        let block = Arc::new(crate::DiskKV::open(block_dir)?) as Arc<dyn DiskDB>;
        let state = Arc::new(crate::DiskKV::open(state_dir)?) as Arc<dyn DiskDB>;
        let log = Arc::new(crate::DiskKV::open(log_dir)?) as Arc<dyn DiskDB>;
        Self::from_disks(block, state, log)
    }

    pub fn from_disks(
        block: Arc<dyn DiskDB>,
        state: Arc<dyn DiskDB>,
        log: Arc<dyn DiskDB>,
    ) -> sys::Ret<Self> {
        Ok(Self {
            block_store: Arc::new(KvBlockStore::new(block.clone())?),
            log_backend: Arc::new(KvLogBackend::new(log.clone())),
            block,
            state,
            log,
        })
    }

    pub fn block_disk(&self) -> Arc<dyn DiskDB> {
        self.block.clone()
    }

    pub fn log_disk(&self) -> Arc<dyn DiskDB> {
        self.log.clone()
    }
}

impl Store for StoreInst {
    fn status(&self) -> Ret<ChainStatus> {
        match self.state_status()? {
            StateStatus::Ready(status) => Ok(status),
            StateStatus::Uninitialized => Ok(ChainStatus::default()),
        }
    }

    fn state_status(&self) -> Ret<StateStatus> {
        let hash = self.state.try_read(PERSIST_KEY_ROOT_HASH)?;
        let height = self.state.try_read(PERSIST_KEY_ROOT_HEIGHT)?;
        match (hash, height) {
            (None, None) => {
                let mut has_data = false;
                self.state.for_each(&mut |_, _| has_data = true)?;
                if has_data {
                    return sys::errf!("state contains data but has no root markers");
                }
                Ok(StateStatus::Uninitialized)
            }
            (Some(hash), Some(height)) if hash.len() == 32 && height.len() == 8 => {
                let mut hash_bytes = [0u8; 32];
                hash_bytes.copy_from_slice(&hash);
                let mut height_bytes = [0u8; 8];
                height_bytes.copy_from_slice(&height);
                let latest_height = u64::from_be_bytes(height_bytes);
                Ok(StateStatus::Ready(ChainStatus {
                    latest_height,
                    latest_hash: Hash::from(hash_bytes),
                    immature_height: latest_height,
                }))
            }
            (Some(_), Some(_)) => sys::errf!("state root markers are malformed"),
            _ => sys::errf!("state root markers are incomplete"),
        }
    }

    fn state_get(&self, key: &[u8]) -> Option<Vec<u8>> {
        base::read_or_panic(self.state.as_ref(), key)
    }

    fn stable_state(&self) -> Arc<dyn StateRead> {
        Arc::new(DiskStateRead {
            disk: self.state.clone(),
        })
    }

    /// State DB - move_root / tree root disk.
    fn disk(&self) -> Arc<dyn DiskDB> {
        self.state.clone()
    }

    fn block_store(&self) -> Arc<dyn base::BlockStore> {
        self.block_store.clone()
    }

    fn log_backend(&self) -> Arc<dyn LogBackend> {
        self.log_backend.clone()
    }
}

struct DiskStateRead {
    disk: Arc<dyn DiskDB>,
}

impl StateRead for DiskStateRead {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        base::read_or_panic(self.disk.as_ref(), key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_status_distinguishes_fresh_and_ready_state() {
        let store = StoreInst::new();
        assert!(matches!(
            store.state_status().unwrap(),
            StateStatus::Uninitialized
        ));

        store.disk().save(PERSIST_KEY_ROOT_HASH, &[7; 32]);
        store
            .disk()
            .save(PERSIST_KEY_ROOT_HEIGHT, &42u64.to_be_bytes());

        let StateStatus::Ready(status) = store.state_status().unwrap() else {
            panic!("expected ready state");
        };
        assert_eq!(status.latest_height, 42);
        assert_eq!(status.immature_height, 42);
        assert_eq!(status.latest_hash, Hash::from([7; 32]));
    }

    #[test]
    fn state_status_rejects_partial_root_markers() {
        let store = StoreInst::new();
        store
            .disk()
            .save(PERSIST_KEY_ROOT_HEIGHT, &0u64.to_be_bytes());
        assert!(store.state_status().is_err());
    }

    #[test]
    fn state_status_rejects_rootless_nonempty_state() {
        let store = StoreInst::new();
        store.disk().save(b"business-key", b"value");
        assert!(store.state_status().is_err());
    }
}
