//! Block sources for local replay and one-shot batches.

use std::sync::Arc;

use base::{BlockBatch, BlockSource, BlockStore, ExecutionServices};
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

/// Reads stored blocks back out of the block DB, for state rebuilds. Bodies
/// are decoded once here, and the batch carries the decoded blocks, so the
/// sync pipeline reuses them instead of decoding every frame a second time.
pub struct LocalReplay {
    registry: Arc<dyn ExecutionServices>,
    store: Arc<dyn BlockStore>,
    next: u64,
    end: u64,
    batch: usize,
}

impl LocalReplay {
    pub fn new(
        registry: Arc<dyn ExecutionServices>,
        store: Arc<dyn BlockStore>,
        from: u64,
        to: u64,
    ) -> Self {
        Self {
            registry,
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
        let mut offsets = Vec::with_capacity(self.batch);
        let mut decoded = Vec::with_capacity(self.batch);
        for _ in 0..self.batch {
            if self.next > self.end {
                break;
            }
            let Some((_, data)) = self.store.read_by_height(self.next)? else {
                return sys::errf!("replay: block {} missing from the block db", self.next);
            };
            let (block, used) = self.registry.decode_block(data.as_ref())?;
            if used == 0 || used != data.len() {
                return sys::errf!(
                    "replay: block {} stored body does not decode to exactly one frame",
                    self.next
                );
            }
            offsets.push(buf.len() as u32);
            decoded.push(block);
            buf.extend_from_slice(data.as_ref());
            self.next += 1;
        }
        Ok(Some(BlockBatch {
            bytes: Arc::new(buf),
            remote_height: self.end,
            block_count: decoded.len() as u32,
            block_offsets: Arc::new(offsets),
            decoded_blocks: Arc::new(decoded),
        }))
    }
}
