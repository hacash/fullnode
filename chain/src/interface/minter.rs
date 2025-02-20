

pub trait Minter : Send + Sync {
    fn config(&self) -> Box<dyn Any> { unimplemented!() }
    fn init(&self, _:&IniObj) {}
    // fn config(&self) -> &MintConf;
    fn next_difficulty(&self, _: &dyn BlockRead, _: &BlockDisk) -> u32 { u32::MAX }
    // check
    fn tx_check(&self, _: &dyn TransactionRead, _: u64) -> Rerr { Ok(()) }
    fn prepare(&self, _: &dyn BlockRead, _: &BlockDisk) -> Rerr { Ok(()) }
    fn consensus(&self, _: &dyn BlockRead, _: &dyn BlockRead, _: &dyn State, _: &BlockDisk, _: BlkOrigin) -> Rerr {  Ok(())  }
    fn coinbase(&self, _: u64, _: &dyn Transaction) -> Rerr { Ok(()) }
    // do
    fn initialize(&self, _: &mut dyn State) -> Rerr { Ok(()) }
    // data
    fn genesis_block(&self) -> Arc<dyn Block> { unimplemented!() }
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;
    fn exit(&self) {}
}



