
impl ChainEngine {

    
    fn do_insert(&self, block: BlockPkg) -> Rerr {
        let hx = block.hash.clone();
        let (r, c, d) = self.do_insert_chunk(block)?;
        self.do_roll_disk(r, c, d, hx)
    }

    fn do_insert_sync(&self, start_hei: u64, mut datas: Vec<u8>) -> Rerr {
        let cur_hei = self.latest_block().height().uint();
        if start_hei != cur_hei + 1 {
            return sync_warning(format!("need height {} but got {}", cur_hei+1, start_hei))
        }
        let this = self;
        // create thread
        let (chblk, chblkcv) = std::sync::mpsc::sync_channel(10);
        let (chrol, chrolcv) = std::sync::mpsc::sync_channel(4);
        let (cherr, cherrcv) = std::sync::mpsc::channel();
        let cherr1 = cherr.clone();
        let cherr2 = cherr.clone();
        std::thread::scope(|s| {
            // parse block
            s.spawn(move || {
                let mut hei = start_hei;
                let mut blocks = datas.as_mut_slice();
                loop {
                    if blocks.len() == 0 {
                        break
                    }
                    // println!("{}", hex::encode(&blocks[0..500]));
                    let blk = block::create(&blocks);
                    if let Err(e) = blk {
                        let _ = cherr1.send(format!("block {} parse error: {}", hei, e));
                        break
                    }
                    let (blk, sk) = blk.unwrap();
                    // println!("block::create() sk = {}", sk);
                    let blkhei = blk.height().uint();
                    // debug_println!("sync -> {}, tx: {}", blkhei, blk.transaction_count().uint()-1);
                    if hei != blkhei {
                        let _ = cherr1.send(format!("need block height {} but got {}", hei, blkhei));
                        break
                    }
                    let (left, right) = blocks.split_at_mut(sk);
                    let mut pkg = BlockPkg::new(blk, left.into());
                    pkg.set_origin( BlkOrigin::SYNC );
                    if let Err(..) = chblk.send(pkg) {
                        break // end
                    }
                    // next
                    blocks = right; // next chunk
                    hei += 1;
                }
            });
            // create chunk
            s.spawn(move || {
                loop {
                    let Ok(blk) = chblkcv.recv() else {
                        break // end
                    };
                    let hei = blk.objc.height().uint();
                    let hx = blk.hash.clone();
                    // debug_println!("sync insert height: {}, body: {}", hei, blk.data.hex());
                    let res = this.do_insert_chunk(blk);
                    if let Err(e) = res {
                        let _ = cherr2.send(format!("create chunk {} error: {}", hei, e));
                        break // end
                    }
                    let (r, c, d) = res.unwrap();
                    if let Err(..) = chrol.send((r, c, d, hx)) {
                        break // end
                    }
                }
            });
            // do roll
            loop {
                let Ok((r, c, d, hx)) = chrolcv.recv() else {
                    break // end
                };
                if let Err(e) = this.do_roll_disk(r, c, d, hx) {
                    let _ = cherr.send(format!("do roll error: {}", e));
                    break
                }
            }
            // ok finish
            let _ = cherr.send("".to_string());
        });
        // finish
        let err = cherrcv.recv().unwrap();
        if err.len() > 0 {
            let e = sync_warning(err);
            println!("{:?}", &e);
            return e
        }
        // ok
        Ok(())
    }


    /*************************/

    fn do_roll_disk(&self, root: Option<Arc<Chunk>>, cptr: Option<Arc<Chunk>>, data: Vec<u8>, hx: Hash) -> Rerr {
        let nrt = root.clone();
        let new_root_hei: u64 = match root {
            Some(rt) => {
                rt.state.write_to_disk(); // write state to disk
                rt.height
            },
            None => self.roller.lock().unwrap().root.height
        };
        let mut block_disk_batch = MemBatch::new();
        if let Some(curr) = cptr {
            block_disk_batch.put(&BlockStore::CSK.to_vec(), &ChainStatus{
                root_height: BlockHeight::from(new_root_hei),
                last_height: BlockHeight::from(curr.height),
            }.serialize());
        }
        // save block data to disk
        block_disk_batch.put(&hx.to_vec(), &data);
        // write all data by batch
        self.store.save_batch(block_disk_batch);
        // scaner do roll
        if let Some(new_root) = nrt {
            let scres = self.scaner.roll(new_root.block.clone(), new_root.state.clone(), self.disk.clone());
            if let Err(e) = scres {
                panic!("\n\nBlock scaner roll error: {}\n\n", e);
            }
        }
        Ok(())
    }

