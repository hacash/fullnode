
impl EngineRead for ChainEngine {

    
    fn config(&self) -> &EngineConf {
        &self.cnf
    }

    
    fn latest_block(&self) -> Arc<dyn Block> {
        self.roller.lock().unwrap().curr.upgrade().unwrap().block.clone()
    }

    
    fn mint_checker(&self) -> &dyn Minter {
        self.minter.as_ref()
    }

    
    fn state(&self) -> Arc<dyn State> {
        self.roller.lock().unwrap().curr.upgrade().unwrap().state.clone()
    }

    fn sub_state(&self) -> Box<dyn State> {
        let state = self.state();
        let sub_state = state.fork_sub(state.clone());
        sub_state
    }
    
    fn disk(&self) -> Arc<dyn DiskDB> {
        self.disk.clone()
    }

    fn recent_blocks(&self) -> Vec<Arc<RecentBlockInfo>> {
        let vs = self.rctblks.lock().unwrap();   
        let res: Vec<_> = vs.iter().map(|x|x.clone()).collect();
        res
    }

    // 1w zhu(shuo) / 200byte(1trs)
    fn average_fee_purity(&self) -> u64 {
        let avgfs = self.avgfees.lock().unwrap();
        let al = avgfs.len();
        if al == 0 {
            return self.cnf.lowest_fee_purity
        }
        let mut allfps = 0u64;
        for a in avgfs.iter() {
            allfps += a;
        }
        allfps / al as u64
    } 

    fn try_execute_tx_by(&self, tx: &dyn TransactionRead, pd_hei: u64, sub_state: &mut Box<dyn State>) -> Rerr {
        // check
        let cnf = &self.cnf;
        if tx.ty() == TransactionCoinbase::TYPE {
            return errf!("cannot submit coinbase tx");
        }
        if tx.action_count().uint() as usize > cnf.max_tx_actions {
            return errf!("tx action count cannot more than {}", cnf.max_tx_actions);
        }
        if tx.size() as usize > cnf.max_tx_size{
            return errf!("tx size cannot more than {} bytes", cnf.max_tx_size);
        }
        // check time        
        let cur_time = curtimes();
        if tx.timestamp().uint() > cur_time {
            return errf!("tx timestamp {} cannot more than now {}", tx.timestamp(), cur_time)
        }
        // execute
        let hash = Hash::from([0u8; 32]); // empty hash
        // ctx
        let env = ctx::Env{
            chain: ctx::Chain{
                id: self.cnf.chain_id,
                diamond_form: false,
                fast_sync: false,
            },
            block: ctx::Block{
                height: pd_hei,
                hash,
                coinbase: Address::default(),
            },
            tx: ctx::Tx::create(tx),
        };
        // cast mut to box
        let sub = unsafe { Box::from_raw(sub_state.as_mut() as *mut (dyn State +'_)) };
        let mut ctxobj = ctx::ContextInst::new(env, sub, tx);
        // do tx exec
        tx.execute(&mut ctxobj)?;
        // minter check
        self.minter.tx_check(tx, pd_hei)?;
        // ok
        Ok(())
    }


    fn try_execute_tx(&self, tx: &dyn TransactionRead) -> Rerr {
        let height = self.latest_block().height().uint() + 1; // next height
        self.try_execute_tx_by(tx, height, &mut self.sub_state())?;
        Ok(())
    }
    
}



impl Engine for ChainEngine {
    
    fn as_read(&self) -> &dyn EngineRead {
        self
    }

    fn insert(&self, blk: BlockPkg) -> Rerr {
        let blkobj = blk.objc.as_read();
        if self.cnf.recent_blocks {
            self.record_recent(blkobj);
        }
        if self.cnf.average_fee_purity {
            self.record_avgfee(blkobj);
        }
        // do insert
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
