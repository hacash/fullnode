//! Fast, deterministic multi-transaction execution harness.
//!
//! `MemChain` is a state/execution simulator rather than a counterfeit
//! `ChainEngine`: production block admission, fork choice and PoW remain in
//! `chain` integration tests. Every accepted transaction still runs through
//! the real `TransactionExecute` implementation and commits state on success.

use std::sync::Arc;

use base::{Context, ExecutionServices, TxRef};
use field::Hash;
use sys::Ret;

use super::context::TestContext;
use super::state::FlatMemState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxOutput {
    Success,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct TxReceipt {
    pub hash: Hash,
    pub output: TxOutput,
    pub gas_remaining: i64,
}

impl TxReceipt {
    pub fn is_success(&self) -> bool {
        matches!(self.output, TxOutput::Success)
    }

    pub fn is_error(&self) -> bool {
        !self.is_success()
    }

    pub fn expect_success(&self) -> &Self {
        assert!(
            self.is_success(),
            "transaction {} failed: {:?}",
            self.hash,
            self.output
        );
        self
    }

    pub fn expect_error_contains(&self, needle: &str) -> &Self {
        match &self.output {
            TxOutput::Error(message) if message.contains(needle) => self,
            TxOutput::Error(message) => {
                panic!("transaction error {message:?} does not contain {needle:?}")
            }
            TxOutput::Success => {
                panic!("transaction succeeded; expected error containing {needle:?}")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockReceipt {
    pub height: u64,
    pub receipts: Vec<TxReceipt>,
}

impl BlockReceipt {
    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }

    pub fn receipt(&self, hash: &Hash) -> Option<&TxReceipt> {
        self.receipts.iter().find(|receipt| receipt.hash == *hash)
    }

    pub fn expect_all_success(&self) -> &Self {
        for receipt in &self.receipts {
            receipt.expect_success();
        }
        self
    }
}

/// In-memory stateful transaction runner for VM/action integration tests.
pub struct MemChain {
    services: Arc<dyn ExecutionServices>,
    state: FlatMemState,
    pending: Vec<TxRef>,
    receipts: Vec<TxReceipt>,
    height: u64,
}

impl MemChain {
    pub fn new() -> Ret<Self> {
        Self::with_services(Arc::new(app::standard_registry()?))
    }

    pub fn with_services(services: Arc<dyn ExecutionServices>) -> Ret<Self> {
        // Fail before tests produce side effects if a codec-only registry was supplied.
        services.vm_params()?;
        Ok(Self {
            services,
            state: FlatMemState::default(),
            pending: Vec::new(),
            receipts: Vec::new(),
            height: 0,
        })
    }

    pub fn services(&self) -> Arc<dyn ExecutionServices> {
        self.services.clone()
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn state(&self) -> &FlatMemState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut FlatMemState {
        &mut self.state
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn receipts(&self) -> &[TxReceipt] {
        &self.receipts
    }

    pub fn receipt(&self, hash: &Hash) -> Option<&TxReceipt> {
        self.receipts.iter().find(|receipt| receipt.hash == *hash)
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub fn submit(&mut self, tx: TxRef) -> Hash {
        let hash = tx.hash();
        self.pending.push(tx);
        hash
    }

    /// Run queued transactions as the next simulated block. Failed transactions
    /// are isolated, so their state writes are discarded and later test cases
    /// can still observe the remaining receipts.
    pub fn mine_block(&mut self) -> BlockReceipt {
        self.height = self.height.saturating_add(1);
        let txs = std::mem::take(&mut self.pending);
        let receipts: Vec<TxReceipt> = txs.into_iter().map(|tx| self.execute(tx)).collect();
        self.receipts.extend(receipts.iter().cloned());
        BlockReceipt {
            height: self.height,
            receipts,
        }
    }

    pub fn execute_now(&mut self, tx: TxRef) -> TxReceipt {
        self.height = self.height.saturating_add(1);
        let receipt = self.execute(tx);
        self.receipts.push(receipt.clone());
        receipt
    }

    fn execute(&mut self, tx: TxRef) -> TxReceipt {
        let hash = tx.hash();
        let mut context = TestContext::new(self.services.clone(), tx.clone());
        context.layer = self.state.clone();
        context.env.block.height = self.height;
        if let Some(vm) = self.services.assign_vm(self.height) {
            context.set_vm(vm);
        }
        if let Some(gas_byte) = tx.gas_max_byte() {
            if let Ok(params) = self.services.vm_params() {
                let _ = context.gas_initialize(params.decode_gas_budget(gas_byte) as i64);
            }
        }
        let output = match tx.execute(&mut context) {
            Ok(()) => {
                self.state = context.layer.clone();
                TxOutput::Success
            }
            Err(error) => TxOutput::Error(error.to_string()),
        };
        let gas_remaining = context.gas_remaining();
        TxReceipt {
            hash,
            output,
            gas_remaining,
        }
    }
}

impl Default for MemChain {
    fn default() -> Self {
        Self::new().expect("standard test services")
    }
}
