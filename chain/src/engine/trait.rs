
struct InsertingLock<'a> {
    mark: &'a AtomicUsize,
}

impl InsertingLock<'_> {
    fn finish(self) {}
}

impl Drop for InsertingLock<'_> {
    fn drop(&mut self) {
        self.mark.store(ISRT_STAT_IDLE, Ordering::Relaxed);
        // println!("---- InsertingLock HasDrop!");
    }
}


macro_rules! inserting_lock {
    ($self:ident, $change_to_stat:expr, $busy_tip:expr) => {
        {
            loop {
                match $self.inserting.compare_exchange(ISRT_STAT_IDLE, $change_to_stat, Ordering::Acquire, Ordering::Relaxed) {
                    Ok(ISRT_STAT_IDLE) => break, // ok idle, go to insert or sync
                    Err(ISRT_STAT_DISCOVER) => {
                        sleep(Duration::from_millis(100)); // wait 0.1s
                        continue // to check again
                    },
                    Err(ISRT_STAT_SYNCING) => {
                        return errf!($busy_tip)
                    }
                    _ => never!()
                }
            };
            InsertingLock{ mark: &$self.inserting }
        }
    }
}





impl EngineRead for ChainEngine {

    
    fn config(&self) -> &EngineConf {
        &self.cnf
    }

    
    fn latest_block(&self) -> Arc<dyn Block> {
        self.roller.lock().unwrap().head.upgrade().unwrap().block.clone()
    }

    
    fn mint_checker(&self) -> &dyn Minter {
        self.minter.as_ref()
    }

    
    fn state(&self) -> Arc<dyn State> {
        self.roller.lock().unwrap().head.upgrade().unwrap().state.clone()
    }

    fn fork_sub_state(&self) -> Box<dyn State> {
        let state = self.state();
        let sub_state = state.fork_sub(Arc::downgrade(&state));
        sub_state
    }
    
    fn store(&self) -> Arc::<dyn Store> {
        Arc::new(BlockStore::wrap(self.disk.clone()))
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
        // average
        let ttn: u64 = avgfs.iter().sum();
        ttn / avgfs.len() as u64
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
        let env = protocol::Env {
            chain: ChainInfo {
                id: self.cnf.chain_id,
                diamond_form: false,
                fast_sync: false,
            },
            block: BlkInfo {
                height: pd_hei,
                hash,
                coinbase: Address::default(),
            },
            tx: create_tx_info(tx),
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
    
    fn exit(&self) {
        // wait block insert finish
        let lk = self.isrtlk.lock().unwrap();
        self.minter.exit();
        self.scaner.exit();
        drop(lk);
    }



    /******** for v2  ********/




    fn discover(&self, blk: BlockPkg) -> Rerr {
        // deal recent_blocks and average_fee_purity
        let blkobj = blk.objc.as_read();
        if self.cnf.recent_blocks {
            self.record_recent(blkobj);
        }
        if self.cnf.average_fee_purity {
            self.record_avgfee(blkobj);
        }
        // lock and wait
        let isrtlock = inserting_lock!( self, ISRT_STAT_DISCOVER, 
            "the blockchain is syncing and cannot insert newly discovered block"
        );
        // get mut roller
        let mut roller = self.roller.lock().unwrap();
        // do insert adnd rool
        let rid = self.insert_by(roller.deref_mut(), blk)?;
        self.roll_by(rid)?; // roll to write disk
        // insert success
        isrtlock.finish(); // unlock
        Ok(())
    }


    fn synchronize(&self, mut datas: Vec<u8>) -> Rerr {
        // lock and wait
        let isrtlock = inserting_lock!( self, ISRT_STAT_SYNCING, 
            "the blockchain is syncing and need wait"
        );
        // roller
        let mut temp = self.roller.lock().unwrap();
        let roller = temp.deref_mut();
        let latest_hei = roller.last_height();
        // check hei limit
        let hei_up = latest_hei + 1;
        let ubh = self.cnf.unstable_block;
        let hei_lo = maybe!(latest_hei>ubh, latest_hei-ubh+1, 1);
        // check start height
        let Ok(blkhei) = BlockHeadOnlyHeight::build(&datas) else {
            return sync_warning("block data format error".to_string())
        };
        let insert_start_hei = *blkhei.height;
        if insert_start_hei < hei_lo || insert_start_hei > hei_up {
            return sync_warning(format!("height need between {} and {} but got {}", hei_lo, hei_up, insert_start_hei))
        }
        // build block
        let tree_isr = hei_up - insert_start_hei;
        for _ in 0..tree_isr  {
            if datas.len() == 0 {
                break // end
            }
            let seek;
            let blkobj = match block::create(&datas) {
                Ok((b, s)) => {seek = s; b},
                Err(e) => return errf!("block parse error: {}", e)
            };
            let right = datas.split_off(seek);
            // try insert
            let rid = self.insert_by(roller, BlockPkg::new(blkobj, datas))?;
            self.roll_by(rid)?;
            // next
            datas = right;
        }
        // sync all by head
        // error channel
        let (errch, errcv) = std::sync::mpsc::channel();
        let errch1 = errch.clone();
        let errch2 = errch.clone();
        // data channel
        let (blkch, blkcv) = std::sync::mpsc::sync_channel(20);
        let (ridch, ridcv) = std::sync::mpsc::sync_channel(8);
        let mut need_blk_hei = roller.last_height() + 1;
        let mut blockdts = datas.as_mut_slice();
        // thread
        std::thread::scope(|s| {
            // parse block
            s.spawn(move || { loop {
                // println!("--------- need_blk_hei {} datas len={}", need_blk_hei, datas.len());
                if blockdts.len() == 0 { break } // finish
                let seek;
                let blkobj = match block::create(&blockdts) {
                    Ok((b, s)) => {seek = s; b},
                    Err(e) => {
                        errch.send(format!("block parse error: {}", e)).unwrap();
                        break // err end
                    }
                };
                let (left, right) = blockdts.split_at_mut(seek);
                let mut pkg = BlockPkg::new(blkobj, left.into());
                pkg.set_origin( BlkOrigin::Sync );
                if pkg.hein != need_blk_hei {
                    let _ = errch.send(format!("need block height {} but got {}", need_blk_hei, pkg.hein));
                    break // err end
                }
                if let Err(..) = blkch.send(pkg) { break }
                // next block
                blockdts = right; // next chunk
                need_blk_hei += 1 ;
            }});
            // do insert
            s.spawn(move || { loop {
                let Ok(blk) = blkcv.recv() else { break };
                let hei = blk.hein;
                let rid = match self.insert_by(roller, blk) {
                    Err(e) => {
                        let _ = errch1.send(format!("insert {} error: {}", hei, e));
                        break
                    },
                    Ok(r) => r,
                };
                if let Err(..) = ridch.send(rid) { break }
            }});
            // do roll
            loop {
                let Ok(rid) = ridcv.recv() else { break };
                if let Err(e) = self.roll_by(rid) {
                    let _ = errch2.send(format!("do roll error: {}", e));
                    break
                }
            }
            let _ = errch2.send("".to_string());
        });
        // may be have error
        let e: String = errcv.recv().unwrap();
        if e.len() > 0 {
            return sync_warning(e)
        }
        // ok finish
        isrtlock.finish();
        Ok(())
    }





}
