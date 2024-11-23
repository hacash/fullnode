
impl EngineRead for ChainEngine {

    fn config(&self) -> &EngineConf {
        &self.cnf
    }

    fn latest_block(&self) -> Arc<dyn Block> {
        self.roller.read().unwrap().curr.upgrade().unwrap().block.clone()
    }

    fn mint_checker(&self) -> &dyn Minter {
        self.minter.as_ref()
    }

    fn state(&self) -> Arc<dyn State> {
        self.roller.read().unwrap().curr.upgrade().unwrap().state.clone()
    }

    fn disk(&self) -> Arc<dyn DiskDB> {
        self.disk.clone()
    }

    fn recent_blocks(&self) -> Vec<Arc<RecentBlockInfo>> { unimplemented!() }
    fn average_fee_purity(&self) -> u64 { 0 } // 1w zhu(shuo) / 200byte(1trs)

    fn try_execute_tx(&self, _: &dyn TransactionRead) -> Rerr { unimplemented!() }
    // realtime average fee purity
    // fn avgfee(&self) -> u32 { 0 }
}



impl Engine for ChainEngine {
    
    fn as_read(&self) -> &dyn EngineRead {
        self
    }

    fn insert(&self, blk: BlockPkg) -> Rerr {
        let lk = self.isrtlk.lock().unwrap();
        self.do_insert(blk)?;
        drop(lk);
        Ok(())
    }

    fn insert_sync(&self, hei: u64, data: Vec<u8>) -> Rerr {
        let lk = self.isrtlk.lock().unwrap();
        self.do_insert_sync(hei, data)?;
        drop(lk);
        Ok(())
    }

    fn exit(&self) {
        // wait block insert finish
        let lk = self.isrtlk.lock().unwrap();
        self.minter.exit();
        self.scaner.exit();
        drop(lk);
    }

}
