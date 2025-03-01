

impl Roller {

    
    fn search(&self, hei: u64, hx: &Hash) -> Option<Arc<Chunk>> {
        // search least current
        let cur = self.head.upgrade().unwrap(); // must have
        if cur.height == hei && cur.hash == *hx {
            return Some(cur.clone())
        }
        let root = &self.root; 
        if hei < root.height {
            return None // height too low
        }
        // or search from root
        search_chunk_tree(root.clone(), hei, hx)
    }

}



fn search_chunk_tree(chunk: Arc<Chunk>, hei: u64, hx: &Hash) -> Option<Arc<Chunk>> {
    if chunk.height == hei && chunk.hash == *hx {
        return Some(chunk.clone()) // find it
    }
    // search childs
    for a in chunk.childs.iter() {
        if let Some(r) = search_chunk_tree(a.clone(), hei, hx) {
            return Some(r)
        }
    }
    // not find
    None
}
