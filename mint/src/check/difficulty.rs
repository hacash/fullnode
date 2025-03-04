
const HXS: usize = 32; // hash size


#[derive(Clone)]
pub struct DifficultyGnr {
    cnf: MintConf,
    block_caches: Arc<Mutex<HashMap<u64,(u64,u32,[u8; HXS])>>>, // height => (time, diffhx) 
}

impl DifficultyGnr {

    pub fn new(cnf: MintConf) -> DifficultyGnr {
        DifficultyGnr {
            cnf: cnf,
            block_caches: Arc::default(),
        }
    }

}



impl DifficultyGnr {

    pub fn req_cycle_block(&self, hei: u64, sto: &BlockStore) -> (u64, u32, [u8; HXS]) {
        let cylnum = self.cnf.difficulty_adjust_blocks; // 288
        if hei < cylnum {
            let cyltime = genesis_block().timestamp().uint();
            let diffcty = genesis_block().difficulty().uint();
            let diffhx = u32_to_hash(diffcty);
            return (cyltime, diffcty, diffhx)
        }
        let cylhei = hei / cylnum * cylnum;
        let mut cache = self.block_caches.lock().unwrap();
        if let Some(blk_time) = cache.get(&cylhei) {
            return *blk_time // find in cache
        }
        // read from database
        let (_, blkdts) = sto.block_data_by_height(&BlockHeight::from(cylhei)).unwrap();
        let intro = BlockIntro::must(&blkdts);
        // get time
        let cyltime = intro.timestamp().uint();
        let diffcty = intro.difficulty().uint();
        let diffhx = u32_to_hash(diffcty);
        let ccitem = (cyltime, diffcty, diffhx);
        cache.insert(cylhei, ccitem);
        if cache.len() as u64 > cylnum {
            cache.clear(); // clear
        }
        // ok
        ccitem
    }

    


    /*
    *
    */
    pub fn target(&self, mcnf: &MintConf, prevdiff: u32, prevblkt: u64, hei: u64, sto: &BlockStore) -> (u32, [u8;32], BigUint) {
        let cylnum = self.cnf.difficulty_adjust_blocks;
        if hei < cylnum * 2 {
            let dn = LOWEST_DIFFICULTY;
            return (dn, u32_to_hash(dn), u32_to_biguint(dn))
        }
        if hei % cylnum != 0 {
            let hx = u32_to_hash(prevdiff);
            return (prevdiff, hx, hash_to_biguint(&hx))
        }
        // count time
        let blk_span = self.cnf.each_block_target_time;
        let target_time_span = cylnum * blk_span; // 288 * 300
        let (prevcltime, _, _) = self.req_cycle_block(hei - cylnum, sto);
        let mut real_time_span = blk_span + prevblkt - prevcltime; // +300: 287+1block
        if mcnf.is_mainnet() && hei < 288*450 {
            // in mainnet chain id = 0
            // -300 = 287block, compatible history code
            real_time_span -= blk_span; 
        }
        let minsecs =  target_time_span / 4;
        let maxsecs =  target_time_span * 4;
        if real_time_span < minsecs {
            real_time_span = minsecs;
        }else if real_time_span > maxsecs {
            real_time_span = maxsecs;
        }
        // calculate
        let prevbign = u32_to_biguint(prevdiff);
        let targetbign = prevbign * BigUint::from(real_time_span) / BigUint::from(target_time_span);
        let tarhash = biguint_to_hash(&targetbign);
        let tarnum = hash_to_u32(&tarhash);
        (tarnum, tarhash, targetbign)
    }

}


impl HacashMinter {

    fn next_difficulty(&self, prev: &dyn BlockRead, sto: &BlockStore) -> u32 {
        let pdif = prev.difficulty().uint();
        let ptim = prev.timestamp().uint();
        let nhei = prev.height().uint() + 1;
        let (difn, ..) = self.difficulty.target(&self.cnf, pdif, ptim, nhei, sto);
        difn
    }

}