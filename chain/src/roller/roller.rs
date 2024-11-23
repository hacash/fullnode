

pub struct Roller {

    unstable: u64, // config unstable height = 4

    pub curr: Weak<Chunk>,     // current latest block

    pub root: Arc<Chunk>,       // tree root block
}


impl Roller {

    pub fn create(alive: u64, blk: Arc<dyn Block>, state: Arc<dyn State>) -> Roller {
        let chunk = Chunk::create(blk, state.clone());
        let ckptr = Arc::new(chunk);
        Roller {
            unstable: alive,
            curr: Arc::downgrade(&ckptr),
            root: ckptr,
        }
    }

    #[allow(dead_code)]
    pub fn root_height(&self) -> u64 {
        self.root.height
    }

    #[allow(dead_code)]
    pub fn last_height(&self) -> u64 {
        self.curr.upgrade().unwrap().height
    }


}