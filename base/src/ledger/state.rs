use crate::{
    StateLayer, StateRead, numeric_state_empty_key, numeric_state_key, numeric_state_prefix,
    read_typed,
};
use field::{
    Address, AssetSmelt, Balance, BlockHeight, Decode, DiamondName, DiamondNumber,
    DiamondOwnedForm, DiamondSmelt, DiamondSto, Encode, Fold64, Hash,
};
use sys::Ret;

use super::total::BaseTotal;

const KEY_BASE_TOTAL: u8 = numeric_state_prefix(0x01);
const KEY_LATEST_DIAMOND: u8 = numeric_state_prefix(0x02);
const KEY_TX_EXIST: u8 = numeric_state_prefix(0x0a);
const KEY_BALANCE: u8 = numeric_state_prefix(0x0b);
const KEY_DIAMOND: u8 = numeric_state_prefix(0x0d);
const KEY_DIAMOND_NAME: u8 = numeric_state_prefix(0x0e);
const KEY_DIAMOND_SMELT: u8 = numeric_state_prefix(0x0f);
const KEY_DIAMOND_OWNED: u8 = numeric_state_prefix(0x10);
const KEY_ASSET: u8 = numeric_state_prefix(0x11);

/// Decode a typed value via the shared `read_typed` helper (§6.2): backend failures
/// propagate, undecodable bytes surface as `Abort + STATE_DECODE_FAILED_CODE`, missing key stays `Ok(None)`.
fn get_typed<T: Decode>(sta: &dyn StateRead, key: &[u8]) -> Ret<Option<T>> {
    read_typed(sta, key)
}

pub struct CoreState<'a>(pub &'a mut dyn StateLayer);

pub struct CoreStateRead<'a>(pub &'a dyn StateRead);

impl<'a> CoreStateRead<'a> {
    pub fn wrap(read: &'a dyn StateRead) -> Self {
        Self(read)
    }

    fn get_value<T: Decode>(&self, key: &[u8]) -> Ret<Option<T>> {
        get_typed(self.0, key)
    }

    pub fn base_total(&self) -> Ret<Option<BaseTotal>> {
        self.get_value(&CoreState::key_empty(KEY_BASE_TOTAL))
    }

    pub fn get_base_total(&self) -> Ret<BaseTotal> {
        Ok(self.base_total()?.unwrap_or_default())
    }

    pub fn latest_diamond(&self) -> Ret<Option<DiamondSmelt>> {
        self.get_value(&CoreState::key_empty(KEY_LATEST_DIAMOND))
    }

    pub fn tx_exist(&self, hash: &Hash) -> Ret<Option<BlockHeight>> {
        self.get_value(&CoreState::key_with(KEY_TX_EXIST, hash))
    }

    pub fn balance(&self, addr: &Address) -> Ret<Option<Balance>> {
        self.get_value(&CoreState::key_with(KEY_BALANCE, addr))
    }

    pub fn diamond(&self, name: &DiamondName) -> Ret<Option<DiamondSto>> {
        self.get_value(&CoreState::key_with(KEY_DIAMOND, name))
    }

    pub fn diamond_name(&self, number: &DiamondNumber) -> Ret<Option<DiamondName>> {
        self.get_value(&CoreState::key_with(KEY_DIAMOND_NAME, number))
    }

    pub fn diamond_smelt(&self, name: &DiamondName) -> Ret<Option<DiamondSmelt>> {
        self.get_value(&CoreState::key_with(KEY_DIAMOND_SMELT, name))
    }

    pub fn diamond_owned(&self, addr: &Address) -> Ret<Option<DiamondOwnedForm>> {
        self.get_value(&CoreState::key_with(KEY_DIAMOND_OWNED, addr))
    }

    pub fn asset(&self, serial: &Fold64) -> Ret<Option<AssetSmelt>> {
        self.get_value(&CoreState::key_with(KEY_ASSET, serial))
    }
}

impl<'a> CoreState<'a> {
    pub fn wrap(layer: &'a mut dyn StateLayer) -> Self {
        Self(layer)
    }

    fn key_empty(idx: u8) -> Vec<u8> {
        numeric_state_empty_key(idx).to_vec()
    }

    fn key_with(idx: u8, key: &impl Encode) -> Vec<u8> {
        numeric_state_key(idx, key)
    }

    fn get_value<T: Decode>(&self, key: &[u8]) -> Ret<Option<T>> {
        get_typed(&*self.0, key)
    }

    fn set_value<T: Encode>(&mut self, key: Vec<u8>, val: &T) {
        self.0.set(&key, val.encode());
    }

    pub fn base_total(&self) -> Ret<Option<BaseTotal>> {
        self.get_value(&Self::key_empty(KEY_BASE_TOTAL))
    }

    pub fn get_base_total(&self) -> Ret<BaseTotal> {
        Ok(self.base_total()?.unwrap_or_default())
    }

    pub fn base_total_set(&mut self, v: &BaseTotal) {
        self.set_value(Self::key_empty(KEY_BASE_TOTAL), v);
    }

    pub fn set_base_total(&mut self, v: &BaseTotal) {
        self.base_total_set(v);
    }

    pub fn base_total_del(&mut self) {
        self.0.del(&Self::key_empty(KEY_BASE_TOTAL));
    }

    pub fn latest_diamond(&self) -> Ret<Option<DiamondSmelt>> {
        self.get_value(&Self::key_empty(KEY_LATEST_DIAMOND))
    }

    pub fn latest_diamond_set(&mut self, v: &DiamondSmelt) {
        self.set_value(Self::key_empty(KEY_LATEST_DIAMOND), v);
    }

    pub fn latest_diamond_del(&mut self) {
        self.0.del(&Self::key_empty(KEY_LATEST_DIAMOND));
    }

