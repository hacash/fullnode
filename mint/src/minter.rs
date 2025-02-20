




#[allow(dead_code)]
pub struct HacashMinter {
    cnf: MintConf,
    difficulty: DifficultyGnr,
    genesis_block: Arc<dyn Block>,
    // check highest bidding
    biddings: Mutex<VecDeque<BiddingRecord>>,
}

impl HacashMinter {

    pub fn create(ini: &IniObj) -> Self {
        let cnf = MintConf::new(ini);
        let dgnr = DifficultyGnr::new(cnf.clone());
        let dbs = Self::DELAY_SECS + Self::RECORD_NUM + 1;
        Self {
            cnf: cnf,
            difficulty: dgnr,
            genesis_block: genesis_block_pkg().into_block().into(),
            biddings: Mutex::new(VecDeque::with_capacity(dbs)),
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

    fn next_difficulty(&self, prev: &dyn BlockRead, sto: &BlockDisk) -> u32 {
        let pdif = prev.difficulty().uint();
        let ptim = prev.timestamp().uint();
        let nhei = prev.height().uint() + 1;
        let (difn, ..) = self.difficulty.target(&self.cnf, pdif, ptim, nhei, sto);
        difn
    }

    fn tx_check(&self, tx: &dyn TransactionRead, chei: u64) -> Rerr {
        impl_tx_check(self, tx, chei)
    }

    fn prepare(&self, curblk: &dyn BlockRead, sto: &BlockDisk ) -> Rerr {
        impl_prepare(self, curblk, sto)
    }

    fn consensus(&self, prevblk: &dyn BlockRead, curblk: &dyn BlockRead, sta: &dyn State, sto: &BlockDisk, blkori: BlkOrigin) -> Rerr {
        impl_consensus(self, prevblk, curblk, sta, sto, blkori)
    }

    fn genesis_block(&self) -> Arc<dyn Block> {
        self.genesis_block.clone()
    }

    fn initialize(&self, sta: &mut dyn State) -> Rerr {
        do_initialize(sta)
    }

    fn coinbase(&self, hei: u64, tx: &dyn Transaction) -> Rerr {
        check_coinbase(hei, tx)
    }



}
