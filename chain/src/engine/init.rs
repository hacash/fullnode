use block::BlkOrigin;


fn load_root_block(minter: &dyn Minter, disk: Arc<DiskKV>, is_state_upgrade: bool) -> Arc<dyn Block> {
    let ret_gns_blk = ||{
        minter.genesis_block().clone()
    };
    if is_state_upgrade {
        return ret_gns_blk() // genesis block for upgrade
    }
    let disk = BlockDisk::wrap(disk.clone());
    let status = disk.status();
    let rhei = &status.root_height;
    let rhein = rhei.to_uint();
    if 0 == rhein {
        return ret_gns_blk() // genesis block
    }
    let Some((_, _, resblk)) = disk.block_by_height(&rhei) else {
        panic!("rebuild state error, cannot laod block {}", rhein)
    };
    resblk.into()
}


fn rebuild_unstable_blocks(this: &ChainEngine) {

    let disk = BlockDisk::wrap(this.disk.clone());
    let status = disk.status();
    // next
    let mut next_height: u64 = {
        let chei = this.roller.read().unwrap().root.height;
        chei
    };
    // build unstable blocks 
    let finish_height = status.last_height.to_uint();
    let is_all_rebuild = finish_height - next_height > 10;
    if is_all_rebuild {
        println!("[Database] rebuild all blocks to upgrade data version, plase waiting...");
    }else{
        print!("[Engine] Data: {}, rebuild ({})", this.cnf.data_dir, next_height);
    }
    // insert lock
    loop {
        next_height += 1;
        let Some((hx, blkdata, block)) = disk.block_by_height(&next_height.into()) else {
            println!(" ok.");
            return // end finish
        };
        if is_all_rebuild {
            if next_height % 500 == 0 {
                let per = next_height as f32 / finish_height as f32;
                flush!("\r{:10} ({:.2}%)", next_height, per*100.0);
            }
        } else {
            flush!("➢{}", next_height);
        }
        // try insert
        let ier = this.do_insert(BlockPkg{
            hein: next_height,
            hash: hx,
            data: blkdata,
            objc: block,
            orgi: BlkOrigin::REBUILD
        });
        if let Err(e) = ier {
            panic!("[State Panic] rebuild block state error: {}", e);
        }
        // next
    }
}

