
// root change, head change, store batch, block hash 
type RollerInsertResult = (Option<Arc<Chunk>>, Option<Arc<Chunk>>, MemBatch, Hash);
// old root hei, ... ,blk hx, blk data, is_sync
type RollerInsertResd = (Option<Arc<Chunk>>, Option<Arc<Chunk>>, Hash);
type RollerInsertData = (u64, Option<Arc<Chunk>>, Option<Arc<Chunk>>, Hash, Vec<u8>, bool);


impl Roller {

    // fn try_insert(&self) {  }

    fn insert(&mut self, parent: Arc<Chunk>, chunk: Chunk) -> Ret<RollerInsertResult> {
        insert_to_roller(self, parent, chunk)
    }

    // just_ckhd just check head
    fn insert_v2(&mut self, parent: Arc<Chunk>, mut chunk: Chunk) -> Ret<RollerInsertResd> {
        let chunk_hei = chunk.height;
        // check
        let old_root_hei = self.root.height;
        let old_head = self.head.upgrade().unwrap();
        let old_head_hei = old_head.height;
        if chunk_hei <= old_root_hei || chunk_hei > old_head_hei+1 {
            return errf!("insert height must between [{}, {}] but got {}", old_root_hei+1, old_head_hei+1, chunk_hei)
        }
        // insert
        let hx = chunk.hash;
        chunk.set_parent(parent.clone());
        let new_chunk = Arc::new(chunk);
        chunk_push_child(parent, new_chunk.clone());
        // move pointer
        let mut mv_root: Option<Arc<Chunk>> = None;
        let mut mv_head: Option<Arc<Chunk>> = None;
        // let mut tc_path = MemBatch::new();
        if chunk_hei > old_head_hei {
            let new_head = new_chunk.clone();
            self.head = Arc::downgrade(&new_head); // update pointer
            mv_head = Some(new_head.clone());
            // root
            let new_root_hei = match chunk_hei > self.unstable && old_root_hei < chunk_hei-self.unstable {
                true => old_root_hei + 1,
                false => 0, // first height
            };
            if new_root_hei > old_root_hei { // set new root
                let nrt = trace_upper_chunk_v2(new_head, new_root_hei);
                self.root = nrt.clone(); // update stat
                mv_root = Some(nrt);
            }
        }
        // return
        Ok((mv_root, mv_head, hx))

    }

















}



/*
* return (change to new root, change to new pointer)
*/
fn insert_to_roller(roller: &mut Roller, parent: Arc<Chunk>, mut chunk: Chunk) -> Ret<RollerInsertResult> {
    let new_hei = chunk.height;
    // check
    let root_hei = roller.root.height;
    let curr_hei = roller.head.upgrade().unwrap().height;
    if new_hei <= root_hei || new_hei > curr_hei+1 {
        return errf!("insert height must between [{}, {}] but got {}", root_hei+1, curr_hei+1, new_hei)
    }
    // insert
    let hx = chunk.hash;
    chunk.set_parent(parent.clone());
    let new_chunk = Arc::new(chunk);
    chunk_push_child(parent, new_chunk.clone());
    // move pointer
    let mut mv_root: Option<Arc<Chunk>> = None;
    let mut mv_curr: Option<Arc<Chunk>> = None;
    let mut tc_path = MemBatch::new();
    if new_hei > curr_hei {
        roller.head = Arc::downgrade(&new_chunk); // update pointer
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
    Ok((mv_root, mv_curr, tc_path, hx))
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


fn trace_upper_chunk_v2(mut seek: Arc<Chunk>, upper_hei: u64) -> Arc<Chunk> {
    while seek.height != upper_hei {
        seek = seek.parent.upgrade().unwrap(); // must move to upper
    }
    seek.clone() // ok find
}


