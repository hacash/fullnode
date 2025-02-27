



struct Chunk {

    height: u64, // block height
    hash: Hash,

    block: Arc<dyn Block>,
    state: Arc<dyn State>,

    childs: Mutex<Vec<Arc<Chunk>>>,
    parent: Weak<Chunk>,

}

/*
impl Drop for Chunk {
    fn drop(&mut self) {
        println!("Chunk.drop({})", self.block.height());
    }
}
*/


impl Chunk {

    fn create(h: Hash, b: Arc<dyn Block>, s: Arc<dyn State>) -> Self {
        Self {
            height: b.height().uint(),
            hash: h,
            block: b,
            state: s,
            childs: Mutex::default(),
            parent: Weak::new(), // none
        }
    }

    fn push_child(&self, c: Arc<Chunk>) {
        self.childs.lock().unwrap().push(c);
    }

    fn set_parent(&mut self, p: Arc<Chunk>) {
        self.parent = Arc::downgrade(&p).into();
    }

}