    // return chunk, data
    fn do_insert_chunk(&self, block: BlockPkg) -> Ret<(Option<Arc<Chunk>>, Option<Arc<Chunk>>, Vec<u8>)> {
        let hei = block.hein;
        let hx = block.hash;
        // find prev chunk
        let prev_hei = block.hein - 1;
        let prev_hx = block.objc.prevhash();
        let prev = { 
            self.roller.lock().unwrap().fast_search(prev_hei, prev_hx) 
        };
        let Some(prev_chunk) = prev else {
            return errf!("not find prev block <{}, {}>", prev_hei, prev_hx)
        };
        // create sub state 
        let prev_state = prev_chunk.state.clone();
        let mut sub_state = prev_state.fork_sub(Arc::downgrade(&prev_state));
        // initialize on first block
        if hei == 1 {
            self.minter.initialize(sub_state.as_mut())?;
        }
        // check
        let fast_sync = block.orgi==BlkOrigin::REBUILD || (block.orgi==BlkOrigin::SYNC && self.cnf.fast_sync);
        // println!("fast_sync = {}", fast_sync);
        if !fast_sync {
            // check repeat
            let brothers: Vec<Arc<Chunk>> = {
                prev_chunk.childs.lock().unwrap().iter().map(|a|a.clone()).collect()
            };
            for sub in brothers {
                if hx == sub.hash {
                    return errf!("repetitive block height {} hash {}", hei, hx)
                }
            }
            // check consensus
            self.check_all_for_insert(&block, prev_chunk.block.clone())?;
        }
        // exec block get state
        let sc = &self.cnf;
        let chaincnf = ctx::Chain {
            fast_sync,
            diamond_form: sc.diamond_form,
            id: sc.chain_id,
        };
        sub_state = block.objc.execute(chaincnf, sub_state)?;
        self.minter.examine(&block, sub_state.as_ref())?;
        // create chunk
        let (hx, objc, data) = block.apart();
        let chunk = Chunk::create(hx, objc.into(), sub_state.into());
        // insert chunk
        let (root, curr, path) = self.roller.lock().unwrap().insert(prev_chunk, chunk)?;
        self.store.save_batch(path);
        Ok((root, curr, data))
    }




}




fn sync_warning(e: String) -> Rerr {
    errf!("\n[Block Sync Warning] {}\n", e)
}






/*


01
0000000001
005c57b130
000000077790ba2fcdeaef4a4299d9b667135bac577ce204dee8388f1b97f7e6
4448ea1749d50416b41848e62edb30f8570153f80bd463f6b76de8d2948050f3
00000001
00000516
fffffffe
0000
00000c1fa1c032d90fd7afc54deb03941e87b4c59756
f80101
20202020202020202020202020202020
00

01
0000000002
005c57b2e6001e231cb03f9938d54f04407797b8188f0375eb10f0bcb426dccae87dcadb564448ea1749d50416b41848e62edb30f8570153f80bd463f6b76de8d2948050f300000001000007adfffffffe000000000c1fa1c032d90fd7afc54deb03941e87b4c59756f801012020202020202020202020202020202000010000000003005c57b3f3000c0a2a3761fec7aa214975c1cce407b509a828d16dcf6d3bdb1f612a2466f54448ea1749d50416b41848e62edb30f8570153f80bd463f6b76de8d2948050f3000000010000037afffffffe000000000c1fa1c032d90fd7afc54deb03941e87b4c59756f801012020202020202020202020202020202000010000000004005c57b52d0015920ecbd8048128b9e27a26bd08b488050c78b89291d740889ed4d785f4104448ea1749d50416b41848e62edb30f8570153f80bd463f6b76de8d2948050f30000000100000039fffffffe000000000c1fa1c032d90fd7afc54deb03941e87

*/



