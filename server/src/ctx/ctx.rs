
/********************/

pub type ChainEngine = Arc<dyn Engine>;
pub type ChainNode = Arc<dyn HNode>;
pub type BlockCaches = Arc<Mutex<VecDeque<Arc<BlockPkg>>>>;

#[derive(Clone)]
pub struct ApiCtx {
    pub engine: ChainEngine,
    pub hcshnd: ChainNode,
    pub blocks: BlockCaches,
    pub miner_worker_notice_count: Arc<Mutex<u64>>,
    pub launch_time: u64,
    blocks_max: usize, // 4
}

impl ApiCtx {
    pub fn new(eng: ChainEngine, nd: ChainNode) -> ApiCtx {
        ApiCtx {
            engine: eng,
            hcshnd: nd,
            blocks: Arc::default(),
            miner_worker_notice_count: Arc::default(),
            launch_time: curtimes(),
            blocks_max: 4,
        }
    }

    pub fn load_block(&self, store: &BlockStore, key: &String) -> Ret<Arc<BlockPkg>> {
        self.load_block_from_cache(store, key, true)
    }

    
    // load block from cache or disk, key = height or hash
    pub fn load_block_from_cache(&self, store: &BlockStore, key: &String, with_cache: bool) -> Ret<Arc<BlockPkg>> {
        let mut hash = Hash::from([0u8; 32]);
        let mut height = BlockHeight::from(0);
        if key.len() == 64 {
            if let Ok(hx) = hex::decode(key) {
                hash = Hash::from(hx.try_into().unwrap());
            }
        }else{
            if let Ok(num) = key.parse::<u64>() {
                height = BlockHeight::from(num);
            }
        }
        // check cache
        if with_cache {
            let list = self.blocks.lock().unwrap();
            for blk in list.iter() {
                if height == *blk.objc.height() || hash == blk.hash {
                    return Ok(blk.clone())
                }
            }
        }
        // read from disk
        let blkdts;
        if *height > 0 {
            blkdts = store.block_data_by_height(&height).map(|(_,a)|a);
        }else{
            blkdts = store.block_data(&hash);
        }
        if let None = blkdts {
            return errf!("block not find")
        }
        let Ok(blkpkg) = BlockPkg::build(blkdts.unwrap()) else {
            return errf!("block parse error")
        };
        // ok
        let blkcp: Arc<BlockPkg> = blkpkg.into();
        if with_cache {
            let mut list = self.blocks.lock().unwrap();
            list.push_front(blkcp.clone());
            if list.len() > self.blocks_max {
                list.pop_back(); // cache limit 
            }
        }
        return Ok(blkcp)
    }
}


