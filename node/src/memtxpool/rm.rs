
impl TxGroup {
    
    fn clear(&mut self) {
        self.txpkgs.clear()
    }

    fn remove(&mut self, txhx: &Hash) -> Option<TxPkg> {
        let Some(rmid) = self.search(txhx) else {
            return None
        };
        // remove
        Some(self.txpkgs.remove(rmid))
    }

    fn drain(&mut self, hxst: &mut HashSet<Hash>) -> Vec<TxPkg> {
        let mut res = vec![];
        let hxs: Vec<Hash> = hxst.iter().map(|a|a.clone()).collect();
        for hx in hxs {
            if let Some(txp) = self.remove(&hx) {
                hxst.remove(&hx);
                res.push(txp);
            }
        }
        res
    }


    // remove if true
    fn drain_filter(&mut self, filter: &dyn Fn(&TxPkg)->bool) -> Rerr {
        self.txpkgs.retain(|a| !filter(a) );
        Ok(())
    }


    fn delete(&mut self, txhxs: &[Hash]) {
        for hx in txhxs {
            if ! self.del_one(hx) {
                return // group is empty
            }
        }
    }

    // delete one tx
    fn del_one(&mut self, hx: &Hash) -> bool {
        let mut rmidx: usize = 0;
        for tx in self.txpkgs.iter() {
            if tx.hash == *hx {
                self.txpkgs.remove(rmidx);
                return true
            }
            rmidx += 1;
        }
        // not find
        false

        /*

        let num = self.txpkgs.len();
        if num <= 0 {
            return false // nothing
        }
        let mut delmk = 0; // 0:notfind   1:remove   2:pop
        let mut i = num - 1;
        while i >= 0 {
            if *hx == self.txpkgs[i].hash {
                if i == num-1 {
                    delmk = 2 // tail
                }else{
                    delmk = 1
                }
                break
            }
            // next
            i -= 1;
        }
        // do rm
        if delmk == 2 {
            self.txpkgs.pop();
        }else if delmk == 1 {
            self.txpkgs.remove(i);
        }
        true
        */
    }
    

}