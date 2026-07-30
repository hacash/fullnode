use field::{Address, Amount, Hash};

#[derive(Clone, Debug, Default)]
pub struct RecentBlock {
    pub height: u64,
    pub hash: Hash,
    pub prev: Hash,
    pub txs: u32,
    pub miner: Address,
    pub message: String,
    pub reward: Amount,
    pub timestamp: u64,
    pub arrive: u64,
}
