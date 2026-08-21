use std::any::Any;
use std::sync::Arc;

use field::{AddrOrList, Address, Amount, Encode, Fixed16, Hash, Sign, Timestamp};
use sys::{Rerr, Ret};

use crate::iface::action::ActionRef;
#[cfg(feature = "execute")]
use crate::iface::context::Context;

/// Wire/offline view of a transaction (hash / req_sign / verify_signature). Execution
/// is a separate trait via `as_execute` in full builds — `TxRef` is type-stable across `execute`.
pub type TxRef = Arc<dyn TransactionSign>;

/// Common input for creating an unsigned user transaction. Contains only fields shared
/// by the standard user envelope; callers add actions/signatures via the concrete builder API.
#[derive(Clone, Debug)]
pub struct TxCreateRequest {
    pub ty: u8,
    pub timestamp: u64,
    pub addrlist: AddrOrList,
    pub fee: Amount,
    pub gas_max: u8,
}

impl TxCreateRequest {
    pub fn new(ty: u8, main: Address, fee: Amount, timestamp: u64) -> Self {
        Self {
            ty,
            timestamp,
            addrlist: AddrOrList::from_addr(main),
            fee,
            gas_max: 0,
        }
    }

    pub fn with_addrlist(mut self, addrlist: AddrOrList) -> Self {
        self.addrlist = addrlist;
        self
    }

    pub fn with_gas_max(mut self, gas_max: u8) -> Self {
        self.gas_max = gas_max;
        self
    }
}

/// Protocol-independent transaction creation boundary: the caller picks a wire type via
/// [`TxCreateRequest`]; a protocol implementation decides whether that type is available.
pub trait TransactionCreator: Send + Sync {
    fn create(&self, request: TxCreateRequest) -> Ret<TxRef>;
}

impl<F> TransactionCreator for F
where
    F: Fn(TxCreateRequest) -> Ret<TxRef> + Send + Sync,
{
    fn create(&self, request: TxCreateRequest) -> Ret<TxRef> {
        self(request)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MempoolPolicy {
    Allowed,
    Forbidden,
    OnlyLocal,
}

/// Cross-crate transaction contract (type, addresses, fee, actions, signatures, timestamp).
/// Hashing/signing live on `TransactionSign`; consensus execution on `TransactionExecute` (full builds only — SDK/wasm compile no execution surface).
pub trait Transaction: Encode + Send + Sync + std::fmt::Debug {
    fn ty(&self) -> u8;

    fn main(&self) -> Address;
    fn addrs(&self) -> Vec<Address> {
        vec![self.main()]
    }
    fn fee(&self) -> &Amount;
    fn fee_pay(&self) -> Amount {
        self.fee().clone()
    }
    fn fee_got(&self) -> Amount {
        self.fee().clone()
    }
    fn fee_purity(&self) -> u64 {
        0
    }
    /// Size used for fee purity / gas price. Type3 uses canonical SignW2 size.
    fn billing_size(&self) -> Ret<usize> {
        Ok(Encode::size(self))
    }
    fn gas_max_byte(&self) -> Option<u8> {
        None
    }
    fn timestamp(&self) -> &Timestamp {
        Timestamp::zero_ref()
    }
    fn nonce(&self) -> u64 {
        0
    }
    fn mempool_policy(&self) -> MempoolPolicy {
        MempoolPolicy::Allowed
    }
    fn is_block_prelude(&self) -> bool {
        false
    }
    fn required_flags(&self) -> u64 {
        0
    }

    fn action_count(&self) -> usize {
        self.actions().len()
    }
    fn actions(&self) -> &[ActionRef] {
        &[]
    }
    fn signs(&self) -> &[Sign] {
        &[]
    }

    fn author(&self) -> Option<Address> {
        None
    }
    fn block_reward(&self) -> Option<&Amount> {
        None
    }
    fn block_message(&self) -> Option<&Fixed16> {
        None
    }
    fn fee_receiver(&self) -> Option<Address> {
        self.author()
    }

    /// Escape hatch to the concrete transaction type. Prefer a trait method for generic envelope
    /// capabilities; downcast is for chain/consensus products owned by mint/protocol — `None` is the intended fallback.
    fn as_any(&self) -> &dyn Any;
}

/// Offline hashing/signing semantics: tx hash, required signer set, signature verification.
/// No state needed — the SDK's inspect/attach surface is built on this view.
pub trait TransactionSign: Transaction {
    fn hash(&self) -> Hash;
    fn hash_with_fee(&self) -> Hash {
        self.hash()
    }

    fn req_sign(&self) -> Ret<Vec<Address>> {
        Ok(vec![self.main()])
    }
    fn verify_signature(&self) -> Rerr;

    /// Upcast to the consensus execute view. Default `None` (codec-only/test stubs); real
    /// transactions return `Some(self)` when `execute` is on, keeping `TxRef` type-stable.
    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn TransactionExecute> {
        None
    }
}

/// Consensus execution view: `TransactionSign` plus the state-changing `execute` body.
/// Implemented only when `execute` is on — SDK/wasm has no callable execution surface.
#[cfg(feature = "execute")]
pub trait TransactionExecute: TransactionSign {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr;
}

#[cfg(feature = "execute")]
impl dyn TransactionSign {
    /// Consensus execution. Looks up the execute view instead of requiring
    /// `TxRef` to be a different type.
    pub fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        match self.as_execute() {
            Some(exec) => TransactionExecute::execute(exec, ctx),
            None => sys::errf!("transaction type {} has no execute surface", self.ty()),
        }
    }
}

/// Offline build/mutation view: mutable fee, nonce, signatures and actions for construction.
pub trait TransactionBuild: Transaction {
    fn set_fee(&mut self, _fee: Amount) {}
    fn set_nonce(&mut self, _nonce: u64) {}
    fn set_mining_nonce(&mut self, _nonce: Hash) {}
    fn fill_sign(&mut self, _acc_addr: &Address) -> Ret<Sign> {
        sys::errf!("transaction does not support fill_sign")
    }
    fn push_sign(&mut self, _sg: Sign) -> Rerr {
        sys::errf!("transaction does not support push_sign")
    }
    /// Insert or replace a signature without verifying it against the digest — validity is
    /// a separate capability (`verify_signature`); construction must not refuse a well-formed `Sign`.
    fn insert_sign(&mut self, _sg: Sign) -> Rerr {
        sys::errf!("transaction does not support insert_sign")
    }
    fn push_action(&mut self, _act: ActionRef) -> Rerr {
        sys::errf!("transaction does not support push_action")
    }
}
