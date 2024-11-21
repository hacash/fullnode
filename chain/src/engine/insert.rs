
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
            // print!("\ndo_roll_disk.save_block_hash_path: ");
            while let Some(ref p) = pointer {
                if hxpaths.len() >= 4 || p.height < 1 {
                    break // end
                }
                // print!("->{}", p.height);
                hxpaths.push((BlockHeight::from(p.height), p.hash));
                pointer = p.parent.upgrade();
            }
            // println!("");
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
        // initialize
        if hei == 1 { // first block
            self.minter.initialize(sub_state.as_mut())?;
        }
        // exec block get state
        let chaincnf = ctx::Chain {
            id: self.cnf.chain_id,
            fast_sync: self.cnf.fast_sync,
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
            // create check
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
            println!("{}", err);
            return sync_warning(err)
        }
        // ok
        Ok(())
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



