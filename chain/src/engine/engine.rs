
#[allow(dead_code)]
pub struct ChainEngine {
    cnf: EngineConf,
    // 
    minter: Box<dyn Minter>,
    scaner: Box<dyn Scaner>,

    // data
    disk: Arc<dyn DiskDB>,
    blockdisk: BlockDisk,

    roller: Mutex<Roller>,

    isrtlk: Mutex<()>, // is exit

    rctblks: Mutex<VecDeque<Arc<RecentBlockInfo>>>,
    avgfees: Mutex<VecDeque<u64>>,

}


impl ChainEngine {


    pub fn open(ini: &IniObj, dbv: u32,
        minter: Box<dyn Minter>,
        scaner: Box<dyn Scaner>
    ) -> ChainEngine {
        // init
        minter.init(ini); 
        // cnf
        let cnf = EngineConf::new(ini, dbv);
        let blk_dir = &cnf.block_data_dir;
        let sta_dir = &cnf.state_data_dir;
        let is_state_upgrade = !sta_dir.exists(); // not find new dir
        std::fs::create_dir_all(blk_dir).unwrap();
        std::fs::create_dir_all(sta_dir).unwrap();
        // build
        let disk = Arc::new(DiskKV::open(blk_dir));
        // if state database upgrade
        let sta_db =  DiskKV::open(sta_dir);
        let state = StateInst::build(Arc::new(sta_db), Weak::<StateInst>::new());
        let staptr = Arc::new(state);
        // base or genesis block
        let bsblk =  load_root_block(minter.as_ref(), disk.clone(), is_state_upgrade);
        let roller = Roller::create(cnf.unstable_block, bsblk, staptr);
        let roller = Mutex::new(roller);
        // engine
        let d1 = disk.clone();
        let mut engine = ChainEngine {
            cnf,
            minter,
            scaner,
            roller,
            disk,
            rctblks: Mutex::default(),
            avgfees: Mutex::default(),
            blockdisk: BlockDisk::wrap(d1),
            isrtlk: ().into(),
        };
        rebuild_unstable_blocks(&mut engine);
        engine
    }



}


