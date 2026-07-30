//! Block sources for local replay and one-shot batches.

use std::sync::Arc;

use base::{BlockBatch, BlockSource, BlockStore};
use sys::Ret;

/// A single pre-assembled blob of concatenated blocks.
pub struct OneShot {
    blob: Option<Arc<Vec<u8>>>,
}

impl OneShot {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            blob: Some(Arc::new(data)),
        }
    }
}

impl BlockSource for OneShot {
    fn next(&mut self) -> Ret<Option<BlockBatch>> {
        Ok(self.blob.take().map(|bytes| BlockBatch::raw(bytes, 0)))
    }
}

/// Reads stored blocks back out of the block DB, for state rebuilds.
pub struct LocalReplay {
    store: Arc<dyn BlockStore>,
    next: u64,
    end: u64,
    batch: usize,
}

impl LocalReplay {
    pub fn new(store: Arc<dyn BlockStore>, from: u64, to: u64) -> Self {
        Self {
            store,
            next: from,
            end: to,
            batch: 64,
        }
    }

    pub fn with_batch(mut self, batch: usize) -> Self {
        self.batch = batch.max(1);
        self
    }
}

impl BlockSource for LocalReplay {
    fn next(&mut self) -> Ret<Option<BlockBatch>> {
        if self.next > self.end {
            return Ok(None);
        }
        let mut buf = Vec::new();
        for _ in 0..self.batch {
            if self.next > self.end {
                break;
            }
            let Some((_, data)) = self.store.read_by_height(self.next) else {
                return sys::errf!("replay: block {} missing from the block db", self.next);
            };
            buf.extend_from_slice(data.as_ref());
            self.next += 1;
        }
        Ok(Some(BlockBatch::raw(Arc::new(buf), self.end)))
    }
}
