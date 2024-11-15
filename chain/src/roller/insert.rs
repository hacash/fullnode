

/*
* return (change to new root, change to new pointer)
*/
pub fn insert_to_roller(roller: &mut Roller, mut chunk: Chunk) -> Ret<(Option<Arc<Chunk>>, Option<Arc<Chunk>>)> {
    let new_hei = chunk.height;
    // check
    let root_hei = roller.root.height;
    let curr_hei = roller.chunk.upgrade().unwrap().height;
    if new_hei <= root_hei || new_hei > curr_hei+1 {
        return errf!("insert height must between [{}, {}] but got {}", root_hei+1, curr_hei+1, new_hei)
    }
    // search
    let phei = new_hei - 1;
    let phx = chunk.block.prevhash();
    let Some(parent) = roller.fast_search(phei, phx) else {
        return errf!("parent block <{}, {}> not find", phei, phx.to_hex())
    };
    // insert
    chunk.set_parent(parent.clone());
    let new_chunk = Arc::new(chunk);
    parent.push_child(new_chunk.clone());
    // update state
    let mut new_root: Option<Arc<Chunk>> = None;
    let mut new_pointer: Option<Arc<Chunk>> = None;
    if new_hei > curr_hei {
        roller.state = Arc::downgrade(&new_chunk.state()); // update stat
        roller.chunk = Arc::downgrade(&new_chunk); // update stat
        new_pointer = Some(new_chunk.clone());
        // maybe new root
        let new_root_hei = curr_hei - roller.unstable;
        if new_root_hei > root_hei {
            // find new root
            let nrt = back_to_parent_chunk(new_chunk, new_root_hei);
            roller.root = nrt.clone(); // update stat
            new_root = Some(nrt);
        }
    }
    // return
    Ok((new_root, new_pointer))
}


// search back
fn back_to_parent_chunk(curr: Arc<Chunk>, tarhei: u64) -> Arc<Chunk> {
    if curr.height == tarhei {
        return curr
    }
    let parent = curr.parent.upgrade().unwrap();
    back_to_parent_chunk(parent, tarhei)
}


