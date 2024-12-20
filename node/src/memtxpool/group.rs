
struct TxGroup {
    maxsz: usize,
    txpkgs: Vec<TxPkg>,
}

impl TxGroup {

    fn new(sz: usize) -> TxGroup {
        TxGroup {
            maxsz: sz,
            txpkgs: Vec::new(),
        }
    }

}
