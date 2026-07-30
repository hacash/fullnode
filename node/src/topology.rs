//! Mainnet-compatible topology distance (byte-ring, not XOR Kademlia).

pub type PeerKey = [u8; 16];

/// Per-byte ring distance used by mainnet DHT ordering.
pub fn byte_topology_distance(dst: u8, src: u8) -> u8 {
    let mut d = if dst > src {
        dst - src
    } else if dst < src {
        src - dst
    } else {
        0
    };
    if d > 128 {
        d = 128 - (d - 128);
    }
    d
}

/// Returns 1 if `left` is closer to `compare` than `right`, -1 if farther, 0 if equal.
pub fn compare_topology(compare: &PeerKey, left: &PeerKey, right: &PeerKey) -> i8 {
    for i in 0..compare.len() {
        let d1 = byte_topology_distance(compare[i], left[i]);
        let d2 = byte_topology_distance(compare[i], right[i]);
        if d1 < d2 {
            return 1;
        } else if d1 > d2 {
            return -1;
        }
    }
    0
}

/// Insert `item` into a DHT-ordered list keyed by `key_of`.
/// Returns the farthest peer if capacity exceeded.
pub fn insert_ordered<T, F>(
    list: &mut Vec<T>,
    max: usize,
    compare: &PeerKey,
    item: T,
    key_of: F,
) -> Option<T>
where
    F: Fn(&T) -> PeerKey,
{
    let key = key_of(&item);
    let mut idx = list.len();
    for i in 0..list.len() {
        if compare_topology(compare, &key, &key_of(&list[i])) == 1 {
            idx = i;
            break;
        }
    }
    list.insert(idx, item);
    if list.len() > max { list.pop() } else { None }
}

/// Candidate key must be closer to `compare` than `least` to enter the nearest list.
pub fn insert_nearest_key(
    list: &mut Vec<PeerKey>,
    compare: &PeerKey,
    least: &PeerKey,
    insert: &PeerKey,
) -> bool {
    if compare_topology(compare, insert, least) != 1 {
        return false;
    }
    let mut idx = list.len();
    for i in 0..list.len() {
        if compare_topology(compare, insert, &list[i]) == 1 {
            idx = i;
            break;
        }
    }
    list.insert(idx, *insert);
    true
}
