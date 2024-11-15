



pub struct Chunk {

    height: u64, // block height
    hash: Hash,

    block: Arc<dyn Block>,
    state: Arc<dyn State>,

    childs: Mutex<Vec<Arc<Chunk>>>,
    parent: Weak<Chunk>,

}


impl Chunk {

    pub fn create(b: Arc<dyn Block>, s: Arc<dyn State>) -> Self {
        Self {
            height: b.height().to_uint(),
            hash: b.hash(),
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

    pub fn state(&self) -> Arc<dyn State> {
        self.state.clone()
    }

}

