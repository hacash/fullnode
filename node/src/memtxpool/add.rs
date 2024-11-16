

impl TxGroup {

    fn insert(&mut self, txp: Box<TxPkg>) -> Rerr {
        let feep = txp.fee_purity();
        if let Some((hid, hav)) = self.find(&txp.hash) {
            if feep <= hav.fee_purity() {
                return errf!("tx already exists in tx pool and it's fee is higher")
            }
            // rm old
            self.txpkgs.remove(hid);
        }
        // check
        let gnum = self.txpkgs.len(); 
        if gnum == 0 {
            // first one
            self.txpkgs.push(txp);
            return Ok(())
        }
        if gnum >= self.maxsz {
            // tt's full, check the lowest fees
            let lowfp = self.txpkgs.last().unwrap().fee_purity();
            if feep <= lowfp {
                return errf!("tx pool is full and your tx fee is too low")
            }
        }
        // do insert
        let mut rxl = 0;
        let mut rxr = gnum; 
        if gnum > 10 {
            (rxl, rxr) = scan_group_rng_by_feep(&self.txpkgs, feep, (rxl, rxr));
        }
        // inser with rng
        self.insert_rng(txp, feep, (rxl, rxr))?;
        // check full
        if self.txpkgs.len() > self.maxsz {
            // drop lowest
            self.txpkgs.pop();
        }
        Ok(())
    }

    fn insert_rng(&mut self, txp: Box<TxPkg>, feep: u64, rng: (usize, usize)) ->Rerr {
        let (rxl, rxr) = rng;
        let mut istx = usize::MAX;
        for i in rxl .. rxr {
            let ctx = &self.txpkgs[i];
            if feep > ctx.fee_purity() {
                istx = i; // scan ok
                break;
            }
        }
        // do
        if istx == usize::MAX {
            self.txpkgs.push(txp); // tail
        }else{
            self.txpkgs.insert(istx, txp);
        }
        Ok(())
    }


}

