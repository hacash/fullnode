//! LRU knowledge set for P2P broadcast dedup (mainnet-compatible).

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

pub const KNOWLEDGE_SIZE: usize = 32;
pub type KnowKey = [u8; KNOWLEDGE_SIZE];

#[derive(Debug)]
pub struct Knowledge {
    size: usize,
    data: Mutex<KnowledgeInner>,
}

#[derive(Debug)]
struct KnowledgeInner {
    order: VecDeque<KnowKey>,
    set: HashSet<KnowKey>,
}

impl Knowledge {
    pub fn new(sz: usize) -> Knowledge {
        Knowledge {
            size: sz,
            data: Mutex::new(KnowledgeInner {
                order: VecDeque::with_capacity(sz + 1),
                set: HashSet::with_capacity(sz * 2 + 1),
            }),
        }
    }

    pub fn add(&self, key: KnowKey) {
        if self.size == 0 {
            return;
        }
        let mut dt = self.data.lock().unwrap();
        if dt.set.contains(&key) {
            return;
        }
        if dt.order.len() >= self.size {
            if let Some(old) = dt.order.pop_back() {
                dt.set.remove(&old);
            }
        }
        dt.order.push_front(key);
        dt.set.insert(key);
    }

    pub fn check(&self, key: &KnowKey) -> bool {
        self.data.lock().unwrap().set.contains(key)
    }
}
