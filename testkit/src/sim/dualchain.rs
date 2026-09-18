//! Differential execution harness built from two independent [`MemChain`]s.

use base::{ExecutionServices, TxRef};
use field::Hash;
use std::sync::Arc;

use super::memchain::{BlockReceipt, MemChain, TxReceipt};

/// Executes each submitted transaction in two independent in-memory contexts
/// and fails fast on a divergent receipt or final key/value state.
pub struct DualMemChain {
    left: MemChain,
    right: MemChain,
}

impl DualMemChain {
    pub fn new() -> sys::Ret<Self> {
        let services: Arc<dyn ExecutionServices> = Arc::new(app::standard_registry()?);
        Self::with_services(services)
    }

    pub fn with_services(services: Arc<dyn ExecutionServices>) -> sys::Ret<Self> {
        Ok(Self {
            left: MemChain::with_services(services.clone())?,
            right: MemChain::with_services(services)?,
        })
    }

    pub fn left(&self) -> &MemChain {
        &self.left
    }
    pub fn right(&self) -> &MemChain {
        &self.right
    }
    pub fn left_mut(&mut self) -> &mut MemChain {
        &mut self.left
    }
    pub fn right_mut(&mut self) -> &mut MemChain {
        &mut self.right
    }
    pub fn height(&self) -> u64 {
        self.assert_same();
        self.left.height()
    }
    pub fn pending_len(&self) -> usize {
        self.assert_same();
        self.left.pending_len()
    }

    pub fn submit(&mut self, tx: TxRef) -> Hash {
        let left = self.left.submit(tx.clone());
        let right = self.right.submit(tx);
        assert_eq!(
            left, right,
            "dual chain assigned divergent transaction hashes"
        );
        left
    }

    pub fn mine_block(&mut self) -> BlockReceipt {
        let left = self.left.mine_block();
        let right = self.right.mine_block();
        assert_same_receipts(&left, &right);
        self.assert_same();
        left
    }

    pub fn execute_now(&mut self, tx: TxRef) -> TxReceipt {
        let left = self.left.execute_now(tx.clone());
        let right = self.right.execute_now(tx);
        assert_eq!(left.hash, right.hash, "dual chain receipt hash mismatch");
        assert_eq!(
            left.output, right.output,
            "dual chain transaction output mismatch"
        );
        assert_eq!(
            left.gas_remaining, right.gas_remaining,
            "dual chain gas mismatch"
        );
        self.assert_same();
        left
    }

    pub fn assert_same(&self) {
        assert_eq!(
            self.left.height(),
            self.right.height(),
            "dual chain height mismatch"
        );
        assert_eq!(
            self.left.pending_len(),
            self.right.pending_len(),
            "dual chain pending-count mismatch"
        );
        assert_eq!(
            self.left.state().entries(),
            self.right.state().entries(),
            "dual chain state mismatch"
        );
    }
}

impl Default for DualMemChain {
    fn default() -> Self {
        Self::new().expect("standard dual test services")
    }
}

fn assert_same_receipts(left: &BlockReceipt, right: &BlockReceipt) {
    assert_eq!(
        left.height, right.height,
        "dual chain block height mismatch"
    );
    assert_eq!(
        left.receipts.len(),
        right.receipts.len(),
        "dual chain receipt count mismatch"
    );
    for (left, right) in left.receipts.iter().zip(&right.receipts) {
        assert_eq!(left.hash, right.hash, "dual chain receipt hash mismatch");
        assert_eq!(
            left.output, right.output,
            "dual chain receipt output mismatch"
        );
        assert_eq!(
            left.gas_remaining, right.gas_remaining,
            "dual chain receipt gas mismatch"
        );
    }
}
