

struct Roller {

    unstable: u64, // config unstable height = 4

    curr: Weak<Chunk>,     // current latest block

    root: Arc<Chunk>,       // tree root block
}


#[allow(dead_code)]
impl Roller {

    fn create(alive: u64, blk: Arc<dyn Block>, state: Arc<dyn State>) -> Roller {
        let chunk = Chunk::create(blk.hash(), blk, state.clone());
        let ckptr = Arc::new(chunk);
        Roller {
            unstable: alive,
            curr: Arc::downgrade(&ckptr),
            root: ckptr,
        }
    }

    fn root_height(&self) -> u64 {
        self.root.height
    }
    
    fn last_height(&self) -> u64 {
        self.curr.upgrade().unwrap().height
    }

}