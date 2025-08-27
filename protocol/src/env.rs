

#[derive(Default, Clone)]
pub struct ChainInfo {
    pub id: u32,
    pub fast_sync: bool,
    pub diamond_form: bool,
}


#[derive(Default, Clone)]
pub struct BlkInfo {
    pub height: u64,
    pub hash: Hash,
    pub coinbase: Address,
}



#[derive(Default, Clone)]
pub struct TxInfo {
    pub ty: u8,
    pub fee: Amount,
    pub main: Address,
    pub addrs: Vec<Address>,
}



#[derive(Default, Clone)]
pub struct Env {
    pub chain: ChainInfo,
    pub block: BlkInfo,
    pub tx: TxInfo,
}


impl Env {
    // return old tx
    pub fn replace_tx(&mut self, tx: TxInfo) -> TxInfo {
        std::mem::replace(&mut self.tx, tx)
    }
}


/*
pub struct Context {
    pub env: Env,
}
*/










