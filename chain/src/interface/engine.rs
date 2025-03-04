
pub trait EngineRead: Send + Sync {
    // key is height or hash
    // fn block(&self, _: &dyn Serialize) -> Option<Box<dyn BlockPkg>>;
    // key is hash
    // fn tx(&self, _: &dyn Serialize) -> Option<Box<dyn TxPkg>>;
    fn config(&self) -> &EngineConf;

    fn state(&self) -> Arc<dyn State>;
    fn fork_sub_state(&self) -> Box<dyn State>;
    fn store(&self) -> BlockStore;

    // fn confirm_state(&self) -> (Arc<dyn State>, Arc<dyn BlockPkg>);
    fn latest_block(&self) -> Arc<dyn Block>;
    fn mint_checker(&self) -> &dyn Minter { never!() }

    fn recent_blocks(&self) -> Vec<Arc<RecentBlockInfo>> { Vec::new() }
    fn average_fee_purity(&self) -> u64 { 0 } // 100:238 / 166byte(1trs)

    fn try_execute_tx(&self, _: &dyn TransactionRead) -> Rerr;
    fn try_execute_tx_by(&self, _: &dyn TransactionRead, _: u64, _: &mut Box<dyn State>) -> Rerr;
    // realtime average fee purity
    // fn avgfee(&self) -> u32 { 0 }
}

pub trait Engine : EngineRead + Send + Sync {
    fn as_read(&self) -> &dyn EngineRead;
    // fn init(&self, _: &IniObj) -> Option<Error>;
    // fn start(&self) -> Option<Error>;
    // fn insert(&self, _: BlockPkg) -> Rerr;
    // fn insert_sync(&self, _: u64, _: Vec<u8>) -> Rerr;

    // for v2
    fn discover(&self, _: BlockPkg) -> Rerr;
    fn synchronize(&self, _: Vec<u8>) -> Rerr;

    fn exit(&self) {}
}


