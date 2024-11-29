



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
        let mut rfees = self.avgfees.lock().unwrap();
        let mut avgf = self.cnf.lowest_fee_purity; 
        let txs = block.transactions();
        let txnum = txs.len();
        if txnum >= 30 {
            let nmspx = txnum / 3;
            let mut allpry = 0;
            for i in nmspx .. nmspx*2 {
                allpry += txs[i].fee_purity();
            }
            avgf = allpry / nmspx as u64;
        }
        // record
        rfees.push_front(avgf);
        if rfees.len() > 8 { // record 8 block avg fee
            rfees.pop_back();
        }
    }



}