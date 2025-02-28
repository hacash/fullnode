

fn load_root_block(minter: &dyn Minter, disk: Arc<DiskKV>, is_state_upgrade: bool) -> Arc<dyn Block> {
    let ret_gns_blk = ||{
        minter.genesis_block().clone()
    };
    if is_state_upgrade {
        return ret_gns_blk() // genesis block for upgrade
    }
    let disk = BlockStore::wrap(disk.clone());
    let status = disk.status();
    let rhei = &status.root_height;
    let rhein = rhei.uint();
    if 0 == rhein {
        return ret_gns_blk() // genesis block
    }
    let Some((_, _, resblk)) = disk.block_by_height(rhei) else {
        panic!("cannot load root block {}", rhein)
    };
    resblk.into()
}


fn rebuild_unstable_blocks(this: &ChainEngine) {

    let status = this.store.status();
    // next
    let mut next_height = this.roller.lock().unwrap().root.height;
    // build unstable blocks 
    let finish_height = *status.last_height;
    let is_all_rebuild = finish_height - next_height > 20;
    if is_all_rebuild {
        println!("[Database] check all blocks to upgrade state db version, plase waiting...");
    }else{
        print!("[Engine] Data: {}, rebuild ({})", this.cnf.data_dir, next_height);
    }
    // insert lock
    loop {
        next_height += 1;
        let Some((hx, blkdata, block)) = this.store.block_by_height(&next_height.into()) else {
            break; // end finish
        };
        // assert_eq!(blkdata, block.serialize(), "assert_eq block {}", block.height().uint());
        if is_all_rebuild {
            if next_height % 631 == 0 {
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
            panic!("[State Panic] rebuild block {} state error: {}", next_height, e);
        }
        // next
    }
    // finish tip
    if is_all_rebuild {
        flush!("\r{:10} ({:.2}%)", next_height-1, 100.0);
    }else{
        println!(" ok.");
    }
}

