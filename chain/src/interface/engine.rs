
pub trait EngineRead: Send + Sync {
    // key is height or hash
    // fn block(&self, _: &dyn Serialize) -> Option<Box<dyn BlockPkg>> { unimplemented!() }
    // key is hash
    // fn tx(&self, _: &dyn Serialize) -> Option<Box<dyn TxPkg>> { unimplemented!() }
    fn config(&self) -> &EngineConf { unimplemented!() }

    fn state(&self) -> Arc<dyn State> { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }

    // fn confirm_state(&self) -> (Arc<dyn State>, Arc<dyn BlockPkg>) { unimplemented!() }
    fn latest_block(&self) -> Arc<dyn Block> { unimplemented!() }
    fn mint_checker(&self) -> Arc<dyn Minter> { unimplemented!() }

    fn recent_blocks(&self) -> Vec<Arc<RecentBlockInfo>> { unimplemented!() }
    fn average_fee_purity(&self) -> u64 { 0 } // 1w zhu(shuo) / 200byte(1trs)

    fn try_execute_tx(&self, _: &dyn TransactionRead) -> Rerr { unimplemented!() }
    // realtime average fee purity
    // fn avgfee(&self) -> u32 { 0 }
}

pub trait Engine : EngineRead + Send + Sync {
    fn as_read(&self) -> &dyn EngineRead { unimplemented!() }
    // fn init(&self, _: &IniObj) -> Option<Error> { unimplemented!() }
    // fn start(&self) -> Option<Error> { unimplemented!() }
    fn insert(&self, _: BlockPkg) -> Rerr { unimplemented!() }
    fn insert_sync(&self, _: u64, _: Vec<u8>) -> Rerr { unimplemented!() }
}


