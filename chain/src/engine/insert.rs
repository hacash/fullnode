
#[allow(dead_code)]
impl ChainEngine {



    fn insert_by(&self, roller: &mut Roller, blk: BlockPkg) -> Ret<RollerInsertData> {

        let fast_sync = (self.cnf.fast_sync&&blk.orgi==BlkOrigin::Sync) || blk.orgi==BlkOrigin::Rebuild;
        // search prev chunk in roller tree
        let hei = blk.hein;
        let hx = blk.hash;
        let prev_hei = hei - 1;
        let prev_hx  = blk.objc.prevhash();
        let Some(prev_chunk) = roller.search(prev_hei, prev_hx) else {
            return errf!("not find prev block <{}, {}>", prev_hei, prev_hx)
        };
        if !fast_sync {
            // check repeat
            if prev_chunk.childs.iter().any(|c|c.hash==hx) {
                return errf!("repetitive block <{}, {}>", hei, hx)
            }
            // minter verify
            let prev_blk_ptr = prev_chunk.block.as_read();
            self.minter.blk_verify(blk.objc.as_read(), prev_blk_ptr, &self.store)?;
            self.block_verify(&blk, prev_blk_ptr)?;
        }
        // try execute
        // create sub state 
        let prev_state = prev_chunk.state.clone();
        let mut sub_state = prev_state.fork_sub(Arc::downgrade(&prev_state));
        // initialize on first block
        if hei == 1 {
            self.minter.initialize(sub_state.as_mut())?;
        }
        let c = &self.cnf;
        let chain_option = ChainInfo {
            fast_sync,
            diamond_form: c.diamond_form,
            id: c.chain_id,
        };
        // execute block
        sub_state = blk.objc.execute(chain_option, sub_state)?;
        if !fast_sync {
            self.minter.blk_insert(&blk, sub_state.as_ref(), prev_state.as_ref())?;
        }
        // create chunk
        let (hx, objc, data) = blk.apart();
        let chunk = Chunk::create(hx, objc.into(), sub_state.into());
        // insert chunk
        roller.insert(prev_chunk, chunk).map(|(a,b)|(
            a, b, hx, data, roller.root.height
        ))
    }

    // justckhd = just check head
    fn roll_by(&self, rid: RollerInsertData) -> Rerr {

        let (root_change, head_change, hx, data, old_root_hei) = rid;
    
        let mut store_batch = MemKV::new();
        // save block data to disk
        store_batch.put(hx.to_vec(), data); // block data
        // if head change
        if let Some(new_head) = head_change {
            let real_root_hei: u64 = match root_change.clone() {
                Some(rt) => rt.height,
                None => old_root_hei // roller.root.height
            };
            let new_head_hei = BlockHeight::from(new_head.height);
            store_batch.put(BlockStore::CSK.to_vec(), ChainStatus{
                root_height: BlockHeight::from(real_root_hei),
                last_height: new_head_hei,
            }.serialize());
            let mut skchk = new_head;
            let mut skhei = new_head_hei;
            for _ in 0..self.cnf.unstable_block+1 { // search the tree
                store_batch.put(skhei.to_vec(), skchk.hash.to_vec());
                skchk = match skchk.parent.upgrade() {
                    Some(h) => h,
                    _ => break // end
                };
                skhei -= 1;
            }
        }
        // write roll path and block data to disk
        self.store.save_batch(&store_batch);
        // if root change
        if let Some(new_root) = root_change.clone() {
            // write state data to disk
            new_root.state.write_to_disk();
            // println!("----  new_root.state.write_to_disk() for height {}", new_root.height);
            // scaner do roll
            self.scaner.roll(new_root.block.clone(), new_root.state.clone(), self.disk.clone());
        }
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