    pub fn tx_exist(&self, hash: &Hash) -> Ret<Option<BlockHeight>> {
        self.get_value(&Self::key_with(KEY_TX_EXIST, hash))
    }

    pub fn tx_exist_set(&mut self, hash: &Hash, v: &BlockHeight) {
        self.set_value(Self::key_with(KEY_TX_EXIST, hash), v);
    }

    pub fn tx_exist_del(&mut self, hash: &Hash) {
        self.0.del(&Self::key_with(KEY_TX_EXIST, hash));
    }

    pub fn balance(&self, addr: &Address) -> Ret<Option<Balance>> {
        self.get_value(&Self::key_with(KEY_BALANCE, addr))
    }

    pub fn balance_set(&mut self, addr: &Address, v: &Balance) {
        self.set_value(Self::key_with(KEY_BALANCE, addr), v);
    }

    pub fn balance_del(&mut self, addr: &Address) {
        self.0.del(&Self::key_with(KEY_BALANCE, addr));
    }

    pub fn diamond(&self, name: &DiamondName) -> Ret<Option<DiamondSto>> {
        self.get_value(&Self::key_with(KEY_DIAMOND, name))
    }

    pub fn diamond_set(&mut self, name: &DiamondName, v: &DiamondSto) {
        self.set_value(Self::key_with(KEY_DIAMOND, name), v);
    }

    pub fn diamond_del(&mut self, name: &DiamondName) {
        self.0.del(&Self::key_with(KEY_DIAMOND, name));
    }

    pub fn diamond_name(&self, number: &DiamondNumber) -> Ret<Option<DiamondName>> {
        self.get_value(&Self::key_with(KEY_DIAMOND_NAME, number))
    }

    pub fn diamond_name_set(&mut self, number: &DiamondNumber, v: &DiamondName) {
        self.set_value(Self::key_with(KEY_DIAMOND_NAME, number), v);
    }

    pub fn diamond_smelt(&self, name: &DiamondName) -> Ret<Option<DiamondSmelt>> {
        self.get_value(&Self::key_with(KEY_DIAMOND_SMELT, name))
    }

    pub fn diamond_smelt_set(&mut self, name: &DiamondName, v: &DiamondSmelt) {
        self.set_value(Self::key_with(KEY_DIAMOND_SMELT, name), v);
    }

    pub fn diamond_owned(&self, addr: &Address) -> Ret<Option<DiamondOwnedForm>> {
        self.get_value(&Self::key_with(KEY_DIAMOND_OWNED, addr))
    }

    pub fn diamond_owned_exist(&self, addr: &Address) -> Ret<bool> {
        Ok(self
            .get(&Self::key_with(KEY_DIAMOND_OWNED, addr))?
            .is_some())
    }

    pub fn diamond_owned_set(&mut self, addr: &Address, v: &DiamondOwnedForm) {
        self.set_value(Self::key_with(KEY_DIAMOND_OWNED, addr), v);
    }

    pub fn diamond_owned_del(&mut self, addr: &Address) {
        self.0.del(&Self::key_with(KEY_DIAMOND_OWNED, addr));
    }

    pub fn asset(&self, serial: &Fold64) -> Ret<Option<AssetSmelt>> {
        self.get_value(&Self::key_with(KEY_ASSET, serial))
    }

    pub fn asset_set(&mut self, serial: &Fold64, v: &AssetSmelt) {
        self.set_value(Self::key_with(KEY_ASSET, serial), v);
    }

    pub fn asset_del(&mut self, serial: &Fold64) {
        self.0.del(&Self::key_with(KEY_ASSET, serial));
    }

    fn get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        self.0.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StateChunkRef, StateLayer};

    struct NoDisk;
    impl crate::DiskDB for NoDisk {
        fn read(&self, _key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
            Ok(None)
        }
        fn save(&self, _key: &[u8], _val: &[u8]) {}
        fn remove(&self, _key: &[u8]) {}
        fn try_write(&self, _memkv: &dyn crate::MemDB) -> sys::Rerr {
            Ok(())
        }
    }

    fn test_block(height: u64) -> std::sync::Arc<dyn crate::Block> {
        std::sync::Arc::new(TestBlock { height })
    }

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
    }

    impl field::Encode for TestBlock {
        fn size(&self) -> usize {
            0
        }
        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }

    impl crate::Block for TestBlock {
        fn version(&self) -> u8 {
            1
        }
        fn height(&self) -> u64 {
            self.height
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
            self.height
        }
        fn transactions(&self) -> &[crate::TxRef] {
            &[]
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn root() -> StateChunkRef {
        StateChunkRef::new_root(std::sync::Arc::new(NoDisk), test_block(0))
    }

    /// Bytes read from persisted state that fail protocol decode must surface as
    /// `Abort + state_decode_failed`, never as a missing key (§3/§12.1).
    #[test]
    fn corrupted_persisted_bytes_decode_as_abort() {
        let root = root();
        let mut tx = StateChunkRef::tx_on(&root, Hash::default());
        let addr = Address::default();
        let key = CoreState::key_with(KEY_BALANCE, &addr);
        tx.set(&key, vec![0xff, 0x01, 0x02]);
        let read = CoreStateRead::wrap(&tx);
        let err = read.balance(&addr).unwrap_err();
        assert!(err.is_abort(), "state decode failure must be fatal");
        assert_eq!(err.code(), Some(crate::STATE_DECODE_FAILED_CODE));
    }

    /// A missing key reads as `Ok(None)` and only that.
    #[test]
    fn missing_key_is_ok_none() {
        let root = root();
        let tx = StateChunkRef::tx_on(&root, Hash::default());
        let read = CoreStateRead::wrap(&tx);
        assert!(read.balance(&Address::default()).unwrap().is_none());
    }
}
