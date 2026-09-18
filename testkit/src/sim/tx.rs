use std::any::Any;

use base::{Context, Transaction, TransactionExecute, TransactionSign};
use field::{Address, Amount, Encode, Hash};
use sys::Rerr;

#[derive(Default, Clone, Debug)]
pub struct DummyTx;

impl Encode for DummyTx {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}
impl Transaction for DummyTx {
    fn ty(&self) -> u8 {
        3
    }
    fn main(&self) -> Address {
        Address::default()
    }
    fn fee(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl TransactionSign for DummyTx {
    fn hash(&self) -> Hash {
        Hash::default()
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn TransactionExecute> {
        Some(self)
    }
}
impl TransactionExecute for DummyTx {
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct StubTx {
    pub ty: u8,
    pub hash: Hash,
    pub main: Address,
    pub addrs: Vec<Address>,
    pub fee: Amount,
    pub gas_max: u8,
    pub tx_size: usize,
    pub fee_purity: u64,
}

impl Default for StubTx {
    fn default() -> Self {
        Self {
            ty: 3,
            hash: Hash::default(),
            main: Address::default(),
            addrs: vec![Address::default()],
            fee: Amount::unit238(10_000_000),
            gas_max: 17,
            tx_size: 128,
            fee_purity: 3_200,
        }
    }
}
impl Encode for StubTx {
    fn size(&self) -> usize {
        self.tx_size
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        out.resize(out.len() + self.tx_size, 0);
    }
}
impl Transaction for StubTx {
    fn ty(&self) -> u8 {
        self.ty
    }
    fn main(&self) -> Address {
        self.main
    }
    fn addrs(&self) -> Vec<Address> {
        self.addrs.clone()
    }
    fn fee(&self) -> &Amount {
        &self.fee
    }
    fn fee_purity(&self) -> u64 {
        self.fee_purity
    }
    fn billing_size(&self) -> sys::Ret<usize> {
        Ok(self.tx_size)
    }
    fn gas_max_byte(&self) -> Option<u8> {
        (self.ty >= 3).then_some(self.gas_max)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl TransactionSign for StubTx {
    fn hash(&self) -> Hash {
        self.hash
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn TransactionExecute> {
        Some(self)
    }
}
impl TransactionExecute for StubTx {
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct StubTxBuilder {
    tx: StubTx,
}
impl StubTxBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn ty(mut self, value: u8) -> Self {
        self.tx.ty = value;
        self
    }
    pub fn hash(mut self, value: Hash) -> Self {
        self.tx.hash = value;
        self
    }
    pub fn main(mut self, value: Address) -> Self {
        self.tx.main = value;
        self
    }
    pub fn addrs(mut self, value: Vec<Address>) -> Self {
        self.tx.addrs = value;
        self
    }
    pub fn fee(mut self, value: Amount) -> Self {
        self.tx.fee = value;
        self
    }
    pub fn gas_max(mut self, value: u8) -> Self {
        self.tx.gas_max = value;
        self
    }
    pub fn tx_size(mut self, value: usize) -> Self {
        self.tx.tx_size = value;
        self
    }
    pub fn fee_purity(mut self, value: u64) -> Self {
        self.tx.fee_purity = value;
        self
    }
    pub fn build(self) -> StubTx {
        self.tx
    }
}
