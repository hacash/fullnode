
type RollerInsertRet = Ret<(Option<Arc<Chunk>>, Option<Arc<Chunk>>)>;


impl Roller {
    pub fn insert(&mut self, chunk: Chunk) -> RollerInsertRet {
        insert_to_roller(self, chunk)
    }
}


/*
* return (change to new root, change to new pointer)
*/
pub fn insert_to_roller(roller: &mut Roller, mut chunk: Chunk) -> RollerInsertRet {
    let new_hei = chunk.height;
    // check
    let root_hei = roller.root.height;
    let curr_hei = roller.curr.upgrade().unwrap().height;
    if new_hei <= root_hei || new_hei > curr_hei+1 {
        return errf!("insert height must between [{}, {}] but got {}", root_hei+1, curr_hei+1, new_hei)
    }
    // search
    let prev_hei = new_hei - 1;
    let prev_hx = chunk.block.prevhash();
    let Some(parent) = roller.fast_search(prev_hei, prev_hx) else {
        return errf!("parent block <{}, {}> not find", prev_hei, prev_hx.to_hex())
    };
    // insert
    chunk.set_parent(parent.clone());
    let new_chunk = Arc::new(chunk);
    parent.push_child(new_chunk.clone());
    // move pointer
    let mut mv_root: Option<Arc<Chunk>> = None;
    let mut mv_curr: Option<Arc<Chunk>> = None;
    if new_hei > curr_hei {
        roller.curr = Arc::downgrade(&new_chunk); // update pointer
        mv_curr = Some(new_chunk.clone());
        let new_root_hei = match curr_hei > roller.unstable {
            true => curr_hei - roller.unstable,
            false => 0, // first height
        };
        if new_root_hei > root_hei { // set new root
            let nrt = trace_upper_chunk(new_chunk, new_root_hei);
            roller.root = nrt.clone(); // update stat
            mv_root = Some(nrt);
        }
        
    }
    // return
    Ok((mv_root, mv_curr))
}


// search back
fn trace_upper_chunk(mut seek: Arc<Chunk>, upper_hei: u64) -> Arc<Chunk> {
    while seek.height != upper_hei {
        seek = seek.parent.upgrade().unwrap(); // must move to upper
    }
    seek.clone() // ok find
}


