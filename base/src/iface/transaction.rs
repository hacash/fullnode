use std::any::Any;
use std::sync::Arc;

use field::{AddrOrList, Address, Amount, Encode, Fixed16, Hash, Sign, Timestamp};
use sys::{Rerr, Ret};

use crate::iface::action::ActionRef;
use crate::iface::context::Context;

pub type TxRef = Arc<dyn Transaction>;

/// Common input for creating an unsigned user transaction.
///
/// Concrete protocol implementations choose which transaction type ids they
/// support. The request deliberately contains only fields shared by the
/// standard user transaction envelope; callers add actions and signatures via
/// the concrete builder API when needed.
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

/// Protocol-independent transaction creation boundary.
///
/// The caller selects a wire transaction type through [`TxCreateRequest`]; a
/// protocol implementation decides whether that type is available.
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

/// Cross-crate transaction contract owned by `base` and consumed by protocol,
/// chain, node, and registry code. Standard Type1/2/3 and prelude transactions
/// live in `protocol/src/codec/tx.rs`; the mining coinbase is in `mint/src/action`.
pub trait Transaction: Encode + Send + Sync + std::fmt::Debug {
    fn ty(&self) -> u8;
    fn hash(&self) -> Hash;
    fn hash_with_fee(&self) -> Hash {
        self.hash()
    }

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
    fn req_sign(&self) -> Ret<Vec<Address>> {
        Ok(vec![self.main()])
    }
    fn verify_signature(&self) -> Rerr;

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

    fn execute(&self, ctx: &mut dyn Context) -> Rerr;

    /// Escape hatch back to the concrete transaction type.
    ///
    /// **Prefer a trait method** when the capability belongs to the generic
    /// transaction envelope shared by every chain (type, hash, main address,
    /// fee, signature set, action list, ...): extend `Transaction` /
    /// `TransactionBuild` with a new method and have each implementation
    /// expose it.
    ///
    /// **Downcast is the right choice** for consensus-mechanism products
    /// (e.g. Hacash PoW coinbase mining-nonce, PoS validator signatures) or
    /// chain-specific transaction payloads (Hacash diamond bidding, channel
    /// state). Those concepts belong to the owning crate (mint, protocol,
    /// app composition root); base must not encode them. `downcast_ref`
    /// returning `None` on a chain that uses a different transaction type is
    /// the intended fallback, not a defect.
    fn as_any(&self) -> &dyn Any;
}

/// / trait / trait
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
    fn push_action(&mut self, _act: ActionRef) -> Rerr {
        sys::errf!("transaction does not support push_action")
    }
}
