
// root curr 
type RollerInsertRet = Ret<(Option<Arc<Chunk>>, Option<Arc<Chunk>>, MemBatch)>;


impl Roller {

    
    pub fn insert(&mut self, parent: Arc<Chunk>, chunk: Chunk) -> RollerInsertRet {
        insert_to_roller(self, parent, chunk)
    }
}


/*
* return (change to new root, change to new pointer)
*/
pub fn insert_to_roller(roller: &mut Roller, parent: Arc<Chunk>, mut chunk: Chunk) -> RollerInsertRet {
    let new_hei = chunk.height;
    // check
    let root_hei = roller.root.height;
    let curr_hei = roller.curr.upgrade().unwrap().height;
    if new_hei <= root_hei || new_hei > curr_hei+1 {
        return errf!("insert height must between [{}, {}] but got {}", root_hei+1, curr_hei+1, new_hei)
    }
    // insert
    chunk.set_parent(parent.clone());
    let new_chunk = Arc::new(chunk);
    parent.push_child(new_chunk.clone());
    // move pointer
    let mut mv_root: Option<Arc<Chunk>> = None;
    let mut mv_curr: Option<Arc<Chunk>> = None;
    let mut tc_path = MemBatch::new();
    if new_hei > curr_hei {
        roller.curr = Arc::downgrade(&new_chunk); // update pointer
        mv_curr = Some(new_chunk.clone());
        // root
        let new_root_hei = match new_hei > roller.unstable && root_hei < new_hei-roller.unstable {
            true => root_hei + 1,
            false => 0, // first height
        };
        if new_root_hei > root_hei { // set new root
            let nrt = trace_upper_chunk(new_chunk, new_root_hei, &mut tc_path);
            roller.root = nrt.clone(); // update stat
            mv_root = Some(nrt);
        }
    }
    // return
    Ok((mv_root, mv_curr, tc_path))
}


// search back

fn trace_upper_chunk(mut seek: Arc<Chunk>, upper_hei: u64, tc_path: &mut MemBatch) -> Arc<Chunk> {
    let mut trc = |s: &Chunk| {
        tc_path.put(&BlockHeight::from(s.height).to_vec(), s.hash.as_ref());
    };
    while seek.height != upper_hei {
        trc(&seek);
        seek = seek.parent.upgrade().unwrap(); // must move to upper
    }
    trc(&seek);
    seek.clone() // ok find
}


