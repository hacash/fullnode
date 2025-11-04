



struct Chunk {

    height: u64, // block height
    hash: Hash,

    block: Arc<dyn Block>,
    state: Arc<Box<dyn State>>,

    childs: Vec<Arc<Chunk>>,
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

    fn create(h: Hash, b: Arc<dyn Block>, s: Arc<Box<dyn State>>) -> Self {
        Self {
            height: b.height().uint(),
            hash: h,
            block: b,
            state: s,
            childs: Vec::new(),
            parent: Weak::new(), // none
        }
    }

    fn push_child(&mut self, c: Arc<Chunk>) {
        self.childs.push(c);
    }

    fn set_parent(&mut self, p: Arc<Chunk>) {
        self.parent = Arc::downgrade(&p).into();
    }

}


fn chunk_push_child(p: Arc<Chunk>, c: Arc<Chunk>) {
    let parent = unsafe { &mut *(Arc::as_ptr(&p) as *mut Chunk) };
    parent.push_child(c);
}

