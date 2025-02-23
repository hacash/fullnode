

pub trait MinterV2 : Send + Sync {
    fn config(&self) -> Box<dyn Any> { unimplemented!() }
    fn init(&self, _:&IniObj) {}
    // check
    fn tx_submit(&self, _: &impl EngineRead, _: &impl TransactionRead) -> Rerr { Ok(()) }
    fn blk_found(&self, _: &impl EngineRead, _prev: &impl BlockRead, _: &BlockPkg) -> Rerr { Ok(()) }
    fn blk_insert(&self, _: &impl EngineRead, _: &BlockPkg) -> Rerr { Ok(()) }
    // 
    fn initialize(&self, _: &mut impl State) -> Rerr { Ok(()) }
    // create block
    fn block_reward(&self, _: u64) -> u64 { 0 }
    fn genesis_block(&self) -> Arc<dyn Block> { unimplemented!() }
    fn packing_next_block(&self, _: &dyn TxPool) -> Box<dyn Block> { unimplemented!() }
}


/*******************************************************/


pub trait Minter : Send + Sync {
    fn config(&self) -> Box<dyn Any> { unimplemented!() }
    fn init(&self, _:&IniObj) {}
    // fn config(&self) -> &MintConf;
    fn next_difficulty(&self, _: &dyn BlockRead, _: &BlockStore) -> u32 { u32::MAX }
    // tx check
    fn tx_check(&self, _: &dyn TransactionRead, _: u64) -> Rerr { Ok(()) }
    // block check
    fn prepare(&self, _: &dyn BlockRead, _: &BlockStore) -> Rerr { Ok(()) }
    fn consensus(&self, _: &dyn BlockRead, _: &dyn BlockRead, _: &BlockStore) -> Rerr {  Ok(())  }
    fn examine(&self, _: &BlockPkg, _: &dyn State) -> Rerr {  Ok(())  }
    // 
    fn coinbase(&self, _: u64, _: &dyn Transaction) -> Rerr { Ok(()) }
    // do
    fn initialize(&self, _: &mut dyn State) -> Rerr { Ok(()) }
    // data
    fn genesis_block(&self) -> Arc<dyn Block> { unimplemented!() }
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;
    fn exit(&self) {}
}



