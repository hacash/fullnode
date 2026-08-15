//! `BlockHistory` over the block store, used by consensus for difficulty and
//! bidding lookups.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base::{
    BlockHistory, BlockRef, ExecutionServices, STATE_DECODE_FAILED_CODE, STATE_READ_FAILED_CODE,
    Store,
};
use field::Hash;
use sys::Ret;

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
    /// The durable canonical root height. `Store::status` read failures are
    /// canonical acquisitions (§6.8): classified as `Abort + storage_read_failed`
    /// so consensus callers treat them as fatal, never as a zero root.
    fn stable_height(&self) -> Ret<u64> {
        match self.store.status() {
            Ok(status) => Ok(status.latest_height),
            Err(error) => Err(sys::Error::abort(format!(
                "canonical root status read failed: {}",
                error
            ))
            .with_code(STATE_READ_FAILED_CODE)),
        }
    }

    fn block_at_height(&self, height: u64) -> sys::Ret<Option<BlockRef>> {
        if height == 0 {
            return Ok(Some(self.genesis.clone()));
        }
        if let Some((_, block)) = self.pending.lock().unwrap().get(&height) {
            return Ok(Some(block.clone()));
        }
        let Some((stored_hash, data)) =
            self.store.block_data_by_height(height).map_err(|error| {
                sys::Error::abort(format!("canonical block read failed: {}", error))
                    .with_code(STATE_READ_FAILED_CODE)
            })?
        else {
            return Ok(None);
        };
        let block = self.registry.decode_block_exact(&data).map_err(|e| {
            sys::Error::abort(format!(
                "stored canonical block {} cannot be decoded: {}",
                height, e
            ))
            .with_code(STATE_DECODE_FAILED_CODE)
        })?;
        if block.height() != height || block.hash() != stored_hash {
            return Err(sys::Error::abort(format!(
                "stored block identity mismatch at height {}: index {:?}, decoded <{}, {:?}>",
                height,
                stored_hash,
                block.height(),
                block.hash()
            ))
            .with_code(STATE_DECODE_FAILED_CODE));
        }
        Ok(Some(block))
    }
}

impl BlockHistory for BranchHistory<'_> {
    fn stable_height(&self) -> Ret<u64> {
        self.canonical.stable_height()
    }

    fn block_at_height(&self, height: u64) -> sys::Ret<Option<BlockRef>> {
        if let Some(block) = self.branch.get(&height) {
            return Ok(Some(block.clone()));
        }
        self.canonical.block_at_height(height)
    }
}
