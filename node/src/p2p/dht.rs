
/**
* return: exist peer
*/
fn check_exist_in_dht_list(lklist: PeerList, peer: &Peer) -> Option<Arc<Peer>> {
    let mut list = lklist.lock().unwrap();
    for p in list.iter() {
        if p.key == peer.key || p.addr == peer.addr{
            return Some(p.clone()) // repeat peer
        }
    }
    // not find
    return None
}


/**
 * remove one from 
 */
fn checkout_one_from_dht_list<F>(lklist: PeerList, choose: F) -> Option<Arc<Peer>>
where
    F: Fn(&Peer) -> bool,
{
    let mut rmid = -1isize;
    let mut list = lklist.lock().unwrap();
    for i in 0..list.len() {
        if choose(&list[i]) {
            rmid = i as isize;
            break
        }
    }
    if rmid == -1 {
        return None // not match
    }
    // ok checkout
    Some(list.remove(rmid as usize))
}


/**
 * insert to dht list
 * return: insert success
 */
fn insert_nearest_to_dht_list(list: &mut Vec<PeerKey>, compare: &PeerKey, least: &PeerKey, insert: &PeerKey) -> bool {
    if 1 != compare_peer_id_topology_distance(compare, insert, least) {
        return false // not 
    }
    let lenght = list.len();
    if 0 == lenght {
        list.push(*insert);
        return true
    }
    let mut istidx = lenght;
    for i in 0..lenght {
        let disnum = compare_peer_id_topology_distance(compare, insert, &list[i]);
        if disnum == 1 {
            istidx = i;
            break;
        }
    }
    // insert
    list.insert(istidx, *insert);
    return true // ok
}


/**
 * return: maybe drop
 */
fn remove_peer_from_dht_list(lklist: PeerList, peer: Arc<Peer>) -> bool {
    let key = peer.key;
    let mut rmid = -1isize;
    let mut list = lklist.lock().unwrap();
    for i in 0..list.len() {
        if key == list[i].key {
            rmid = i as isize;
            break
        }
    }
    // rm 
    if rmid >=0 {
        list.remove(rmid as usize);
        return true;
    }
    // not find
    false
}

/**
 * find
 */
 fn find_peer_from_dht_list(lklist: PeerList, pk: &PeerKey) -> Option<Arc<Peer>> {
    let mut list = lklist.lock().unwrap();
    for i in 0..list.len() {
        if *pk == list[i].key {
            return Some(list[i].clone())
        }
    }
    // not find
    None
}


/**
 * return: maybe drop one
 */
fn insert_peer_to_dht_list(lklist: PeerList, max: usize, 
    compare: &PeerKey, peer: Arc<Peer>
) -> Option<Arc<Peer>> {

    let mut list = lklist.lock().unwrap();
    let length = list.len();
    let mut insert_idx = length;
    for i in 0..length {
        let disnum = compare_peer_id_topology_distance(compare, &peer.key, &list[i].key);
        if disnum == 1 {
            insert_idx = i;
            break;
        }
    }
    // insert
    list.insert(insert_idx, peer);
    // drop the tail
    if list.len() > max {
        return list.pop()
    }
    // isert ok no drop
    None
}


/**
 * left closer ret 1
 * right closer ret -1
 * same ret 0
 */
fn compare_peer_id_topology_distance(compare: &PeerKey, left: &PeerKey, right: &PeerKey) -> i8 {
    for i in 0..compare.len() {
		let ds1 = calculate_one_byte_topology_distance(compare[i], left[i]);
		let ds2 = calculate_one_byte_topology_distance(compare[i], right[i]);
		if ds1 < ds2 {
			return 1
		} else if ds1 > ds2 {
			return -1
		}
		// diff next byte
	}
    // same
	return 0
}


pub fn calculate_one_byte_topology_distance(dst: u8, src: u8) -> u8 {
    let mut disnum = 0;
    if dst > src {
        disnum = dst - src
    }else if dst < src {
        disnum = src - dst
    }
    if disnum > 128 {
        disnum = 128 - (disnum - 128);
    }
    return disnum
}