//! Public address book: topology-ordered, max 200; backs `stable.nodes`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::RwLock;

use crate::topology::{PeerKey, compare_topology};

const STABLE_EXPIRE_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrSource {
    Boot,
    Stable,
    Find,
    Eject,
}

#[derive(Clone, Debug)]
pub struct AddrEntry {
    pub key: Option<PeerKey>,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    pub fail_count: u32,
    pub cooldown_until: Option<Instant>,
    pub source: AddrSource,
}

struct BookInner {
    /// Topology-ordered (nearest first) relative to `my_key`.
    entries: Vec<AddrEntry>,
}

pub struct AddrBook {
    inner: RwLock<BookInner>,
    my_key: PeerKey,
    max: usize,
    data_dir: String,
    persist_max: usize,
}

impl AddrBook {
    pub fn new(my_key: PeerKey, max: usize, data_dir: String, persist_max: usize) -> Self {
        Self {
            inner: RwLock::new(BookInner {
                entries: Vec::new(),
            }),
            my_key,
            max: max.max(1),
            data_dir,
            persist_max: persist_max.max(1),
        }
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.entries.len()
    }

    pub async fn insert(&self, key: Option<PeerKey>, addr: SocketAddr, source: AddrSource) -> bool {
        if addr.ip().is_loopback() {
            return false;
        }
        let mut g = self.inner.write().await;
        // Dedupe by addr; refresh if known.
        if let Some(pos) = g.entries.iter().position(|e| e.addr == addr) {
            let e = &mut g.entries[pos];
            e.last_seen = Instant::now();
            if key.is_some() {
                e.key = key;
            }
            e.source = source;
            e.fail_count = 0;
            e.cooldown_until = None;
            return true;
        }
        if let Some(k) = key {
            if g.entries.iter().any(|e| e.key == Some(k)) {
                // Same key, different addr — replace addr.
                if let Some(e) = g.entries.iter_mut().find(|e| e.key == Some(k)) {
                    e.addr = addr;
                    e.last_seen = Instant::now();
                    e.source = source;
                    e.fail_count = 0;
                    e.cooldown_until = None;
                    return true;
                }
            }
        }
        let entry = AddrEntry {
            key,
            addr,
            last_seen: Instant::now(),
            fail_count: 0,
            cooldown_until: None,
            source,
        };
        let idx = insert_index(&g.entries, &self.my_key, &entry);
        g.entries.insert(idx, entry);
        while g.entries.len() > self.max {
            g.entries.pop();
        }
        true
    }

    pub async fn note_fail(&self, addr: SocketAddr) {
        let mut g = self.inner.write().await;
        if let Some(e) = g.entries.iter_mut().find(|e| e.addr == addr) {
            e.fail_count = e.fail_count.saturating_add(1);
            let backoff_secs = match e.fail_count {
                1 => 60,
                2 => 5 * 60,
                3 => 15 * 60,
                _ => 60 * 60,
            };
            e.cooldown_until = Some(Instant::now() + Duration::from_secs(backoff_secs));
        }
    }

    pub async fn note_success(&self, addr: SocketAddr) {
        let mut g = self.inner.write().await;
        if let Some(e) = g.entries.iter_mut().find(|e| e.addr == addr) {
            e.fail_count = 0;
            e.cooldown_until = None;
            e.last_seen = Instant::now();
        }
    }

    /// Topology-ordered dial candidates not in `exclude_addrs`, skipping cooldown.
    pub async fn dial_candidates(
        &self,
        exclude_addrs: &[SocketAddr],
        exclude_keys: &[PeerKey],
        limit: usize,
    ) -> Vec<SocketAddr> {
        let now = Instant::now();
        let g = self.inner.read().await;
        let mut out = Vec::new();
        for e in &g.entries {
            if out.len() >= limit {
                break;
            }
            if exclude_addrs.contains(&e.addr) {
                continue;
            }
            if let Some(k) = e.key {
                if exclude_keys.iter().any(|x| *x == k) {
                    continue;
                }
            }
            if let Some(until) = e.cooldown_until {
                if now < until {
                    continue;
                }
            }
            out.push(e.addr);
        }
        out
    }

    pub async fn addrs_for_persist(&self) -> Vec<SocketAddr> {
        let g = self.inner.read().await;
        g.entries
            .iter()
            .take(self.persist_max)
            .map(|e| e.addr)
            .collect()
    }

    pub async fn load_from_stable(&self) {
        if self.data_dir.is_empty() {
            return;
        }
        for addr in read_stable_file(&self.data_dir, self.persist_max) {
            self.insert(None, addr, AddrSource::Stable).await;
        }
    }

    pub async fn persist_async(&self) {
        if self.data_dir.is_empty() {
            return;
        }
        let addrs = self.addrs_for_persist().await;
        let dir = self.data_dir.clone();
        tokio::task::spawn_blocking(move || {
            write_stable_file(&dir, &addrs);
        });
    }
}

fn entry_sort_key(e: &AddrEntry) -> PeerKey {
    e.key.unwrap_or([0xff; 16])
}

fn insert_index(entries: &[AddrEntry], my_key: &PeerKey, neu: &AddrEntry) -> usize {
    let nk = entry_sort_key(neu);
    for (i, e) in entries.iter().enumerate() {
        if compare_topology(my_key, &nk, &entry_sort_key(e)) == 1 {
            return i;
        }
    }
    entries.len()
}

fn stable_path(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("stable.nodes")
}

fn expired(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    elapsed.as_secs() >= STABLE_EXPIRE_SECS
}

pub fn read_stable_file(data_dir: &str, max: usize) -> Vec<SocketAddr> {
    if max == 0 || data_dir.is_empty() {
        return vec![];
    }
    let path = stable_path(data_dir);
    if expired(&path) {
        let _ = std::fs::remove_file(&path);
        return vec![];
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(addr) = line.parse::<SocketAddr>() else {
            continue;
        };
        if addr.ip().is_loopback() {
            continue;
        }
        if seen.insert(addr) {
            out.push(addr);
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

pub fn write_stable_file(data_dir: &str, addrs: &[SocketAddr]) {
    if data_dir.is_empty() {
        return;
    }
    let path = stable_path(data_dir);
    let mut out = String::new();
    for a in addrs {
        out.push_str(&format!("{}\n", a));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut tmp = path.clone();
    tmp.set_extension("nodes.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}
