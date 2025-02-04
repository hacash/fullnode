


fn impl_tx_check(_this: &HacashMinter, tx: &dyn TransactionRead, next_hei: u64) -> Rerr {
    let tn =  tx.action_count().uint();
    let txs = tx.actions();
    if tn as usize != txs.len() {
        return errf!("tx action count not match")
    }
    let Some(diamintact) = pickout_diamond_mint_action(tx) else {
        return Ok(()) // other normal tx
    };
    // deal with diamond mint action
    if next_hei % 5 == 0 {
        println!("diamond mint transaction cannot submit after height of ending in 4 or 9");
        return errf!("diamond mint transaction cannot submit after height of ending in 4 or 9")
    }
    let bidmin = block_reward(next_hei);
    let bidaddr = tx.main();
    let bidfee  = tx.fee().clone();
    let dianame = diamintact.d.diamond;
    let dianum  = *diamintact.d.number;
    const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
    println!("**** {} diamond bidding {}-{} addr: {}, fee: {}", ctshow().split_off(11),
        dianame.to_readable(), dianum, bidaddr.readable(), bidfee);
    if dianum <= CKN {
        // println!("not check before 107000");
        return Ok(()) // not check before 107000
    }
    if bidfee < bidmin {
        return errf!("diamond biding fee cannot less than {} after number {}", bidmin, CKN)
    }

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
    // check diamond mint action
    if curhei > 625000 && curhei % 5 == 0 {
        if let Some((txp, diamint)) = pickout_diamond_mint_action_from_block(curblk) {
            let dianame = diamint.d.diamond;
            let dianum  = *diamint.d.number;
            let bidaddr  = txp.main();
            let bidfee  = txp.fee().clone();
            println!("\n----\nblock height {} have a diamond {}-{}, address: {}, fee: {}\n----", 
                curhei, dianame.to_readable(), dianum, bidaddr.readable(), bidfee)
        }
    }
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