


/*******************************************************/


pub trait Minter : Send + Sync {
    // fn config(&self) -> &MintConf;
    // fn next_difficulty(&self, _: &dyn BlockRead, _: &BlockStore) -> u32 { u32::MAX }
    // tx check
    // block check
    // 
    // fn coinbase(&self, _: u64, _: &dyn TransactionRead) -> Rerr { Ok(()) }
    // do
    // data
    fn genesis_block(&self) -> Arc<dyn Block> { never!() }
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;

    // v2

    fn config(&self) -> Box<dyn Any> { never!() }
    // check
    fn initialize(&self, _: &mut dyn State) -> Rerr { Ok(()) }
    fn tx_submit(&self, _: &dyn EngineRead, _: &TxPkg) -> Rerr { Ok(()) }
    fn blk_found(&self, _: &dyn BlockRead, _: &dyn Store) -> Rerr { Ok(()) }
    fn blk_verify(&self, _: &dyn BlockRead, _prev: &dyn BlockRead, _: &dyn Store) -> Rerr { Ok(()) }
    fn blk_insert(&self, _: &BlockPkg, _sub: &dyn State, _prev: &dyn State) -> Rerr { Ok(()) }
    // 
    // create block
    fn block_reward(&self, _: u64) -> u64 { 0 }
    fn packing_next_block(&self, _: &dyn EngineRead, _: &dyn TxPool) -> Box<dyn Any> { never!() } // BlockV1
    fn tx_pool_group(&self, _: &TxPkg) -> usize { 0 }
    fn tx_pool_refresh(&self, _: &dyn EngineRead, _: &dyn TxPool, _txs: Vec<Hash>, _blkhei: u64) {}
    
    // close exit
    fn exit(&self) {}

}



