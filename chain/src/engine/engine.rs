
pub struct ChainEngine {
    cnf: EngineConf,
    // 
    minter: Box<dyn Minter>,
    // data
    disk: Arc<dyn DiskDB>,
    roller: Mutex<Roller>,

}


impl ChainEngine {


    pub fn open(ini: &IniObj, dbv: u32,
        minter: Box<dyn Minter>
    ) -> ChainEngine {
        // init
        minter.init(ini); 
        // cnf
        let cnf = EngineConf::new(ini, dbv);
        let blk_dir = &cnf.block_data_dir;
        let sta_dir = &cnf.state_data_dir;
        std::fs::create_dir_all(blk_dir).unwrap();
        std::fs::create_dir_all(sta_dir).unwrap();
        // build
        let disk = Arc::new(DiskKV::open(blk_dir));
        // if state database upgrade
        let is_state_upgrade = !sta_dir.exists(); // not find new dir
        let sta_db =  DiskKV::open(sta_dir);
        let state = StateInst::build(Arc::new(sta_db), Weak::<StateInst>::new());
        let staptr = Arc::new(state);
        // base or genesis block
        let bsblk = match is_state_upgrade {
            true => minter.genesis_block().into(), // rebuild all block
            false => load_root_block(minter.as_ref(), disk.clone())
        };
        let roller = Roller::create(cnf.unstable_block, bsblk.into(), staptr);
        let roller = Mutex::new(roller);
        // engine
        let mut engine = ChainEngine {
            cnf,
            minter,
            disk,
            roller,
        };
        rebuild_unstable_blocks(&mut engine);
        engine
    }



}


impl EngineRead for ChainEngine {}

impl Engine for ChainEngine {
    fn as_read(&self) -> &dyn EngineRead {
        self
    }
}

