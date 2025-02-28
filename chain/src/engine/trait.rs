
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

    fn fork_sub_state(&self) -> Box<dyn State> {
        let state = self.state();
        let sub_state = state.fork_sub(Arc::downgrade(&state));
        sub_state
    }
    
    fn store(&self) -> BlockStore {
        BlockStore::wrap(self.disk.clone())
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
        let an = tx.action_count().uint() as usize;
        if an != tx.actions().len() {
            return errf!("tx action count not match")
        }
        if an > cnf.max_tx_actions {
            return errf!("tx action count cannot more than {}", cnf.max_tx_actions)
        }
        if tx.size() as usize > cnf.max_tx_size{
            return errf!("tx size cannot more than {} bytes", cnf.max_tx_size)
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
        let sub = unsafe { Box::from_raw(sub_state.as_mut() as *mut dyn State) };
        let mut ctxobj = ctx::ContextInst::new(env, sub, tx);
        // do tx exec
        let exec_res = tx.execute(&mut ctxobj);
        // drop the box, back to mut ptr do manage
        let _ = Box::into_raw( ctxobj.into_state() ); 
        // return execute result
        exec_res
    }


    fn try_execute_tx(&self, tx: &dyn TransactionRead) -> Rerr {
        let height = self.latest_block().height().uint() + 1; // next height
        self.try_execute_tx_by(tx, height, &mut self.fork_sub_state())?;
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





    // for v2
    fn discover(&self, blk: BlockPkg) -> Rerr {
        // do lock
        loop {
            match self.inserting.compare_exchange(ISRT_STAT_IDLE, ISRT_STAT_DISCOVER, Ordering::Acquire, Ordering::Relaxed) {
                Ok(ISRT_STAT_IDLE) => break, // idle, go to insert
                Err(ISRT_STAT_DISCOVER) => {
                    sleep(Duration::from_millis(100)); // wait 0.1s
                    continue // to check again
                },
                Err(ISRT_STAT_SYNCING) => {
                    return errf!("the blockchain is syncing and cannot insert newly discovered block")
                }
                _ => never!()
            }
        }
        // get mut roller
        let mut roller = self.roller.lock().unwrap();
        let roller = roller.deref_mut();
        // do insert
        // search prev chunk in roller tree
        let hei = blk.hein;
        let hx = blk.hash;
        let prev_hei = hei - 1;
        let prev_hx  = blk.objc.prevhash();
        let Some(prev_chunk) = roller.search(prev_hei, prev_hx) else {
            return errf!("not find prev block <{}, {}>", prev_hei, prev_hx)
        };
        // check repeat
        if prev_chunk.childs.iter().any(|c|c.hash==hx) {
            return errf!("repetitive block <{}, {}>", hei, hx)
        }
        // minter verify
        self.minter.blk_verify(&blk, prev_chunk.block.as_read(), &self.store)?;
        self.block_verify(&blk, prev_chunk.block.clone())?;
        // try execute
        // create sub state 
        let prev_state = prev_chunk.state.clone();
        let mut sub_state = prev_state.fork_sub(Arc::downgrade(&prev_state));
        // initialize on first block
        if hei == 1 {
            self.minter.initialize(sub_state.as_mut())?;
        }
        let c = &self.cnf;
        let chain_option = ctx::Chain {
            fast_sync: false,
            diamond_form: c.diamond_form,
            id: c.chain_id,
        };
        // execute block
        sub_state = blk.objc.execute(chain_option, sub_state)?;
        // create chunk
        let (hx, objc, data) = blk.apart();
        let chunk = Chunk::create(hx, objc.into(), sub_state.into());
        // insert chunk
        let (root, head, path) = roller.insert(prev_chunk, chunk)?;
        let mut store_batch = path;
        let mut state_write: Option<Arc<dyn State>> = None;
        // Ok((root, curr, data))
        let new_root_hei: u64 = match root.clone() {
            Some(rt) => {
                state_write = Some(rt.state.clone()); // write
                rt.height
            },
            None => roller.root.height
        };
        if let Some(head) = head {
            store_batch.put(&BlockStore::CSK.to_vec(), &ChainStatus{
                root_height: BlockHeight::from(new_root_hei),
                last_height: BlockHeight::from(head.height),
            }.serialize());
        }
        // save block data to disk
        store_batch.put(&hx.to_vec(), &data);
        // scaner do roll
        if let Some(new_root) = root {
            let scres = self.scaner.roll(new_root.block.clone(), new_root.state.clone(), self.disk.clone());
            if let Err(e) = scres {
                panic!("\n\nBlock scaner roll error: {}\n\n", e);
            }
        }
        // write all data to disk
        if let Some(sw) = state_write {
            sw.write_to_disk();
        };
        self.store.save_batch(store_batch);
        // insert success
        Ok(())
    }


    fn synchronize(&self, _: Vec<u8>) -> Rerr {
        // do lock
        loop {
            match self.inserting.compare_exchange(ISRT_STAT_IDLE, ISRT_STAT_SYNCING, Ordering::Acquire, Ordering::Relaxed) {
                Ok(ISRT_STAT_IDLE) => break, // idle, go to sync
                Err(ISRT_STAT_DISCOVER) => {
                    sleep(Duration::from_millis(100)); // wait 0.1s
                    continue // to check again
                },
                Err(ISRT_STAT_SYNCING) => {
                    return errf!("the blockchain is syncing and need wait")
                }
                _ => never!()
            }
        }
        // do sync
        let _roller = self.roller.lock().unwrap().deref_mut();
    
    
    
    
        unimplemented!()
    }





}
