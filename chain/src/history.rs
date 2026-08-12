//! `BlockHistory` over the block store, used by consensus for difficulty and
//! bidding lookups.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base::{BlockHistory, BlockRef, ExecutionServices, Store};
use field::Hash;

pub struct StoreHistory {
    store: Arc<dyn Store>,
    registry: Arc<dyn ExecutionServices>,
    genesis: BlockRef,
    /// Decoded canonical blocks attached to the tree but not stable yet. This
    /// bridges the bounded interval where execution is ahead of block storage.
    pending: Mutex<BTreeMap<u64, (Hash, BlockRef)>>,
}

pub(crate) struct BranchHistory<'a> {
    canonical: &'a StoreHistory,
    branch: BTreeMap<u64, BlockRef>,
}

impl StoreHistory {
    pub fn new(
        store: Arc<dyn Store>,
        registry: Arc<dyn ExecutionServices>,
        genesis: BlockRef,
    ) -> Self {
        Self {
            store,
            registry,
            genesis,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn remember(&self, block: BlockRef) {
        self.pending
            .lock()
            .unwrap()
            .insert(block.height(), (block.hash(), block));
    }

    pub fn cached(&self, height: u64, hash: &Hash) -> Option<BlockRef> {
        self.pending
            .lock()
            .unwrap()
            .get(&height)
            .filter(|(cached_hash, _)| cached_hash == hash)
            .map(|(_, block)| block.clone())
    }

    pub fn forget(&self, height: u64, hash: &Hash) {
        let mut pending = self.pending.lock().unwrap();
        if pending
            .get(&height)
            .is_some_and(|(cached_hash, _)| cached_hash == hash)
        {
            pending.remove(&height);
        }
    }

    pub fn clear_pending(&self) {
        self.pending.lock().unwrap().clear();
    }

    /// Overlay the candidate parent's in-memory ancestry on the durable
    /// canonical history. Consensus difficulty checks must follow the branch
    /// being extended, not whichever branch currently owns a height index.
    pub(crate) fn for_branch(&self, blocks: Vec<BlockRef>) -> BranchHistory<'_> {
        BranchHistory {
            canonical: self,
            branch: blocks
                .into_iter()
                .map(|block| (block.height(), block))
                .collect(),
        }
    }
}

impl BlockHistory for StoreHistory {
    fn stable_height(&self) -> u64 {
        match self.store.status() {
            Ok(status) => status.latest_height,
            Err(error) => std::panic::panic_any(base::StorageReadPanic { error }),
        }
    }

    fn block_at_height(&self, height: u64) -> sys::Ret<Option<BlockRef>> {
        if height == 0 {
            return Ok(Some(self.genesis.clone()));
        }
        if let Some((_, block)) = self.pending.lock().unwrap().get(&height) {
            return Ok(Some(block.clone()));
        }
        let Some((stored_hash, data)) = self
            .store
            .block_data_by_height(height)
            .map_err(|error| {
                error.with_code(crate::engine::CoreFault::StorageReadFailed.code())
            })?
        else {
            return Ok(None);
        };
        let block = self.registry.decode_block_exact(&data).map_err(|e| {
            sys::Error::fault(format!("stored block {} cannot be decoded: {}", height, e))
                .with_code(crate::engine::CoreFault::StorageReadFailed.code())
        })?;
        if block.height() != height || block.hash() != stored_hash {
            return Err(sys::Error::fault(format!(
                "stored block identity mismatch at height {}: index {:?}, decoded <{}, {:?}>",
                height,
                stored_hash,
                block.height(),
                block.hash()
            ))
            .with_code(crate::engine::CoreFault::StorageReadFailed.code()));
        }
        Ok(Some(block))
    }
}

impl BlockHistory for BranchHistory<'_> {
    fn stable_height(&self) -> u64 {
        self.canonical.stable_height()
    }

    fn block_at_height(&self, height: u64) -> sys::Ret<Option<BlockRef>> {
        if let Some(block) = self.branch.get(&height) {
            return Ok(Some(block.clone()));
        }
        self.canonical.block_at_height(height)
    }
}
