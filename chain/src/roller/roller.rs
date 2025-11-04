

struct Roller {

    unstable: u64, // config unstable height = 4

    head: Weak<Chunk>,     // current latest block

    root: Arc<Chunk>,       // tree root block
}


#[allow(dead_code)]
impl Roller {

    fn create(alive: u64, blk: Arc<dyn Block>, state: Arc<Box<dyn State>>) -> Roller {
        let chunk = Chunk::create(blk.hash(), blk, state.clone());
        let ckptr = Arc::new(chunk);
        Roller {
            unstable: alive,
            head: Arc::downgrade(&ckptr),
            root: ckptr,
        }
    }

    fn root_height(&self) -> u64 {
        self.root.height
    }
    
    fn last_height(&self) -> u64 {
        self.head.upgrade().unwrap().height
    }

}