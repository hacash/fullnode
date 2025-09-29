


fn impl_tx_submit(this: &HacashMinter, engine: &dyn EngineRead, txp: &TxPkg) -> Rerr {
    let txr = txp.objc.as_read();
    let curr_hei = engine.latest_block().height().uint();
    let next_hei = curr_hei + 1;
    let Some(diamintact) = action::pickout_diamond_mint_action(txr) else {
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
    check_diamond_mint_minimum_bidding_fee(next_hei, txr, &diamintact)?;
    // record tx
    let mut biddings = this.bidding_prove.lock().unwrap();
    biddings.record(curr_hei, txp, &diamintact);
    // ok
    Ok(())
}



fn impl_blk_found(this: &HacashMinter, curblkhead: &dyn BlockRead, sto: &dyn Store) -> Rerr {
    let curhei = curblkhead.height().uint(); // u64
    let curdifnum = curblkhead.difficulty().uint();
    let blkspan = this.cnf.difficulty_adjust_blocks;
    if curhei <= blkspan {
        return Ok(()) // not check in first cycle
    }
    if curhei < blkspan*200 && this.cnf.is_mainnet() {
        return Ok(()) // not check, compatible history code
    }

    let cblkhx = curblkhead.hash();
    if curhei % blkspan == 0 {
        let (_, difnum, _) = this.difficulty.req_cycle_block(curhei - 1, sto);
        let bign = u32_to_biguint(difnum).mul(4usize); // max is 4 times
        let mindiffhx = biguint_to_hash(&bign);
        if hash_big_than(cblkhx.as_ref(), &mindiffhx) {
            return errf!("block found {} PoW hashrates check failed cannot more than {} but got {}", 
                curhei, hex::encode(mindiffhx),  hex::encode(cblkhx))
        }
        return Ok(()) // not check in here, difficulty change to update
    }
    // checka
    let (_, difnum, diffhx) = this.difficulty.req_cycle_block(curhei, sto);
    if difnum != curdifnum {
        return errf!("found block {} PoW difficulty must be {} but got {}", curhei, difnum, curdifnum)
    }
    if hash_big_than(cblkhx.as_ref(), &diffhx) {
        return errf!("found block {} PoW hashrates check failed cannot more than {} but got {}", 
            curhei, hex::encode(diffhx),  hex::encode(cblkhx))
    }
    // check success
    Ok(())
}



fn impl_blk_verify(this: &HacashMinter, curblk: &dyn BlockRead, prevblk: &dyn BlockRead, sto: &dyn Store) -> Rerr {
    let curhei = curblk.height().uint(); // u64
    let smaxh = this.cnf.sync_maxh;
    if smaxh > 0 && curhei > smaxh {
        return errf!("config [mint].height_max limit: {}", smaxh)
    }
    // verify coinbase
    verify_coinbase(curhei, curblk.coinbase_transaction()?)?;
    // check difficulty
    let blkcln = this.cnf.difficulty_adjust_blocks; // 288
    if curhei < blkcln*200 && this.cnf.is_mainnet() {
        return Ok(()) // not check, compatible history code
    }
    // check
    let curn = curblk.difficulty().uint(); // u32
    let (tarn, tarhx, _tarbign) = this.next_difficulty(prevblk, sto);
    /*
    // dev test
    if curhei > 633000 && (curhei%288==0 || curhei%288==1 || curhei%287==2 || curhei%288==287 || curhei%288==286) {
        println!("---- this.next_difficulty(prevblk, sto) curhei: {}, tarint: {}, tarhx: {} ", curhei, tarn, hex::encode(&tarhx));
    }
    */
    // check
    /*if curbign!=tarbign || tarn!=curn || tarhx!=u32_to_hash(curn) {
        println!("\nheight: {}, {} {}, tarhx: {}  curhx: {} ----------------", 
        curhei, tarn, curn, hex::encode(&tarhx), hex::encode(u32_to_hash(curn)));
        return errf!("curbign != tarbign")
    }*/
    if tarn != curn {
        return errf!("height {} PoW difficulty check failed must be {} but got {}", curhei, tarn, curn)
    }
    // must check hashrates cuz impl_prepare not do check
    if hash_big_than(curblk.hash().as_ref(), &tarhx) {
        return errf!("height {} PoW hashrates check failed cannot more than {} but got {}", 
            curhei, hex::encode(tarhx),  hex::encode(curblk.hash()))
    }
    // success
    Ok(())
}



fn impl_blk_insert(this: &HacashMinter, curblk: &BlockPkg, _sta: &dyn State, prev: &dyn State) -> Rerr {

    check_highest_bid_of_block(this, curblk, prev)?;

    Ok(())
}



/************************/



fn check_highest_bid_of_block(this: &HacashMinter, curblk: &BlockPkg, prevsta: &dyn State) -> Rerr {

    let curhei = curblk.hein; // u64
    // check diamond mint action
    // let is_discover = curblk.orgi == BlkOrigin::DISCOVER;
    if curhei > 630000 && curhei % 5 == 0 {
        let block = curblk.objc.as_read();
        if let Some((tidx, txp, diamint)) = action::pickout_diamond_mint_action_from_block(block) {
            const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
            if tidx != 1 && curhei > 600000 { // idx 0 is coinbase
                return errf!("diamond mint transaction must be first one tx in block")
            }
            let dianum  = *diamint.d.number;
            let bidfee  = txp.fee().clone();
            // check_diamond_mint_minimum_bidding_fee
            check_diamond_mint_minimum_bidding_fee(curhei, txp.as_read(), &diamint)?; // HIP-18
            let mut bidrecord = this.bidding_prove.lock().unwrap();
            if let Some(rhbf) = bidrecord.highest(curhei, dianum, prevsta, block.timestamp().uint()) {
                if bidfee < rhbf { // 
                    bidrecord.failure(dianum, curblk); // record check block fail
                    /* test print start */
                    println!("\n✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖ ✕ ✖\ndiamond mint bidding fee {} less than consensus record {}", bidfee, rhbf);
                    println!("block height {} have a diamond {}-{}, address: {}, fee: {}, RecordHighestBidding: {}, {}", 
                        curhei, diamint.d.diamond.to_readable(), dianum, txp.main().readable(), bidfee,
                        rhbf, bidrecord.print(dianum),
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
            bidrecord.remove_tx(dianum, txp.hash());
            bidrecord.roll(dianum);
        }
    }


    Ok(())
}



fn check_diamond_mint_minimum_bidding_fee(next_hei: u64, tx: &dyn TransactionRead, dmact: &action::DiamondMint) -> Rerr {
    const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
    // check
    let bidmin = genesis::block_reward(next_hei);
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