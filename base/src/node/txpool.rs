use field::Hash;
use sys::{Rerr, Ret};

use crate::chain::TxPkg;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxGroupId(u16);

impl TxGroupId {
    pub const DEFAULT: Self = Self(0);

    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for TxGroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxOrdering {
    FeePurity,
    Fee,
    Fifo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxPoolGroupSpec {
    pub id: TxGroupId,
    pub name: String,
    pub default_capacity: usize,
    pub ordering: TxOrdering,
    /// Optional accepted-block interval for generic re-execution maintenance.
    pub revalidate_interval: Option<u64>,
    pub relay_service_bit: Option<u64>,
}

impl TxPoolGroupSpec {
    pub fn new(id: TxGroupId, name: impl Into<String>, ordering: TxOrdering) -> Self {
        Self {
            id,
            name: name.into(),
            default_capacity: usize::MAX,
            ordering,
            revalidate_interval: Some(11),
            relay_service_bit: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxAdmissionStatus {
    AcceptedPool,
    AcceptedBroadcast,
    Duplicate,
    Rejected,
    Replaced,
    Ignored,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxRejectReason {
    Malformed(String),
    NonCanonical(String),
    TooLarge { size: usize, max: usize },
    FeeTooLow { got: u64, min: u64 },
    MempoolForbidden,
    InvalidSignature(String),
    ExecutionFailed(String),
    PoolFull,
    Policy(String),
}

impl TxRejectReason {
    /// Human-readable rejection message. Mirrors the wording the dev node
    /// returned through the HTTP API so clients keep recognizing the errors.
    pub fn as_message(&self) -> String {
        match self {
            TxRejectReason::Malformed(s) => s.clone(),
            TxRejectReason::NonCanonical(s) => s.clone(),
            TxRejectReason::TooLarge { size, max } => format!(
                "tx size {} exceeds maximum {} bytes",
                size, max
            ),
            TxRejectReason::FeeTooLow { got, min } => format!(
                "The transaction fee purity {} is too low, the node minimum configuration is {}.",
                got, min
            ),
            TxRejectReason::MempoolForbidden => {
                "transaction type is forbidden in mempool".to_owned()
            }
            TxRejectReason::InvalidSignature(s) => s.clone(),
            TxRejectReason::ExecutionFailed(s) => s.clone(),
            TxRejectReason::PoolFull => "transaction pool is full".to_owned(),
            TxRejectReason::Policy(s) => s.clone(),
        }
    }
}

/// A valid transaction may be unsuitable for this node's bounded local pool
/// while still being eligible for network relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxPoolInsertReject {
    Capacity,
    UnderpricedReplacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxPoolInsertOutcome {
    Stored,
    NotStored(TxPoolInsertReject),
}

#[derive(Clone, Debug)]
pub struct TxSubmitResult {
    pub status: TxAdmissionStatus,
    pub hash: Hash,
    pub group: Option<TxGroupId>,
    pub reason: Option<TxRejectReason>,
}

impl TxSubmitResult {
    pub fn accepted(hash: Hash, group: TxGroupId, relay: bool) -> Self {
        let status = if relay {
            TxAdmissionStatus::AcceptedBroadcast
        } else {
            TxAdmissionStatus::AcceptedPool
        };
        Self {
            status,
            hash,
            group: Some(group),
            reason: None,
        }
    }

    pub fn should_relay(&self) -> bool {
        self.status == TxAdmissionStatus::AcceptedBroadcast
    }

    pub fn rejected(hash: Hash, reason: TxRejectReason) -> Self {
        Self {
            status: TxAdmissionStatus::Rejected,
            hash,
            group: None,
            reason: Some(reason),
        }
    }

    pub fn duplicate(hash: Hash) -> Self {
        Self {
            status: TxAdmissionStatus::Duplicate,
            hash,
            group: None,
            reason: None,
        }
    }
}

// =============================================================
// =============================================================

pub trait TxPool: Send + Sync {
    fn min_fee_purity(&self) -> u64 {
        0
    }
    fn group_ids(&self) -> Vec<TxGroupId>;
    fn count(&self, group: TxGroupId) -> usize;
    fn first(&self, group: TxGroupId) -> Option<TxPkg>;
    fn iter(&self, _group: TxGroupId, _f: &mut dyn FnMut(&TxPkg) -> bool) -> Rerr {
        Ok(())
    }
    fn insert(&self, group: TxGroupId, tx: TxPkg) -> Ret<TxPoolInsertOutcome>;
    fn insert_by(
        &self,
        tx: TxPkg,
        picker: &dyn Fn(&TxPkg) -> TxGroupId,
    ) -> Ret<TxPoolInsertOutcome> {
        self.insert(picker(&tx), tx)
    }
    fn find(&self, hash: &[u8]) -> Option<TxPkg>;
    fn take(&self, group: TxGroupId, max: usize) -> Vec<TxPkg>;
    fn remove(&self, group: TxGroupId, hashes: &[Hash]) -> Rerr;
    fn clear(&self, _group: TxGroupId) -> Rerr {
        Ok(())
    }
    fn retain(&self, _group: TxGroupId, _keep: &mut dyn FnMut(&TxPkg) -> bool) -> Rerr {
        Ok(())
    }
    fn drain(&self, _hashes: &[Hash]) -> Vec<TxPkg> {
        vec![]
    }
    fn print(&self) -> String {
        String::new()
    }
}
