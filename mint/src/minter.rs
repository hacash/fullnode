




#[allow(dead_code)]
pub struct HacashMinter {
    cnf: MintConf,
    difficulty: DifficultyGnr,
    genesis_block: Arc<dyn Block>,
    // check highest bidding
    bidding_prove: Mutex<BiddingProve>,
}

impl HacashMinter {

    pub fn create(ini: &IniObj) -> Self {

        // create
        let cnf = MintConf::new(ini);
        let dgnr = DifficultyGnr::new(cnf.clone());
        Self {
            cnf: cnf,
            difficulty: dgnr,
            genesis_block: genesis::genesis_block_pkg().into_block().into(),
            bidding_prove: Mutex::default(),
        }
    }

}


impl Minter for HacashMinter {

    fn config(&self) -> Box<dyn Any> {
        Box::new(self.cnf.clone())
    }

    fn tx_submit(&self, eng: &dyn EngineRead, tx: &TxPkg) -> Rerr {
        impl_tx_submit(self, eng, tx)
    }

    fn blk_found(&self, curblk: &dyn BlockRead, sto: &dyn Store ) -> Rerr {
        impl_blk_found(self, curblk, sto)
    }

    fn blk_verify(&self, curblk: &dyn BlockRead, prevblk: &dyn BlockRead, sto: &dyn Store) -> Rerr {
        impl_blk_verify(self, curblk, prevblk, sto)
    }

    fn blk_insert(&self, curblk: &BlockPkg, sta: &dyn State, prev: &dyn State) -> Rerr {
        impl_blk_insert(self, curblk, sta, prev)
    }

    fn genesis_block(&self) -> Arc<dyn Block> {
        self.genesis_block.clone()
    }

    fn initialize(&self, sta: &mut dyn State) -> Rerr {
        do_initialize(self, sta)
    }

    // <dyn Block> == BlockV1
    fn packing_next_block(&self, eng: &dyn EngineRead, tp: &dyn TxPool) -> Box<dyn Any> {
        impl_packing_next_block(self, eng, tp)
    }


    /*
    fn coinbase(&self, hei: u64, tx: &dyn TransactionRead) -> Rerr {
        verify_coinbase(hei, tx)
    }
    */

    fn tx_pool_group(&self, tx: &TxPkg) -> usize {
        let mut group_id =  TXGID_NORMAL;
        if let Some(..) = action::pickout_diamond_mint_action(tx.objc.as_read()) {
            group_id = TXGID_DIAMINT;
        }
        group_id
    }


    fn tx_pool_refresh(&self, eng: &dyn EngineRead, txp: &dyn TxPool, txs: Vec<Hash>, blkhei: u64) {
        impl_tx_pool_refresh(self, eng, txp, txs, blkhei)
    }




}
