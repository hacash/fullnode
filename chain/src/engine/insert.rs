
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
        let brothers: Vec<Arc<Chunk>> = {
            prev_chunk.childs.lock().unwrap().iter().map(|a|a.clone()).collect()
        };
        for sub in brothers {
            if hx == sub.hash {
                return errf!("repetitive block height {} hash {}", hei, hx)
            }
        }
        // create sub state 
        let prev_state = prev_chunk.state.clone();
        let mut sub_state = prev_state.fork_sub(prev_state.clone());
        // 
        


        todo!()
    }

}


