//! KV-backed `BlockStore`: content-addressed block bodies + height index.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base::{BlockStore as BlockStoreTrait, DiskDB, MemKV};
use field::Hash;
use sys::{Bytes, Rerr};

// High-cardinality records use compact binary namespaces. The cursor is a
// singleton operational value, so it intentionally remains human-readable.
const KEY_PREFIX_BLOCK: &[u8] = &[0x01];
const KEY_PREFIX_BLOCK_INDEX: &[u8] = &[0x02];
const KEY_AVAILABLE_CURSOR: &[u8] = b"_block.cursor";

fn block_key(hash: &Hash) -> Vec<u8> {
    let mut v = Vec::with_capacity(KEY_PREFIX_BLOCK.len() + 32);
    v.extend_from_slice(KEY_PREFIX_BLOCK);
    v.extend_from_slice(&hash.0);
    v
}

fn height_key(height: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(KEY_PREFIX_BLOCK_INDEX.len() + 8);
    v.extend_from_slice(KEY_PREFIX_BLOCK_INDEX);
    v.extend_from_slice(&height.to_be_bytes());
    v
}

fn read_hash(disk: &dyn DiskDB, key: &[u8]) -> Option<Hash> {
    let bytes = disk.read(key)?;
    if bytes.len() < 32 {
        return None;
    }
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&bytes[..32]);
    Some(Hash::from(buf))
}

/// Slow recovery path for a missing or corrupt cursor.
fn scan_available_tip(block: &dyn DiskDB) -> u64 {
    let mut max_index = 0u64;
    let _ = block.for_each(&mut |k, _| {
        if k.starts_with(KEY_PREFIX_BLOCK_INDEX) && k.len() == KEY_PREFIX_BLOCK_INDEX.len() + 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&k[KEY_PREFIX_BLOCK_INDEX.len()..]);
            max_index = max_index.max(u64::from_be_bytes(buf));
        }
    });

    let mut tip = 0u64;
    for height in 1..=max_index {
        let Some(hash) = read_hash(block, &height_key(height)) else {
            break;
        };
        if block.read(&block_key(&hash)).is_none() {
            break;
        }
        tip = height;
    }
    tip
}

fn cursor_height(block: &dyn DiskDB) -> Option<u64> {
    let raw = block.read(KEY_AVAILABLE_CURSOR)?;
    if raw.len() != 8 {
        return None;
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&raw);
    let height = u64::from_be_bytes(bytes);
    if height == 0 {
        return None;
    }
    let hash = read_hash(block, &height_key(height))?;
    block.read(&block_key(&hash)).is_some().then_some(height)
}

fn restore_cursor(block: &dyn DiskDB) -> u64 {
    if let Some(height) = cursor_height(block) {
        return height;
    }
    let height = scan_available_tip(block);
    if height > 0 {
        let mut batch = MemKV::new();
        batch.put(KEY_AVAILABLE_CURSOR.to_vec(), height.to_be_bytes().to_vec());
        let _ = block.try_write(&batch);
    }
    height
}

pub(crate) struct KvBlockStore {
    block: Arc<dyn DiskDB>,
    tip: AtomicU64,
}

impl KvBlockStore {
    pub(crate) fn new(block: Arc<dyn DiskDB>) -> Self {
        let tip = restore_cursor(block.as_ref());
        Self {
            block,
            tip: AtomicU64::new(tip),
        }
    }
}

impl BlockStoreTrait for KvBlockStore {
    fn put_block(&self, height: u64, hash: &Hash, data: Bytes) -> Rerr {
        if height == 0 {
            return sys::errf!("genesis block must not be stored in BlockStore");
        }
        let mut batch = MemKV::new();
        batch.put(block_key(hash), data.into_vec());
        self.block.try_write(&batch)
    }

    fn put_block_available(&self, height: u64, hash: &Hash, data: Bytes) -> Rerr {
        if height == 0 {
            return sys::errf!("genesis block must not be stored in BlockStore");
        }
        let current = self.tip.load(Ordering::Acquire);
        if height != current.saturating_add(1) {
            return sys::errf!(
                "cannot append available block {} after cursor {}",
                height,
                current
            );
        }
        let mut batch = MemKV::new();
        batch.put(block_key(hash), data.into_vec());
        batch.put(height_key(height), hash.0.to_vec());
        batch.put(KEY_AVAILABLE_CURSOR.to_vec(), height.to_be_bytes().to_vec());
        self.block.try_write(&batch)?;
        self.tip.store(height, Ordering::Release);
        Ok(())
    }

