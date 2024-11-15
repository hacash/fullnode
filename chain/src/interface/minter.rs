

pub trait Minter : Send + Sync + DynClone {
    // fn config(&self) -> &MintConf;
    // fn next_difficulty(&self, _: &dyn BlockRead, _: &dyn Store) -> u32;
    // check
    fn prepare(&self, _: &dyn DiskDB, _: &dyn BlockRead) -> Rerr;
    fn consensus(&self, _: &dyn DiskDB, _: &dyn BlockRead, _: &dyn BlockRead) -> Rerr;
    fn coinbase(&self, _: u64, _: &dyn Transaction) -> Rerr;
    // do
    fn initialize(&self, _: &mut dyn State) -> Rerr;
    // data
    fn genesis(&self) -> Arc<dyn Block>;
    fn genesis_block(&self) -> Box<dyn Block>;
    // actions
    // fn actions(&self) -> Vec<Box<dyn Action>>;
}

clone_trait_object!(Minter);
