

pub struct MemTxPool {
    group_size: Vec<usize>,
    groups: Vec<Mutex<TxGroup>>,
}

impl MemTxPool {
    
    pub fn new(gs: Vec<usize>) -> MemTxPool {
        const MS: usize = TXPOOL_GROUP_MAX_SIZE;
        if gs.len() != MS {
            panic!("new tx pool group size must be {}", MS)
        }
        let mut grps = Vec::with_capacity(MS);
        for sz in &gs {
            grps.push( Mutex::new( TxGroup::new(*sz)) );
        }
        MemTxPool {
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

    fn iter_at(&self, scan: &mut dyn FnMut(&TxPkg)->bool, gi: usize) -> Rerr {
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
    fn insert_at(&self, txp: TxPkg, gi: usize) -> Rerr { 
        check_group_id(gi)?;
        // do insert
        let mut grp = self.groups[gi].lock().unwrap();
        grp.insert(txp)
    }

    fn delete_at(&self, hxs: &Vec<Hash>, gi: usize) -> Rerr {
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
    fn find_at(&self, hx: &Hash, gi: usize) -> Option<TxPkg> {
        check_group_id(gi).unwrap();
        // do clean
        let grp = self.groups[gi].lock().unwrap();
        match grp.find(hx) {
            Some((_, &ref tx)) => Some(tx.clone()),
            None => None,
        }
    }

    // remove if true
    fn drain_filter_at(&self, filter: &dyn Fn(&TxPkg)->bool, gi: usize) 
        -> Rerr
    {
        check_group_id(gi)?;
        self.groups[gi].lock().unwrap().drain_filter(filter)
    }

    
    // find
    fn find(&self, hx: &Hash) -> Option<TxPkg> {
        for gi in 0..self.groups.len() {
            if let Some(tx) = self.find_at(hx, gi) {
                return Some(tx) // ok find
            }
        }
        // not find
        None
    }

    fn insert(&self, txp: TxPkg) -> Rerr {
        let acts = txp.objc.actions();
        let actlen = acts.len();
        // check for group
        const DMINT: u16 = DiamondMint::KIND;
        let mut group_id = TXPOOL_GROUP_NORMAL;
        for i in 0..actlen {
            let act = &acts[i];
            if act.kind() == DMINT {
                group_id = TXPOOL_GROUP_DIAMOND_MINT;
            }
        }
        // println!("TXPOOL: insert tx {} in group {}", tx.hash().hex(), group_id);
        // insert
        self.insert_at(txp, group_id)
    }

    fn drain(&self, hxs: &Vec<Hash>) -> Ret<Vec<TxPkg>> {
        let mut txres = vec![];
        let mut hxst = HashSet::from_iter(hxs.clone().into_iter());
        for gi in 0..self.groups.len() {
            let mut grp = self.groups[gi].lock().unwrap();
            let mut res = grp.drain(&mut hxst);
            txres.append(&mut res);
        }
        Ok(txres)
    }

    fn print(&self) -> String {
        let mut shs: Vec<String> = vec![];
        for gi in 0..self.groups.len() {
            if let Ok(gr) = self.groups[gi].try_lock() {
                shs.push(format!("{}({})", TXPOOL_GROUP_TIPS[gi], gr.txpkgs.len()));
            }
        }
        format!("[TxPool] tx count: {}", shs.join(", "))
    }


}

