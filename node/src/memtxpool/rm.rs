
impl TxGroup {
    
    fn clear(&mut self) {
        self.txpkgs.clear()
    }

    fn remove(&mut self, txhx: &Hash) -> Option<Box<dyn TxPkg>> {
        let rmid = self.search(txhx);
        let rmid = match rmid {
            None => return None, // not find
            Some(idx) => idx,
        };
        // rm & ret
        Some(self.txpkgs.remove(rmid))
    }

    fn drain(&mut self, hxst: &mut HashSet<Hash>) -> Vec<Box<dyn TxPkg>> {
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
    fn drain_filter(&mut self, filter: &dyn Fn(&Box<dyn TxPkg>)->bool) -> RetErr {
        self.txpkgs.retain(|a| !filter(a) );
        Ok(())
    }


    fn delete(&mut self, txhxs: &Vec<Hash>) {
        for hx in txhxs {
            if ! self.del_one(hx) {
                return // group is empty
            }
        }
    }

    // delete one tx
    fn del_one(&mut self, hx: &Hash) -> bool {
        let num = self.txpkgs.len();
        if num <= 0 {
            return false // nothing
        }
        let mut delmk = 0; // 0:notfind   1:remove   2:pop
        let mut i = num - 1;
        while i >= 0 {
            if *hx == *self.txpkgs[i].hash() {
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
    }
    

}