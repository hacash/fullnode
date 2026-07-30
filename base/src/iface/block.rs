use std::any::Any;
use std::sync::Arc;

use field::{Encode, Hash};
use sys::{Rerr, Ret};

use crate::iface::transaction::{Transaction, TxRef};

pub type BlockRef = Arc<dyn Block>;

/// Optional proof-of-work capability. Generic chain and block consumers do
/// not require it; PoW consensus implementations opt in explicitly.
pub trait PowBlock: Send + Sync {
    fn nonce(&self) -> u32;
    fn difficulty(&self) -> u32;
}

/// Cross-crate block contract owned by `base` and consumed by chain, node, and
/// registry code. Standard blocks are implemented in `protocol/src/codec/block.rs`;
/// the genesis wrapper is in `mint/src/consensus/genesis.rs`.
pub trait Block: Encode + Send + Sync + std::fmt::Debug {
    fn version(&self) -> u8;
    fn height(&self) -> u64;
    fn hash(&self) -> Hash;
    fn prev_hash(&self) -> Hash;
    fn mrklroot(&self) -> Hash;
    fn timestamp(&self) -> u64;
    fn as_pow(&self) -> Option<&dyn PowBlock> {
        None
    }

    fn transaction_count(&self) -> u32 {
        self.transactions().len() as u32
    }
    fn transactions(&self) -> &[TxRef];

    fn prelude_transaction(&self) -> Ret<&dyn Transaction> {
        match self.transactions().first() {
            Some(tx) => Ok(tx.as_ref()),
            None => sys::errf!("block has no prelude transaction"),
        }
    }

    fn transaction_hash_list(&self, with_fee: bool) -> Vec<Hash> {
        self.transactions()
            .iter()
            .map(|t| {
                if with_fee {
                    t.hash_with_fee()
                } else {
                    t.hash()
                }
            })
            .collect()
    }

    /// Escape hatch back to the concrete block type.
    ///
    /// **Prefer a trait method** when the capability is part of the block
    /// protocol (height, hash, prev_hash, mrklroot, transactions, ...): add a
    /// method to `Block` and have every implementation expose it.
    ///
    /// **Downcast is the right choice** for consensus-mechanism products
    /// (Hacash PoW difficulty/nonce fields, x16rs block intro layout) or
    /// chain-specific block payloads. The owning crate (mint, protocol) knows
    /// its own concrete block type; base must not encode those concepts.
    /// `downcast_ref` returning `None` on a chain that uses a different block
    /// type is the intended fallback.
    fn as_any(&self) -> &dyn Any;
}

pub trait BlockBuild: Block {
    fn update_mrklroot(&mut self);
    fn set_mrklroot(&mut self, root: Hash);
    fn replace_transaction(&mut self, idx: usize, tx: TxRef) -> Rerr;
    fn push_transaction(&mut self, tx: TxRef) -> Rerr;
}

pub trait PowBlockBuild: BlockBuild + PowBlock {
    fn set_nonce(&mut self, nonce: u32);
}

/// Convenience methods for code that is itself explicitly PoW-specific.
pub trait PowBlockExt: Block {
    fn pow(&self) -> &dyn PowBlock {
        self.as_pow()
            .expect("PoW consensus received a block without PowBlock capability")
    }

    fn pow_nonce(&self) -> u32 {
        self.pow().nonce()
    }

    fn pow_difficulty(&self) -> u32 {
        self.pow().difficulty()
    }
}

impl<T: Block + ?Sized> PowBlockExt for T {}

#[cfg(test)]
mod tests {
    use super::Block;
    use field::{Encode, Hash};
    use std::any::Any;

    #[derive(Debug)]
    struct NonPowBlock;

    impl Encode for NonPowBlock {
        fn size(&self) -> usize {
            0
        }

        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }

    impl Block for NonPowBlock {
        fn version(&self) -> u8 {
            1
        }
        fn height(&self) -> u64 {
            1
        }
        fn hash(&self) -> Hash {
            Hash::default()
        }
        fn prev_hash(&self) -> Hash {
            Hash::default()
        }
        fn mrklroot(&self) -> Hash {
            Hash::default()
        }
        fn timestamp(&self) -> u64 {
            0
        }
        fn transactions(&self) -> &[crate::TxRef] {
            &[]
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn block_does_not_require_pow_capability() {
        let block: &dyn Block = &NonPowBlock;
        assert!(block.as_pow().is_none());
    }
}
