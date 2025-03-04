

#[allow(dead_code)]
pub struct MemTxPool {
    lowest_fepr: u64,
    group_size: Vec<usize>,
    groups: Vec<Mutex<TxGroup>>,
}

impl MemTxPool {
    
    pub fn new(lfepr: u64, gs: Vec<usize>, fpmds: Vec<bool>) -> MemTxPool {
        const MS: usize = MemTxPool::GROUP;
        if gs.len() != MS {
            panic!("new tx pool group size must be {}", MS)
        }
        let mut grps = Vec::with_capacity(MS);
        for i in 0 .. gs.len() {
            let sz = gs[i];
            grps.push( Mutex::new( TxGroup::new(sz, fpmds[i])) );
        }
        MemTxPool {
            lowest_fepr: lfepr,
            group_size: gs,
            groups: grps,
        }
    }

}


impl TxPool for MemTxPool {

    fn count_at(&self, gi: usize) -> Ret<usize> {
        check_group_id(gi)?;
        let count = self.groups[gi].lock().unwrap().txpkgs.len();
        Ok(count)
    }

    fn first_at(&self, gi: usize) -> Ret<Option<TxPkg>> {
        check_group_id(gi)?;
        let grp = self.groups[gi].lock().unwrap();
        Ok(grp.txpkgs.first().map(|a|a.clone())) // get first one if have
    }

    fn iter_at(&self, gi: usize, scan: &mut dyn FnMut(&TxPkg)->bool) -> Rerr {
        check_group_id(gi)?;
        let grp = self.groups[gi].lock().unwrap();
        for txp in &grp.txpkgs {
            if false == scan(&txp) {
                break // finish
            }
        }
        Ok(())
    }

    // insert to target group
    fn insert_at(&self, gi: usize, txp: TxPkg) -> Rerr { 
        if txp.fepr < self.lowest_fepr {
            return errf!("tx fee purity {} too low to add txpool", txp.fepr)
        }
        // let test_hex_show = txp.data.hex();
        check_group_id(gi)?;
        // do insert
        let mut grp = self.groups[gi].lock().unwrap();
        grp.insert(txp)?;
        /*
        drop(grp);
        print!("memtxpool insert_at {} total {}", gi, self.print());
        if gi == 0 {
            println!(", tx body: {}", test_hex_show);
        }else{
            println!("\n");
        }
        */
        Ok(())
    }

    fn delete_at(&self, gi: usize, hxs: &[Hash]) -> Rerr {
        check_group_id(gi)?;
        // do delete
        let mut grp = self.groups[gi].lock().unwrap();
        grp.delete(hxs);
        Ok(())
    }

    fn clear_at(&self, gi: usize) -> Rerr {
        check_group_id(gi)?;
        // do clean
        let mut grp = self.groups[gi].lock().unwrap();
        grp.clear();
        Ok(())
    }

    // from group id
    fn find_at(&self, gi: usize, hx: &Hash) -> Option<TxPkg> {
        check_group_id(gi).unwrap();
        // do clean
        let grp = self.groups[gi].lock().unwrap();
        match grp.find(hx) {
            Some((_, &ref tx)) => Some(tx.clone()),
            None => None,
        }
    }

    // remove if true
    fn retain_at(&self, gi: usize, f: &mut dyn FnMut(&TxPkg)->bool) -> Rerr {
        check_group_id(gi)?;
        self.groups[gi].lock().unwrap().retain(f);
        Ok(())
    }

    
    // find
    fn find(&self, hx: &Hash) -> Option<TxPkg> {
        for gi in 0..self.groups.len() {
            if let Some(tx) = self.find_at(gi, hx) {
                return Some(tx) // ok find
            }
        }
        // not find
        None
    }

    fn insert_by(&self, txp: TxPkg, check_group: &dyn Fn(&TxPkg)->usize) -> Rerr {
        // check for group
        let group_id = check_group(&txp);
        // println!("TXPOOL: insert tx {} in group {}", tx.hash().hex(), group_id);
        // insert
        self.insert_at(group_id, txp)
    }

    fn drain(&self, hxs: &[Hash]) -> Ret<Vec<TxPkg>> {
        let mut txres = vec![];
        let mut hxst = HashSet::from_iter(hxs.to_vec());
        for gi in 0..self.groups.len() {
            let mut grp = self.groups[gi].lock().unwrap();
            let mut res = grp.drain(&mut hxst);
            txres.append(&mut res);
        }
        // println!("memtxpool drain hxs {} status: {}", hxs.len(), self.print());
        Ok(txres)
    }

    fn print(&self) -> String {
        let mut shs: Vec<String> = vec![];
        for gi in 0..self.groups.len() {
            if let Ok(gr) = self.groups[gi].try_lock() {
                shs.push(format!("{}({})", Self::TIPS[gi], gr.txpkgs.len()));
            }
        }
        format!("[TxPool] tx count: {}", shs.join(", "))
    }


}

