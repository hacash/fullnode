use field::{Address, Amount, Hash};

use crate::ChainId;

#[derive(Clone, Default, Debug)]
pub struct Env {
    pub chain: ChainInfo,
    pub block: BlockInfo,
    pub tx: TxInfo,
}

impl Env {
    pub fn replace_tx(&mut self, tx: TxInfo) -> TxInfo {
        std::mem::replace(&mut self.tx, tx)
    }
}

#[derive(Clone, Default, Debug)]
pub struct ChainInfo {
    /// id 0L2/ 0
    pub id: ChainId,
    pub fast_sync: bool,
    /// consensus-defined flag bits. base/chain only carry them; applications pick the bit
    /// assignments in the execution profile, so business flags never leak into the core.
    pub consensus_flags: u64,
}

#[derive(Clone, Default, Debug)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: Hash,
    pub author: Address,
}

#[derive(Clone, Default, Debug)]
pub struct TxInfo {
    pub ty: u8,
    pub main: Address,
    /// for AddrOrPtr::Ptr
    pub addrs: Vec<Address>,
    pub fee: Amount,
}

impl TxInfo {
    pub fn swap_addrs(&mut self, other: &mut Vec<Address>) {
        std::mem::swap(&mut self.addrs, other);
    }
}
