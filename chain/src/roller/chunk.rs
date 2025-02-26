



pub struct Chunk {

    pub height: u64, // block height
    pub hash: Hash,

    pub block: Arc<dyn Block>,
    pub state: Arc<dyn State>,

    pub childs: Mutex<Vec<Arc<Chunk>>>,
    pub parent: Weak<Chunk>,

}

/*
impl Drop for Chunk {
    fn drop(&mut self) {
        println!("Chunk.drop({})", self.block.height());
    }
}
*/


impl Chunk {

    pub fn create(h: Hash, b: Arc<dyn Block>, s: Arc<dyn State>) -> Self {
        Self {
            height: b.height().uint(),
            hash: h,
            block: b,
            state: s,
            childs: Mutex::default(),
            parent: Weak::new(), // none
        }
    }

    pub fn push_child(&self, c: Arc<Chunk>) {
        self.childs.lock().unwrap().push(c);
    }

    pub fn set_parent(&mut self, p: Arc<Chunk>) {
        self.parent = Arc::downgrade(&p).into();
    }

}

