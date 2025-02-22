


fn impl_tx_check(this: &HacashMinter, tx: &dyn TransactionRead, next_hei: u64) -> Rerr {
    let Some(diamintact) = pickout_diamond_mint_action(tx) else {
        return Ok(()) // other normal tx
    };
    // deal with diamond mint action
    if next_hei % 5 == 0 {
        // println!("diamond mint transaction cannot submit after height of ending in 4 or 9");
        return errf!("diamond mint transaction cannot submit after height of ending in 4 or 9")
    }
    /*  test start *
    let bidaddr = tx.main();
    let bidfee  = tx.fee().clone();
    let dianame = diamintact.d.diamond;
    let dianum  = *diamintact.d.number;
    println!("**** {} diamond bidding {}-{} addr: {}, fee: {}", ctshow().split_off(11),
        dianame.to_readable(), dianum, bidaddr.readable(), bidfee);
    * test end */
    // check_diamond_mint_minimum_bidding_fee
    check_diamond_mint_minimum_bidding_fee(next_hei, tx, &diamintact)?;
    // record tx
    let mut biddings = this.bidding_prove.lock().unwrap();
    biddings.record(tx, &diamintact);
    // ok
    Ok(())
}



fn impl_prepare(this: &HacashMinter, curblkhead: &dyn BlockRead, sto: &BlockDisk) -> Rerr {
    let curhei = curblkhead.height().uint(); // u64
    let curdifnum = curblkhead.difficulty().uint();
    let blkspan = this.cnf.difficulty_adjust_blocks;
    if curhei <= blkspan {
        return Ok(()) // not check in first cycle
    }
    if this.cnf.chain_id == 0 && curhei < 288*200 {
        return Ok(()) // not check, compatible history code
    }
    if curhei % blkspan == 0 {
        return Ok(()) // not check, difficulty change to update
    }
    // check
    let (_, difnum, diffhx) = this.difficulty.req_cycle_block(curhei, sto);
    if difnum != curdifnum {
        return errf!("block {} PoW difficulty must be {} but got {}", curhei, difnum, curdifnum)
    }
    let cblkhx = curblkhead.hash();
    if hash_big_than(cblkhx.as_ref(), &diffhx) {
        return errf!("block {} PoW hashrates check failed cannot more than {} but got {}", 
            curhei, hex::encode(diffhx),  hex::encode(cblkhx))
    }
    // check success
    Ok(())
}



fn impl_consensus(this: &HacashMinter, prevblk: &dyn BlockRead, curblk: &dyn BlockRead, sto: &BlockDisk) -> Rerr {
    let curhei = curblk.height().uint(); // u64
    /*if curhei > 628955 { // test debug
        return errf!("test for curhei <= 628955")
    }*/
    // check difficulty
    let blkspan = this.cnf.difficulty_adjust_blocks;
    if this.cnf.chain_id==0 && curhei < 288*200 {
        return Ok(()) // not check, compatible history code
    }
    // check
    let curn = curblk.difficulty().uint(); // u32
    let _curbign = u32_to_biguint(curn);
    let prevn = prevblk.difficulty().uint(); // u32
    let prevtime = prevblk.timestamp().uint(); // u64
    let (tarn, tarhx, _tarbign) = this.difficulty.target(&this.cnf, prevn, prevtime, curhei, sto);
    // check
    /*if curbign!=tarbign || tarn!=curn || tarhx!=u32_to_hash(curn) {
        println!("\nheight: {}, {} {}, tarhx: {}  curhx: {} ----------------", 
        curhei, tarn, curn, hex::encode(&tarhx), hex::encode(u32_to_hash(curn)));
        return errf!("curbign != tarbign")
    }*/
    if tarn != curn {
        return errf!("height {} PoW difficulty check failed must be {} but got {}", curhei, tarn, curn)
    }
    if curhei % blkspan == 0 {
        // must check hashrates cuz impl_prepare not do check
        if  hash_big_than(curblk.hash().as_ref(), &tarhx) {
            return errf!("height {} PoW hashrates check failed cannot more than {} but got {}", 
                curhei, hex::encode(tarhx),  hex::encode(curblk.hash()))
        }
    }
    // success
    Ok(())
}



fn impl_examine(this: &HacashMinter, curblk: &BlockPkg, sta: &dyn State) -> Rerr {
    let curhei = curblk.hein; // u64
    // check diamond mint action
    // let is_discover = curblk.orgi == BlkOrigin::DISCOVER;
    if curhei > 630000 && curhei % 5 == 0 {
        if let Some((tidx, txp, diamint)) = pickout_diamond_mint_action_from_block(curblk.objc.as_read()) {
            const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
            if tidx != 1 && curhei > 600000 { // idx 0 is coinbase
                return errf!("diamond mint transaction must be first one tx in block")
            }
            let dianum  = *diamint.d.number;
            let bidfee  = txp.fee().clone();
            // check_diamond_mint_minimum_bidding_fee
            check_diamond_mint_minimum_bidding_fee(curhei, txp.as_read(), &diamint)?; // HIP-18
            let mut biddings = this.bidding_prove.lock().unwrap();
            if let Some(rhbf) = biddings.highest(dianum, sta) {
                if bidfee < rhbf { // 
                    /* test print start */
                    println!("\n\n✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖\ndiamond mint bidding fee {} less than consensus record {}", bidfee, rhbf);
                    println!("block height {} have a diamond {}-{}, address: {}, fee: {}, RecordHighestBidding: {}, {}\n", 
                        curhei, diamint.d.diamond.to_readable(), dianum, txp.main().readable(), bidfee,
                        rhbf, biddings.print(dianum),
                    );
                    /* test print end */ 
                    if dianum > CKN {  // HIP-19, check after 107000, reject blocks that don't follow the rules
                        return errf!("diamond mint bidding fee {} less than consensus record {}", bidfee, rhbf)
                    }
                } else if bidfee > rhbf {
                    print!(",\n        diamond bid fee {} record highest {} ", bidfee, rhbf)
                }
                // check success
            }
            // check ok and clear for next diamond
            biddings.roll(dianum);
        }
    }
    

    Ok(())
}



/************************/



fn check_diamond_mint_minimum_bidding_fee(next_hei: u64, tx: &dyn TransactionRead, dmact: &DiamondMint) -> Rerr {
    const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
    // check
    let bidmin = block_reward(next_hei);
    let _bidaddr = tx.main();
    let bidfee  = tx.fee().clone();
    let _dianame = dmact.d.diamond;
    let dianum  = *dmact.d.number;
    // test print
    /* if bidfee < bidmin {
        println!("DIAMOND MINT WARNNING: diamond biding fee {} cannot less than {} after number {}", bidfee, bidmin, CKN)
    } */
    // not check before 107000
    if bidfee < bidmin && dianum > CKN {
        return errf!("diamond biding fee {} cannot less than {} after number {}", bidfee, bidmin, CKN)
    }
    // all ok
    Ok(())
}