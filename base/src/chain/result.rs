use field::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAcceptStatus {
    Accepted,
    Duplicate,
    Orphan,
    Deferred,
    Ignored,
}

#[derive(Clone, Debug)]
pub struct BlockRejectReason {
    pub code: &'static str,
    pub message: String,
}

impl BlockRejectReason {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockAcceptResult {
    pub status: BlockAcceptStatus,
    pub height: Option<u64>,
    pub confirmed_txs: Vec<Hash>,
    pub reverted_txs: Vec<Hash>,
    pub reason: Option<BlockRejectReason>,
    pub requested_parents: Vec<Hash>,
}

impl BlockAcceptResult {
    pub fn accepted(height: u64, confirmed_txs: Vec<Hash>, reverted_txs: Vec<Hash>) -> Self {
        Self {
            status: BlockAcceptStatus::Accepted,
            height: Some(height),
            confirmed_txs,
            reverted_txs,
            reason: None,
            requested_parents: vec![],
        }
    }

    pub fn should_relay(&self) -> bool {
        matches!(
            self.status,
            BlockAcceptStatus::Accepted | BlockAcceptStatus::Deferred
        )
    }

    pub fn deferred() -> Self {
        Self::empty(BlockAcceptStatus::Deferred)
    }

    pub fn duplicate(hash: Hash) -> Self {
        let mut result = Self::empty(BlockAcceptStatus::Duplicate);
        result.reason = Some(BlockRejectReason::new(
            "duplicate",
            format!("block {:?} already known", hash),
        ));
        result
    }

    pub fn orphan(parent: Hash) -> Self {
        let mut result = Self::empty(BlockAcceptStatus::Orphan);
        result.reason = Some(BlockRejectReason::new(
            "missing_parent",
            format!("parent {:?} not found", parent),
        ));
        result.requested_parents.push(parent);
        result
    }

    /// Unclassifiable block (e.g. a side branch dropped after an execution or
    /// body-write failure). No peer penalty, no relay; the stream continues.
    pub fn ignored() -> Self {
        Self::empty(BlockAcceptStatus::Ignored)
    }

    fn empty(status: BlockAcceptStatus) -> Self {
        Self {
            status,
            height: None,
            confirmed_txs: vec![],
            reverted_txs: vec![],
            reason: None,
            requested_parents: vec![],
        }
    }
}