    fn commit_reorg(
        &self,
        height: u64,
        hash: &Hash,
        data: Bytes,
        canonical: &[(u64, Hash)],
    ) -> Rerr {
        if height == 0 {
            return sys::errf!("genesis block must not be stored in BlockStore");
        }
        let Some(&(last_height, last_hash)) = canonical.last() else {
            return sys::errf!("canonical reorg path is empty");
        };
        if last_height != height || last_hash != *hash {
            return sys::errf!(
                "canonical reorg path ends at <{}, {:?}> instead of <{}, {:?}>",
                last_height,
                last_hash,
                height,
                hash
            );
        }
        for pair in canonical.windows(2) {
            if pair[0].0.checked_add(1) != Some(pair[1].0) {
                return sys::errf!(
                    "canonical reorg path is not contiguous at heights {} and {}",
                    pair[0].0,
                    pair[1].0
                );
            }
        }
        for &(canonical_height, canonical_hash) in canonical {
            if canonical_height == 0 {
                return sys::errf!("genesis block must not be indexed in BlockStore");
            }
            if canonical_hash != *hash && self.read_by_hash(&canonical_hash).is_none() {
                return sys::errf!(
                    "cannot index missing block {:?} at height {}",
                    canonical_hash,
                    canonical_height
                );
            }
        }

        let current = self.tip.load(Ordering::Acquire);
        let mut batch = MemKV::new();
        batch.put(block_key(hash), data.into_vec());
        for &(canonical_height, canonical_hash) in canonical {
            batch.put(height_key(canonical_height), canonical_hash.0.to_vec());
        }
        if height < current {
            for stale_height in height + 1..=current {
                batch.del(height_key(stale_height));
            }
        }
        batch.put(KEY_AVAILABLE_CURSOR.to_vec(), height.to_be_bytes().to_vec());
        self.block.try_write(&batch)?;
        self.tip.store(height, Ordering::Release);
        Ok(())
    }

    fn read_by_hash(&self, hash: &Hash) -> Option<Bytes> {
        self.block.read(&block_key(hash)).map(Bytes::from_vec)
    }

    fn read_by_height(&self, height: u64) -> Option<(Hash, Bytes)> {
        let hash = read_hash(self.block.as_ref(), &height_key(height))?;
        let data = self.read_by_hash(&hash)?;
        Some((hash, data))
    }

    fn available_cursor(&self) -> Option<u64> {
        let height = self.tip.load(Ordering::Acquire);
        (height > 0).then_some(height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::BlockStore;

    fn hash(value: u8) -> Hash {
        Hash::from([value; 32])
    }

    #[test]
    fn reorg_body_indexes_tail_and_cursor_commit_together() {
        let disk = crate::mem::MemDiskDB::new() as Arc<dyn DiskDB>;
        let store = KvBlockStore::new(disk);
        store
            .put_block_available(1, &hash(1), Bytes::from_vec(vec![1]))
            .unwrap();
        store
            .put_block_available(2, &hash(2), Bytes::from_vec(vec![2]))
            .unwrap();
        store
            .put_block(1, &hash(11), Bytes::from_vec(vec![11]))
            .unwrap();

        store
            .commit_reorg(
                2,
                &hash(12),
                Bytes::from_vec(vec![12]),
                &[(1, hash(11)), (2, hash(12))],
            )
            .unwrap();
        assert_eq!(store.available_cursor(), Some(2));
        assert_eq!(store.read_by_height(1).unwrap().0, hash(11));
        assert_eq!(store.read_by_height(2).unwrap().0, hash(12));

        store
            .commit_reorg(1, &hash(21), Bytes::from_vec(vec![21]), &[(1, hash(21))])
            .unwrap();
        assert_eq!(store.available_cursor(), Some(1));
        assert_eq!(store.read_by_height(1).unwrap().0, hash(21));
        assert!(store.read_by_height(2).is_none());
        assert_eq!(store.read_by_hash(&hash(2)).unwrap().as_ref(), &[2]);
    }

    struct RejectWrites;

    impl DiskDB for RejectWrites {
        fn read(&self, _key: &[u8]) -> Option<Vec<u8>> {
            None
        }

        fn save(&self, _key: &[u8], _val: &[u8]) {}

        fn remove(&self, _key: &[u8]) {}

        fn try_write(&self, _memkv: &dyn base::MemDB) -> Rerr {
            sys::errf!("injected write failure")
        }
    }

    #[test]
    fn block_store_propagates_recoverable_batch_failures() {
        let store = KvBlockStore::new(Arc::new(RejectWrites));
        assert!(
            store
                .put_block_available(1, &hash(1), Bytes::from_vec(vec![1]))
                .is_err()
        );
        assert_eq!(store.available_cursor(), None);
    }
}
