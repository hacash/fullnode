//! Sync peer lock + remote height tracking (mainnet SyncTracker).

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct SyncState {
    pub active_peer: Option<String>,
    pub next_height: u64,
    pub remote_height: u64,
    pub updated_at: Instant,
    /// True while a v1 serial BLOCK batch is being applied.
    pub applying: bool,
}

pub struct SyncTracker {
    inner: Mutex<Option<SyncState>>,
    /// A peer that produced a terminal sync error must not immediately start a
    /// fresh session from a late STATUS or MSG_BLOCK response.
    halted_peers: Mutex<HashSet<String>>,
}

impl SyncTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            halted_peers: Mutex::new(HashSet::new()),
        }
    }

    /// Acquire or refresh sync lock for `peer_id`. Returns false if another peer holds it (<10s).
    pub fn begin_or_refresh(&self, peer_id: &str, start_height: u64, remote_height: u64) -> bool {
        if self.halted_peers.lock().unwrap().contains(peer_id) {
            return false;
        }
        let mut sync = self.inner.lock().unwrap();
        let now = Instant::now();
        if let Some(st) = sync.as_mut() {
            if let Some(ref pk) = st.active_peer {
                if pk != peer_id && now.duration_since(st.updated_at).as_secs() < 10 {
                    return false;
                }
            }
            st.active_peer = Some(peer_id.to_string());
            st.next_height = start_height;
            st.remote_height = remote_height.max(st.remote_height);
            st.updated_at = now;
            return true;
        }
        *sync = Some(SyncState {
            active_peer: Some(peer_id.to_string()),
            next_height: start_height,
            remote_height,
            updated_at: now,
            applying: false,
        });
        true
    }

    /// Within the doing_sync throttle window: only refresh if `peer_id` already owns the lock.
    pub fn refresh_if_active(&self, peer_id: &str, start_height: u64, remote_height: u64) -> bool {
        if self.halted_peers.lock().unwrap().contains(peer_id) {
            return false;
        }
        let mut sync = self.inner.lock().unwrap();
        let Some(st) = sync.as_mut() else {
            return false;
        };
        if st.active_peer.as_deref() != Some(peer_id) {
            return false;
        }
        st.next_height = start_height;
        st.remote_height = remote_height.max(st.remote_height);
        st.updated_at = Instant::now();
        true
    }

    /// Claim the next expected v1 batch so concurrent MSG_BLOCK tasks are rejected.
    pub fn claim_batch(&self, peer_id: &str, start_height: u64) -> bool {
        if self.halted_peers.lock().unwrap().contains(peer_id) {
            return false;
        }
        let mut sync = self.inner.lock().unwrap();
        let Some(st) = sync.as_mut() else {
            return false;
        };
        if st.active_peer.as_deref() != Some(peer_id) {
            return false;
        }
        if st.applying || st.next_height != start_height {
            return false;
        }
        st.applying = true;
        st.updated_at = Instant::now();
        true
    }

    pub fn release_batch(&self, peer_id: &str) {
        let mut sync = self.inner.lock().unwrap();
        let Some(st) = sync.as_mut() else {
            return;
        };
        if st.active_peer.as_deref() == Some(peer_id) {
            st.applying = false;
        }
    }

    pub fn finish_if_done(&self, peer_id: &str, next_height: u64, remote_height: u64) {
        let mut sync = self.inner.lock().unwrap();
        let Some(st) = sync.as_mut() else {
            return;
        };
        if st.active_peer.as_deref() != Some(peer_id) {
            return;
        }
        st.next_height = next_height;
        st.remote_height = st.remote_height.max(remote_height);
        st.updated_at = Instant::now();
        st.applying = false;
        if next_height > remote_height {
            *sync = None;
        }
    }

    pub fn active_remote_height(&self) -> Option<u64> {
        self.inner.lock().ok()?.as_ref().map(|s| s.remote_height)
    }

    pub fn clear_peer(&self, peer_id: &str) {
        let mut sync = self.inner.lock().unwrap();
        if sync.as_ref().and_then(|s| s.active_peer.as_deref()) == Some(peer_id) {
            *sync = None;
        }
    }

    /// Mark a peer terminally failed for this process. This deliberately does
    /// not clear itself on another STATUS, otherwise late legacy replies can
    /// recreate the failed pipeline immediately.
    pub fn halt_peer(&self, peer_id: &str) {
        self.halted_peers.lock().unwrap().insert(peer_id.to_owned());
        self.clear_peer(peer_id);
    }
}

impl Default for SyncTracker {
    fn default() -> Self {
        Self::new()
    }
}
