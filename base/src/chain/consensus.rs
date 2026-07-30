use std::sync::Arc;

use field::{Address, Hash};
use sys::{Rerr, Ret, Waiter};

use crate::chain::{BlkPkg, TxPkg};
use crate::chain::{ChainView, Engine};
use crate::node::{Peer, TxGroupId, TxOrdering, TxPool, TxPoolGroupSpec};
use crate::state::{StateChunkRef, StateLayer, StateRead};
use crate::{Block, BlockRef, Transaction};

/// Consensus-defined wire and transaction limits. The generic chain runtime
/// carries this shape, while each consensus implementation owns its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MintParams {
    pub max_block_txs: usize,
    pub max_block_size: usize,
    pub max_tx_size: usize,
    pub difficulty_adjust_blocks: u64,
    pub difficulty_group_blocks: u64,
    pub each_block_target_time: u64,
}

pub trait BlockHistory: Send + Sync {
    fn stable_height(&self) -> u64;
    fn block_at_height(&self, height: u64) -> Option<BlockRef>;
}

/// Opaque, lexicographically ordered score used by the generic fork tree.
/// Consensus implementations are responsible for encoding their complete
/// branch priority into this key. Equal keys keep the current head, making
/// tie handling deterministic and independent of arrival-order callbacks.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForkChoiceKey(Vec<u8>);

impl ForkChoiceKey {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn from_height(height: u64) -> Self {
        Self(height.to_be_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Consensus-owned branch scoring. This runs before the tree write lock is
/// acquired; the tree only compares the returned immutable keys.
pub trait ForkChoice: Send + Sync {
    fn fork_choice_key(
        &self,
        block: &BlkPkg,
        _parent_key: &ForkChoiceKey,
        _history: &dyn BlockHistory,
    ) -> Ret<ForkChoiceKey> {
        Ok(ForkChoiceKey::from_height(block.height()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeferredId(u64);

impl DeferredId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

pub struct DeferredCandidate {
    pub blocks: Vec<BlkPkg>,
}

pub struct DeferredBatch {
    pub id: DeferredId,
    pub candidates: Vec<DeferredCandidate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferredBatchResult {
    Accepted { candidate: usize },
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAdmissionDecision {
    Continue,
    Defer(DeferredId),
}

pub trait Consensus: Send + Sync {
    fn name(&self) -> &str;

    fn chain_id(&self) -> crate::ChainId;

    fn mint_params(&self) -> MintParams;

    fn genesis_block(&self) -> BlockRef;

    fn initialize(&self, _layer: &mut dyn StateLayer) -> Rerr {
        Ok(())
    }

    /// Validate consensus configuration persisted in the canonical genesis
    /// state before the engine repairs, rebuilds, or replays the chain.
    ///
    /// A missing value is left to the consensus implementation to classify:
    /// an empty genesis state may be initialized, while an already advanced
    /// chain can be rejected as unverifiable.
    fn validate_genesis_state(&self, _state: &dyn StateRead, _root_height: u64) -> Rerr {
        Ok(())
    }

    /// Whether a root-height-zero state needs genesis reconstruction so the
    /// consensus can materialize a newly introduced genesis marker.
    fn genesis_state_needs_rebuild(&self, _state: &dyn StateRead, _root_height: u64) -> bool {
        false
    }

    fn chain_flags(&self, _height: u64) -> u64 {
        0
    }

    fn check_block_admission(
        &self,
        _pkg: &BlkPkg,
        _view: &dyn ChainView,
    ) -> Ret<BlockAdmissionDecision> {
        Ok(BlockAdmissionDecision::Continue)
    }

    fn check_block_data(&self, _data: &[u8], _view: &dyn ChainView) -> Rerr {
        Ok(())
    }

    /// Cheap arrival gate over the fixed block intro, before full transaction
    /// decoding. Implementations may leave this at the default and perform
    /// the package-level check in `check_block_arrive` instead.
    fn check_block_arrive_data(&self, _data: &[u8], _view: &dyn ChainView) -> Rerr {
        Ok(())
    }

    fn check_block_arrive(&self, _pkg: &BlkPkg, _view: &dyn ChainView) -> Rerr {
        Ok(())
    }

    fn check_block_before_execute(
        &self,
        _pkg: &BlkPkg,
        _parent: &dyn Block,
        _history: &dyn BlockHistory,
    ) -> Rerr {
        Ok(())
    }

    fn check_block_after_execute(
        &self,
        _pkg: &BlkPkg,
        _new_state: &StateChunkRef,
        _parent_state: &dyn StateRead,
        _view: &dyn ChainView,
    ) -> Rerr {
        Ok(())
    }

    fn on_stable_block(&self, _block: &dyn Block, _view: &dyn ChainView) {}
}

/// Transaction-pool policy owned by the active consensus implementation.
pub trait TxPolicy: Send + Sync {
    fn check_tx(&self, _view: &dyn ChainView, _tx: &TxPkg) -> Rerr {
        Ok(())
    }

    /// Whether a failed pool revalidation conclusively makes this transaction
    /// removable. `false` keeps it and stops before judging dependent entries.
    fn failed_revalidation_can_remove(&self, _tx: &dyn Transaction) -> bool {
        true
    }

    fn tx_pool_groups(&self) -> Vec<TxPoolGroupSpec> {
        vec![TxPoolGroupSpec::new(
            TxGroupId::DEFAULT,
            "default",
            TxOrdering::FeePurity,
        )]
    }

    fn tx_pool_group(&self, _tx: &TxPkg) -> TxGroupId {
        TxGroupId::DEFAULT
    }

    fn on_txs_confirmed(
        &self,
        _view: &dyn ChainView,
        _txpool: &dyn TxPool,
        _txs: Vec<Hash>,
        _height: u64,
    ) {
    }
}

/// Optional block construction owned by the active consensus implementation.
pub trait BlockProducer: Send + Sync {
    fn external_exec_author(&self) -> Address {
        Address::default()
    }

    fn build_next_block(
        &self,
        _engine: &dyn Engine,
        _txpool: &dyn TxPool,
    ) -> Ret<Option<BlockRef>> {
        Ok(None)
    }
}

/// Node-facing consensus hooks. These are intentionally outside `Consensus`
/// so block validation never depends on node or peer abstractions.
pub trait ConsensusNodeHooks: Send + Sync {
    fn on_p2p_connect(
        &self,
        _peer: Arc<dyn Peer>,
        _engine: Arc<dyn Engine>,
        _txpool: Arc<dyn TxPool>,
    ) -> Rerr {
        Ok(())
    }

    fn poll_deferred_batches(&self, _view: &dyn ChainView) -> Vec<DeferredBatch> {
        vec![]
    }

    fn on_deferred_batch_result(&self, _id: DeferredId, _result: DeferredBatchResult) {}

    fn start(&self, _waiter: Waiter) -> Rerr {
        Ok(())
    }

    fn exit(&self) {}
}

/// One assembled consensus service used by the engine. The component traits
/// let chain, node and mining code request only the capability they need.
pub trait ConsensusRuntime:
    Consensus + ForkChoice + TxPolicy + BlockProducer + ConsensusNodeHooks
{
}

impl<T> ConsensusRuntime for T where
    T: Consensus + ForkChoice + TxPolicy + BlockProducer + ConsensusNodeHooks
{
}
