
fn impl_prepare(this: &HacashMinter, curblk: &dyn BlockRead, sto: &BlockDisk) -> Rerr {
    let curhei = curblk.height().uint(); // u64
    let curdifnum = curblk.difficulty().uint();
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
    let cblkhx = curblk.hash();
    if hash_big_than(cblkhx.as_ref(), &diffhx) {
        return errf!("block {} PoW hashrates check failed cannot more than {} but got {}", 
            curhei, hex::encode(diffhx),  hex::encode(cblkhx))
    }
    // check success
    Ok(())
}



fn impl_consensus(this: &HacashMinter, prevblk: &dyn BlockRead, curblk: &dyn BlockRead, sto: &BlockDisk) -> Rerr {
    let curhei = curblk.height().uint(); // u64
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