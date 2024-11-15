

pub struct Roller {

    unstable: u64, // config

    pub state: Weak<dyn State>, // current latest state
    pub chunk: Weak<Chunk>,     // current latest block

    pub root: Arc<Chunk>, // tree root block
}


impl Roller {

    pub fn create(alive: u64, blk: Arc<dyn Block>, state: Arc<dyn State>) -> Roller {
        let chunk = Chunk::create(blk, state.clone());
        let ckptr = Arc::new(chunk);
        Roller {
            unstable: alive,
            state: Arc::downgrade(&state),
            chunk: Arc::downgrade(&ckptr),
            root: ckptr,
        }
    }

    pub fn root_height(&self) -> u64 {
        self.root.height
    }

    pub fn last_height(&self) -> u64 {
        self.chunk.upgrade().unwrap().height
    }


}