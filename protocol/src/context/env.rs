

#[derive(Default, Clone)]
pub struct Chain {
    pub id: u32,
    pub fast_sync: bool,
    pub diamond_form: bool,
}


#[derive(Default, Clone)]
pub struct Block {
    pub height: u64,
    pub hash: Hash
}



#[derive(Default, Clone)]
pub struct Tx {
    // pub version: u8,
    pub fee: Amount,
    pub main: Address,
    pub addrs: Vec<Address>,
}

impl Tx {
    pub fn create(tx: &dyn TransactionRead) -> Self {
        Self {
            main: tx.main(),
            addrs: tx.addrs(),
            fee: tx.fee_pay(),
        }
    }
}


#[derive(Default, Clone)]
pub struct Env {
    pub chain: Chain,
    pub block: Block,
    pub tx: Tx,
}


/*
pub struct Context {
    pub env: Env,
}
*/










