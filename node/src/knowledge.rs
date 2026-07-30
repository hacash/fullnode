//! LRU knowledge set for P2P broadcast dedup (mainnet-compatible).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const KNOWLEDGE_SIZE: usize = 32;
pub type KnowKey = [u8; KNOWLEDGE_SIZE];

#[derive(Debug)]
pub struct Knowledge {
    size: usize,
    data: Mutex<KnowledgeInner>,
}

/// Bounded negative cache for expensive admission failures. Entries are valid
/// only while the canonical tip is unchanged and for a short wall-clock TTL,
/// so state-dependent transactions and blocks become retryable after progress.
#[derive(Debug)]
pub struct RejectCache {
    size: usize,
    ttl: Duration,
    data: Mutex<RejectCacheInner>,
}

#[derive(Debug)]
struct RejectCacheInner {
    order: VecDeque<KnowKey>,
    entries: HashMap<KnowKey, RejectEntry>,
}

#[derive(Clone, Copy, Debug)]
struct RejectEntry {
    tip: KnowKey,
    expires_at: Instant,
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

impl RejectCache {
    pub fn new(size: usize, ttl: Duration) -> Self {
        Self {
            size,
            ttl,
            data: Mutex::new(RejectCacheInner {
                order: VecDeque::with_capacity(size.saturating_add(1)),
                entries: HashMap::with_capacity(size.saturating_mul(2).saturating_add(1)),
            }),
        }
    }

    pub fn contains(&self, key: &KnowKey, tip: &KnowKey) -> bool {
        let now = Instant::now();
        let mut data = self.data.lock().unwrap();
        let valid = data
            .entries
            .get(key)
            .is_some_and(|entry| entry.tip == *tip && now < entry.expires_at);
        if !valid && data.entries.remove(key).is_some() {
            data.order.retain(|known| known != key);
        }
        valid
    }

    pub fn add(&self, key: KnowKey, tip: KnowKey) {
        if self.size == 0 || self.ttl.is_zero() {
            return;
        }
        let mut data = self.data.lock().unwrap();
        let entry = RejectEntry {
            tip,
            expires_at: Instant::now() + self.ttl,
        };
        if let Some(current) = data.entries.get_mut(&key) {
            *current = entry;
            return;
        }
        while data.entries.len() >= self.size {
            let Some(old) = data.order.pop_front() else {
                break;
            };
            data.entries.remove(&old);
        }
        data.order.push_back(key);
        data.entries.insert(key, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::RejectCache;
    use std::time::Duration;

    #[test]
    fn reject_cache_is_scoped_to_canonical_tip() {
        let cache = RejectCache::new(2, Duration::from_secs(30));
        let key = [1; 32];
        let old_tip = [2; 32];
        let new_tip = [3; 32];

        cache.add(key, old_tip);
        assert!(cache.contains(&key, &old_tip));
        assert!(!cache.contains(&key, &new_tip));
    }

    #[test]
    fn reject_cache_is_bounded() {
        let cache = RejectCache::new(1, Duration::from_secs(30));
        let tip = [9; 32];

        cache.add([1; 32], tip);
        cache.add([2; 32], tip);
        assert!(!cache.contains(&[1; 32], &tip));
        assert!(cache.contains(&[2; 32], &tip));
    }
}
