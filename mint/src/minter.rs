




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
        let cnf = MintConf::new(ini);
        let dgnr = DifficultyGnr::new(cnf.clone());
        Self {
            cnf: cnf,
            difficulty: dgnr,
            genesis_block: genesis_block_pkg().into_block().into(),
            bidding_prove: Mutex::default(),
        }
    }

}


impl Minter for HacashMinter {

    fn config(&self) -> Box<dyn Any> {
        Box::new(self.cnf.clone())
    }

    fn init(&self, _: &IniObj) {
        // extend actions
        // protocol::action::setup_extend_actions_try_create(action::empty_try_create);
        // protocol::action::setup_action_hook(hook::empty_action_hook);
    }

    fn next_difficulty(&self, prev: &dyn BlockRead, sto: &BlockStore) -> u32 {
        let pdif = prev.difficulty().uint();
        let ptim = prev.timestamp().uint();
        let nhei = prev.height().uint() + 1;
        let (difn, ..) = self.difficulty.target(&self.cnf, pdif, ptim, nhei, sto);
        difn
    }

    fn tx_submit(&self, eng: &dyn EngineRead, tx: &TxPkg) -> Rerr {
        impl_tx_submit(self, eng, tx)
    }

    fn blk_found(&self, curblk: &dyn BlockRead, sto: &BlockStore ) -> Rerr {
        impl_blk_found(self, curblk, sto)
    }

    fn blk_verify(&self, curblk: &dyn BlockRead, prevblk: &dyn BlockRead, sto: &BlockStore) -> Rerr {
        impl_blk_verify(self, curblk, prevblk, sto)
    }

    fn blk_insert(&self, curblk: &BlockPkg, sta: &dyn State, prev: &dyn State) -> Rerr {
        impl_blk_insert(self, curblk, sta, prev)
    }

    fn genesis_block(&self) -> Arc<dyn Block> {
        self.genesis_block.clone()
    }

    fn initialize(&self, sta: &mut dyn State) -> Rerr {
        do_initialize(sta)
    }

    fn coinbase(&self, hei: u64, tx: &dyn TransactionRead) -> Rerr {
        verify_coinbase(hei, tx)
    }



}
