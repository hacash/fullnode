

impl Roller {

    pub fn fast_search(&self, hei: u64, hx: &Hash) -> Option<Arc<Chunk>> {
        // search least current
        let root = self.chunk.upgrade().unwrap(); // must have
        if root.height == hei && root.hash == *hx {
            return Some(root.clone())
        }
        if hei <= root.height {
            return None // height too low
        }
        // or search from root
        search_chunk_tree(self.root.clone(), hei, hx)
    }

}


pub fn search_chunk_tree(chunk: Arc<Chunk>, hei: u64, hx: &Hash) -> Option<Arc<Chunk>> {
    if chunk.hash == *hx {
        return Some(chunk.clone())
    }
    // search childs
    let childs = {
        chunk.childs.lock().unwrap().clone()
    };
    for a in childs {
        if let Some(r) = search_chunk_tree(a, hei, hx) {
            return Some(r)
        }
    }
    // not find
    None
}
