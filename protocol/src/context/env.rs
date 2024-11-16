

#[derive(Default, Clone)]
pub struct Chain {
    pub id: u64
}


#[derive(Default, Clone)]
pub struct Block {
    pub height: u64,
    pub hash: Hash
}



#[derive(Default, Clone)]
pub struct Tx {
    // pub version: u8,
    pub main: Address,
    pub addrs: Vec<Address>,
    // pub fee: Uint8,
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










