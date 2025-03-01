


/*******************************************************/


pub trait Minter : Send + Sync {
    fn config(&self) -> Box<dyn Any> { unimplemented!() }
    fn init(&self, _:&IniObj) {}
    // fn config(&self) -> &MintConf;
    fn next_difficulty(&self, _: &dyn BlockRead, _: &BlockStore) -> u32 { u32::MAX }
    // tx check
    // block check
    // 
    fn coinbase(&self, _: u64, _: &dyn TransactionRead) -> Rerr { Ok(()) }
    // do
    fn initialize(&self, _: &mut dyn State) -> Rerr { Ok(()) }
    // data
    fn genesis_block(&self) -> Arc<dyn Block> { unimplemented!() }
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;
    fn exit(&self) {}



    // v2

    // check
    fn tx_submit(&self, _: &dyn EngineRead, _: &dyn TransactionRead) -> Rerr { Ok(()) }
    fn blk_found(&self, _: &dyn BlockRead, _: &BlockStore) -> Rerr { Ok(()) }
    fn blk_verify(&self, _: &dyn BlockRead, _prev: &dyn BlockRead, _: &BlockStore) -> Rerr { Ok(()) }
    fn blk_insert(&self, _: &BlockPkg, _sub: &dyn State, _prev: &dyn State) -> Rerr { Ok(()) }
    // 
    // create block
    fn block_reward(&self, _: u64) -> u64 { 0 }
    fn packing_next_block(&self, _: &dyn TxPool) -> Box<dyn Block> { unimplemented!() }

}



