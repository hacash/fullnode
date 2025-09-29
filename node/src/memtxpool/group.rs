
struct TxGroup {
    maxsz: usize,
    txpkgs: Vec<TxPkg>,
    fpmd: bool,
}

impl TxGroup {

    fn new(sz: usize, fpmd: bool,) -> TxGroup {
        TxGroup {
            maxsz: sz,
            txpkgs: Vec::new(),
            fpmd,
        }
    }

}
