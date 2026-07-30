//! /""
//!
//! `Arc`

use std::sync::Arc;

use field::Hash;
use sys::{Bytes, Ret};

use crate::registry::BinaryCodecs;
use crate::{BlockRef, TxRef};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PkgOrigin {
    #[default]
    Unknown,
    Local,
    Sync,
    Broadcast,
    Mining,
    Rebuild,
    Replay,
    Api,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PkgSource {
    pub origin: PkgOrigin,
    pub peer: Option<String>,
    pub received_at: u64,
}

impl PkgSource {
    pub fn new(origin: PkgOrigin) -> Self {
        Self {
            origin,
            peer: None,
            received_at: 0,
        }
    }
    pub fn with_peer(mut self, peer: impl Into<String>) -> Self {
        self.peer = Some(peer.into());
        self
    }
    pub fn with_received_at(mut self, received_at: u64) -> Self {
        self.received_at = received_at;
        self
    }
}

#[derive(Clone)]
pub struct TxPkg {
    obj: TxRef,
    data: Bytes,
    hash: Hash,
    fee_purity: u64,
    source: PkgSource,
}

impl TxPkg {
    pub fn from_bytes(reg: &dyn BinaryCodecs, data: Vec<u8>, source: PkgSource) -> Ret<Self> {
        let obj = reg.decode_transaction_exact(data.as_slice())?;
        Ok(Self::from_transaction_with_data(
            obj,
            Bytes::from_vec(data),
            source,
        ))
    }

    pub fn from_transaction(obj: TxRef, source: PkgSource) -> Self {
        let data = Bytes::from_vec(obj.encode());
        Self::from_transaction_with_data(obj, data, source)
    }

    fn from_transaction_with_data(obj: TxRef, data: Bytes, source: PkgSource) -> Self {
        let hash = obj.hash();
        let fee_purity = obj.fee_purity();
        Self {
            obj,
            data,
            hash,
            fee_purity,
            source,
        }
    }

    pub fn tx(&self) -> &dyn crate::Transaction {
        self.obj.as_ref()
    }

    pub fn tx_ref(&self) -> TxRef {
        self.obj.clone()
    }

    pub fn data(&self) -> &Bytes {
        &self.data
    }

    pub fn hash(&self) -> Hash {
        self.hash
    }

    pub fn fee_purity(&self) -> u64 {
        self.fee_purity
    }

    pub fn origin(&self) -> PkgOrigin {
        self.source.origin
    }

    pub fn source(&self) -> &PkgSource {
        &self.source
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[derive(Clone)]
pub struct BlkPkg {
    obj: BlockRef,
    data: Bytes,
    hash: Hash,
    source: PkgSource,
}

impl BlkPkg {
    pub fn from_bytes(reg: &dyn BinaryCodecs, data: Vec<u8>, source: PkgSource) -> Ret<Self> {
        let obj = reg.decode_block_exact(data.as_slice())?;
        Ok(Self::from_block_with_data(
            obj,
            Bytes::from_vec(data),
            source,
        ))
    }

    pub fn from_block(obj: BlockRef, source: PkgSource) -> Self {
        let data = Bytes::from_vec(obj.encode());
        Self::from_block_with_data(obj, data, source)
    }

    /// blob  `[off, off+len)` `BlkPkg.data`  blob
    /// range decode
    pub fn from_shared(
        reg: &dyn BinaryCodecs,
        data: Arc<Vec<u8>>,
        off: usize,
        len: usize,
        source: PkgSource,
    ) -> Ret<Self> {
        let Some(end) = off.checked_add(len) else {
            return sys::errf!("shared block range overflow");
        };
        if end > data.len() {
            return sys::errf!(
                "shared block range {}..{} exceeds payload length {}",
                off,
                end,
                data.len()
            );
        }
        let slice = &data[off..end];
        let obj = reg.decode_block_exact(slice)?;
        Ok(Self::from_block_with_data(
            obj,
            Bytes::from_arc_vec_slice(data, off, len),
            source,
        ))
    }

    pub fn from_shared_decoded(
        data: Arc<Vec<u8>>,
        off: usize,
        len: usize,
        obj: BlockRef,
        source: PkgSource,
    ) -> Ret<Self> {
        let Some(end) = off.checked_add(len) else {
            return sys::errf!("shared block range overflow");
        };
        if end > data.len() {
            return sys::errf!(
                "shared block range {}..{} exceeds payload length {}",
                off,
                end,
                data.len()
            );
        }
        if obj.encode().as_slice() != &data[off..end] {
            return sys::errf!("pre-decoded block does not match its shared payload");
        }
        Ok(Self::from_block_with_data(
            obj,
            Bytes::from_arc_vec_slice(data, off, len),
            source,
        ))
    }

    fn from_block_with_data(obj: BlockRef, data: Bytes, source: PkgSource) -> Self {
        let hash = obj.hash();
        Self {
            obj,
            data,
            hash,
            source,
        }
    }

    pub fn block(&self) -> &dyn crate::Block {
        self.obj.as_ref()
    }

    pub fn block_ref(&self) -> BlockRef {
        self.obj.clone()
    }

    pub fn data(&self) -> &Bytes {
        &self.data
    }

    pub fn hash(&self) -> Hash {
        self.hash
    }

    pub fn height(&self) -> u64 {
        self.obj.height()
    }

    pub fn origin(&self) -> PkgOrigin {
        self.source.origin
    }

    pub fn source(&self) -> &PkgSource {
        &self.source
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;

    use field::{Address, Amount, Encode, Hash};

    use crate::{Block, BlockRef, Context, PkgOrigin, PkgSource, Transaction, TxRef};

    use super::{BlkPkg, TxPkg};

    #[derive(Debug)]
    struct TestTx {
        hash: Hash,
        fee_purity: u64,
    }

    impl Encode for TestTx {
        fn size(&self) -> usize {
            32
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(self.hash.as_bytes());
        }
    }

    impl Transaction for TestTx {
        fn ty(&self) -> u8 {
            2
        }

        fn hash(&self) -> Hash {
            self.hash
        }

        fn main(&self) -> Address {
            Address::default()
        }

        fn fee(&self) -> &Amount {
            Amount::zero_ref()
        }

        fn fee_purity(&self) -> u64 {
            self.fee_purity
        }

        fn verify_signature(&self) -> sys::Rerr {
            Ok(())
        }

        fn execute(&self, _ctx: &mut dyn Context) -> sys::Rerr {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
        hash: Hash,
    }

    impl Encode for TestBlock {
        fn size(&self) -> usize {
            40
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(&self.height.to_be_bytes());
            out.extend_from_slice(self.hash.as_bytes());
        }
    }

    impl Block for TestBlock {
        fn version(&self) -> u8 {
            1
        }

        fn height(&self) -> u64 {
            self.height
        }

        fn hash(&self) -> Hash {
            self.hash
        }

        fn prev_hash(&self) -> Hash {
            Hash::default()
        }

        fn mrklroot(&self) -> Hash {
            Hash::default()
        }

        fn timestamp(&self) -> u64 {
            1
        }

        fn transactions(&self) -> &[TxRef] {
            &[]
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn test_block(height: u64, byte: u8) -> BlockRef {
        Arc::new(TestBlock {
            height,
            hash: Hash::from([byte; 32]),
        })
    }

    #[test]
    fn transaction_identity_and_fee_purity_come_from_the_private_object() {
        let tx: TxRef = Arc::new(TestTx {
            hash: Hash::from([3; 32]),
            fee_purity: 99,
        });
        let pkg = TxPkg::from_transaction(
            tx.clone(),
            PkgSource::new(PkgOrigin::Api).with_received_at(123),
        );

        assert_eq!(pkg.hash(), tx.hash());
        assert_eq!(pkg.fee_purity(), tx.fee_purity());
        assert_eq!(pkg.data().as_ref(), tx.encode().as_slice());
        assert_eq!(pkg.origin(), PkgOrigin::Api);
        assert_eq!(pkg.source().received_at, 123);
    }

    #[test]
    fn block_identity_and_height_come_from_the_private_object() {
        let block = test_block(5, 7);
        let pkg = BlkPkg::from_block(
            block.clone(),
            PkgSource::new(PkgOrigin::Broadcast).with_peer("peer-1"),
        );

        assert_eq!(pkg.height(), block.height());
        assert_eq!(pkg.hash(), block.hash());
        assert_eq!(pkg.data().as_ref(), block.encode().as_slice());
        assert_eq!(pkg.origin(), PkgOrigin::Broadcast);
        assert_eq!(pkg.source().peer.as_deref(), Some("peer-1"));
    }

    #[test]
    fn shared_decoded_block_must_match_the_wire_payload() {
        let encoded = Arc::new(test_block(5, 7).encode());
        let result = BlkPkg::from_shared_decoded(
            encoded.clone(),
            0,
            encoded.len(),
            test_block(6, 8),
            PkgSource::new(PkgOrigin::Sync),
        );

        assert!(result.is_err());
    }
}
