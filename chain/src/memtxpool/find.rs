
impl TxGroup {

    fn search(&self, txhx: &Hash) -> Option<usize> {
        for (i, txp) in self.txpkgs.iter().enumerate() {
            if *txhx == txp.hash {
                return Some(i);
            }
        }
        // not find
        return None
    }

    fn find(&self, txhx: &Hash) -> Option<(usize, &TxPkg)> {
        let havid = self.search(txhx);
        if let Some(hid) = havid {
            if let Some(tx) = self.txpkgs.get(hid) {
                return Some((hid, tx))
            }
        }
        return None
    } 

}