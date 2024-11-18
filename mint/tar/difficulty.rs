

#[derive(Clone)]
pub struct DifficultyGnr {
    cnf: MintConf,
    block_caches: Arc<Mutex<HashMap<u64,(u64,u32,[u8; HXS])>>>, // height => (time, diffhx) 
}

impl DifficultyGnr {

    pub fn new(cnf: MintConf) -> DifficultyGnr {
        DifficultyGnr {
            cnf: cnf,
            block_caches: Arc::default(),
        }
    }

}





impl DifficultyGnr {

    pub fn req_cycle_block(&self, hei: u64, sto: Arc<dyn DiskDB>) -> (u64, u32, [u8; HXS]) {
        let cylnum = self.cnf.difficulty_adjust_blocks; // 288
        if hei < cylnum {
            let cyltime = genesis_block().timestamp().uint();
            let diffcty = genesis_block().difficulty().uint();
            let diffhx = u32_to_hash(diffcty);
            return (cyltime, diffcty, diffhx)
        }
        let cylhei = hei / cylnum * cylnum;
        let mut cache = self.block_caches.lock().unwrap();
        if let Some(blk_time) = cache.get(&cylhei) {
            return *blk_time // find in cache
        }
        // read from database
        let store = BlockDisk::wrap(sto);
        let (_, blkdts) = store.block_data_by_height(&BlockHeight::from(cylhei)).unwrap();
        let mut intro = BlockIntro::default();
        intro.parse(&blkdts).unwrap();
        // get time
        let cyltime = intro.timestamp().uint();
        let diffcty = intro.difficulty().uint();
        let diffhx = u32_to_hash(diffcty);
        let ccitem = (cyltime, diffcty, diffhx);
        cache.insert(cylhei, ccitem);
        if cache.len() as u64 > cylnum {
            cache.clear(); // clear
        }
        // ok
        ccitem
    }

}