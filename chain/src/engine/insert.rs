
impl ChainEngine {

    fn do_insert(&self, block: BlockPkg) -> Rerr {
        let hx = block.hash.clone();
        let (r, p, d) = self.do_insert_chunk(block)?;
        self.do_roll_disk(r, p, hx, d)
    }

    fn do_roll_disk(&self, root: Option<Arc<Chunk>>, mut pointer: Option<Arc<Chunk>>, hx: Hash, data: Vec<u8>) -> Rerr {
        // write state to disk
        if let Some(root) = root {
            root.state.write_to_disk();
        }
        if let Some(..) = pointer {
            let mut hxpaths = Vec::new();
            print!("do_roll_disk.save_block_hash_path ");
            while let Some(ref p) = pointer {
                print!("->{}", p.height);
                hxpaths.push((BlockHeight::from(p.height), p.hash));
                pointer = p.parent.upgrade();
            }
            println!("");
            self.blockdisk.save_block_hash_path(hxpaths);
        }
        // write block data to disk
        self.blockdisk.save_block_data(&hx, &data);
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
            self.roller.read().unwrap().fast_search(prev_hei, prev_hx) 
        };
        let Some(prev_chunk) = prev else {
            return errf!("not find prev block <{}, {}>", prev_hei, prev_hx)
        };
        // check repeat
        let brothers: Vec<Arc<Chunk>> = {
            prev_chunk.childs.lock().unwrap().iter().map(|a|a.clone()).collect()
        };
        for sub in brothers {
            if hx == sub.hash {
                return errf!("repetitive block height {} hash {}", hei, hx)
            }
        }
        // create sub state 
        let prev_state = prev_chunk.state.clone();
        let mut sub_state = prev_state.fork_sub(prev_state.clone());
        // exec block get state
        let chaincnf = ctx::Chain {
            id: self.cnf.chain_id,
        };
        sub_state = block.objc.execute(chaincnf, sub_state)?;
        // create chunk
        let (objc, data) = block.apart();
        let chunk = Chunk::create(objc.into(), sub_state.into());
        // insert chunk
        let (root, pointer) = self.roller.write().unwrap().insert(chunk)?;
        Ok((root, pointer, data))
    }



    fn do_insert_sync(&self, start_hei: u64, mut datas: Vec<u8>) -> Rerr {
        let cur_hei = self.latest_block().height().uint();
        if start_hei != cur_hei + 1 {
            return sync_warning(format!("need height {} but got {}", cur_hei+1, start_hei))
        }
        let this = self;
        // create thread
        let (chblk, chblkcv) = std::sync::mpsc::sync_channel(10);
        let (chrol, chrolcv) = std::sync::mpsc::sync_channel(1);
        let (cherr, cherrcv) = std::sync::mpsc::sync_channel(2);
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
                    let blk = block::create(&blocks);
                    if let Err(e) = blk {
                        let _ = cherr1.send(format!("blocks::create error: {}", e));
                        break
                    }
                    let (blk, sk) = blk.unwrap();
                    let blkhei = blk.height().uint();
                    if hei != blkhei {
                        let _ = cherr1.send(format!("need block height {} but got {}", hei, blkhei));
                        break
                    }
                    let (left, right) = blocks.split_at_mut(sk);
                    blocks = right; // next chunk
                    let mut pkg = BlockPkg::new(blk, left.into());
                    pkg.set_origin( BlkOrigin::SYNC );
                    if let Err(..) = chblk.send(pkg) {
                        break // end
                    }
                    // next
                    hei += 1;
                }
            });
            // create check
            s.spawn(move || {
                loop {
                    let Ok(blk) = chblkcv.recv() else {
                        break // end
                    };
                    let hei = blk.objc.height().uint();
                    let hx = blk.hash.clone();
                    // println!("sync insert height: {}, body: {}", hei, blk.body().bytes.hex());
                    let res = this.do_insert_chunk(blk);
                    if let Err(e) = res {
                        let _ = cherr2.send(format!("create chunk {} error: {}", hei, e));
                        break // end
                    }
                    let (r, p, d) = res.unwrap();
                    if let Err(..) = chrol.send((r, p, hx, d)) {
                        break // end
                    }
                }
            });
            // do roll
            loop {
                let Ok((r, p, h, d)) = chrolcv.recv() else {
                    break // end
                };
                if let Err(e) = this.do_roll_disk(r, p, h, d) {
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
            return sync_warning(err)
        }
        // ok
        Ok(())
    }



    /*
    fn do_insert_sync(&self, start_hei: u64, mut datas: Vec<u8>) -> Rerr {
        let (crtchin, crtchout) = std::sync::mpsc::sync_channel(10);
        let (istchin, istchout) = std::sync::mpsc::sync_channel(1);
        let (errchin, errchout) = std::sync::mpsc::sync_channel(5);
        let this = self;
        let errchin1 = errchin.clone();
        let errchin2 = errchin.clone();
        std::thread::scope(|s| {
            // parse block
            s.spawn(move || {
                let mut hei = start_hei;
                let mut blocks = datas.as_mut_slice();
                // let mut benchmark = Duration::new(0, 0);
                loop {
                    // let now = Instant::now();
                    if blocks.len() == 0 {
                        break
                    }
                    // let now0 = Instant::now();
                    let blk = block::create(&blocks);
                    if let Err(e) = blk {
                        let err = sync_warning(format!("blocks::create error: {}", e));
                        errchin1.send(err);
                        break
                    }
                    let (blk, sk) = blk.unwrap();
                    let blkhei = blk.height().uint();
                    if hei != blkhei {
                        let err = sync_warning(format!("need block height {} but got {}", hei, blkhei));
                        errchin1.send(err);
                        break
                    }
                    let (left, right) = blocks.split_at_mut(sk);
                    blocks = right; // next chunk
                    let mut pkg = BlockPkg::new(blk, left.into());
                    pkg.set_origin( BlkOrigin::SYNC ); // mark block is sync
                    // let now1 = Instant::now();
                    // benchmark += now1.duration_since(now);
                    if let Err(_) = crtchin.send(pkg) {
                        break // end
                    }
                    // next block
                    hei += 1;
                }
                // print!(" {:?}", benchmark);
            });
            // do insert
            
            loop {
                // lpnum += 1;
                let chk = istchout.recv();
                if chk.is_err() {
                    break // end
                }
                // let now = Instant::now();
                let chunk_ptr = chk.unwrap();
                let res = self.roll_store(chunk_ptr);
                if res.is_err() {
                    let err = sync_warning(format!("roll store error: {}", res.err().unwrap()));
                    errchin.send(err);
                    break // end
                }
                // next
                // let now1 = Instant::now();
                // benchmark += now1.duration_since(now);
            }
            // print!(" {:?} loop{} ", benchmark, lpnum);
            // ok not err
            errchin.send("".to_string());
        });
        let err = errchout.recv().unwrap();
        if err.len() > 0 {
            return Err(err)
        }
        // finish
        Ok(())
    }
    */



}




fn sync_warning(e: String) -> Rerr {
    errf!("\n[Block Sync Warning] {}\n", e)
}


