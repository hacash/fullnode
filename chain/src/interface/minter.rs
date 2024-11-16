

pub trait Minter : Send + Sync {
    fn init(&self, _:&IniObj) {}
    // fn config(&self) -> &MintConf;
    // fn next_difficulty(&self, _: &dyn BlockRead, _: &dyn Store) -> u32;
    // check
    fn prepare(&self, _: &dyn DiskDB, _: &dyn BlockRead) -> Rerr { Ok(()) }
    fn consensus(&self, _: &dyn DiskDB, _: &dyn BlockRead, _: &dyn BlockRead) -> Rerr {  Ok(())  }
    fn coinbase(&self, _: u64, _: &dyn Transaction) -> Rerr { Ok(()) }
    // do
    fn initialize(&self, _: &mut dyn State) -> Rerr { Ok(()) }
    // data
    fn genesis(&self) -> Arc<dyn Block> { unimplemented!() }
    fn genesis_block(&self) -> Box<dyn Block> { unimplemented!() }
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;
}



