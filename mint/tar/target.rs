
impl DifficultyGnr {

    pub fn target(&self, mcnf: &MintConf, prevdiff: u32, prevblkt: u64, hei: u64, sto: Arc<dyn DiskDB>) -> (u32, [u8;32], BigUint) {
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
        if mcnf.chain_id==0 && hei < 288*450 {
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