


// a simple hac trs size is 166 bytes
pub const DEFAULT_AVERAGE_FEE_PURITY: u64 = 10000_00000000 / 166; 





impl ChainEngine {

    fn record_recent(&self, block: &dyn BlockRead) {
        let chei = block.height().uint() as i128;
        let deln = (self.cnf.unstable_block * 2) as i128;
        let deln = chei - deln;
        // delete
        let mut rcts = self.rctblks.lock().unwrap();
        rcts.retain(|x| x.height as i128 > deln);
        // insert
        let rctblk = create_recent_block_info(block);
        rcts.push_front(rctblk.into()); // arc
    }

    fn record_avgfee(&self, block: &dyn BlockRead) {
        let mut juptxidx = 1usize; // jump coinbase tx
        let txs = block.transactions();
        let txnum = txs.len();
        if block.height().uint() % 5 == 0 {
            juptxidx += 1; // jump diamond mint tx
        }
        // 10000_00000000u64 / 200; // 1w zhu / 200byte(1 trs)
        let mut avgf = DEFAULT_AVERAGE_FEE_PURITY; 
        if juptxidx < txnum {
            let mut allpry = 0;
            for i in juptxidx .. txnum {
                let tx = &txs[i];
                let feeg = tx.fee_got().to_shuo_unsafe() as u64
                    / tx.size() as u64;
                allpry += feeg;
            }
            avgf = allpry / (txnum - juptxidx) as u64;
        }
        let mut rfees = self.avgfees.lock().unwrap();
        rfees.push_front(avgf);
        if rfees.len() > 8 { // record 8 block avg fee
            rfees.pop_back();
        }
    }



}