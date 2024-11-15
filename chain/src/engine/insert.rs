
impl ChainEngine {
    
    fn do_insert(&self, block: BlockPkg) -> Rerr {
        let hei = block.hein;
        let hx = block.hash;
        // find prev chunk
        let prev_hei = block.hein - 1;
        let prev_hx = block.objc.prevhash();
        let prev = { 
            self.roller.lock().unwrap().fast_search(prev_hei, prev_hx) 
        };
        let Some(prev_chunk) = prev else {
            return errf!("not find prev block <{}, {}>", prev_hei, prev_hx)
        };
        // check repeat
        for sub in prev_chunk.childs.lock().unwrap().iter() {
            if hx == sub.hash {
                return errf!("repetitive block height {} hash {}", hei, hx)
            }
        }
        // 

        todo!()
    }

}


