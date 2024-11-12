use field::*;




pub struct Chain {
    pub id: u64
}


pub struct Block {
    pub height: u64,
    pub hash: Hash
}



pub struct Tx {
    pub version: u8,
    pub main: Address,
    pub addrs: Vec<Address>,
    pub fee: Uint8,
}


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










