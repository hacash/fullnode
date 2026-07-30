//! One-shot block batch source for sync pipeline.

use base::{BlockBatch, BlockSource};

pub(crate) struct OneShotBlocks {
    batch: Option<BlockBatch>,
}

impl OneShotBlocks {
    pub fn from_batch(batch: BlockBatch) -> Self {
        Self { batch: Some(batch) }
    }
}

impl BlockSource for OneShotBlocks {
    fn next(&mut self) -> sys::Ret<Option<BlockBatch>> {
        Ok(self.batch.take())
    }
}
