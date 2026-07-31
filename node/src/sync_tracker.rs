//! Sync peer lock + remote height tracking (mainnet SyncTracker).

use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use crate::topology::PeerKey;

const SYNC_TAKEOVER_IDLE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct SyncState {
    /// Exact TCP connection that owns in-flight responses.
    pub active_peer: Option<String>,
    /// Stable node identity used for dev-compatible takeover arbitration.
    pub active_node: Option<PeerKey>,
    pub next_height: u64,
    pub remote_height: u64,
    pub updated_at: Instant,
    /// True while a v1 serial BLOCK batch is being applied.
    pub applying: bool,
}

pub struct SyncTracker {
    inner: Mutex<Option<SyncState>>,
}

impl SyncTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Acquire or refresh the sync lock. Different connections for the same
    /// stable node key count as the same peer, matching fullnodedev.
    pub fn begin_or_refresh(
        &self,
        peer_id: &str,
        node_key: Option<PeerKey>,
        start_height: u64,
        remote_height: u64,
    ) -> bool {
        let mut sync = self.inner.lock().unwrap();
        let now = Instant::now();
        if let Some(st) = sync.as_mut() {
            if let Some(ref active_peer) = st.active_peer {
                let same_connection = active_peer == peer_id;
                let same_node = st.active_node.is_some() && st.active_node == node_key;
                if !same_connection
                    && !same_node
                    && now.duration_since(st.updated_at) < SYNC_TAKEOVER_IDLE
                {
                    return false;
                }
            }
            st.active_peer = Some(peer_id.to_string());
            st.active_node = node_key;
            st.next_height = start_height;
            st.remote_height = remote_height.max(st.remote_height);
            st.updated_at = now;
            return true;
        }
        *sync = Some(SyncState {
            active_peer: Some(peer_id.to_string()),
            active_node: node_key,
            next_height: start_height,
            remote_height,
            updated_at: now,
            applying: false,
        });
        true
    }

    /// Claim the next expected v1 batch so concurrent MSG_BLOCK tasks are rejected.
    pub fn claim_batch(&self, peer_id: &str, start_height: u64) -> bool {
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
}

impl Default for SyncTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SyncTracker;

    #[test]
    fn clear_releases_sync_for_another_peer_immediately() {
        let tracker = SyncTracker::new();
        assert!(tracker.begin_or_refresh("peer-a", Some([1; 16]), 10, 20));
        tracker.clear_peer("peer-a");
        assert!(tracker.begin_or_refresh("peer-b", Some([2; 16]), 10, 20));
    }

    #[test]
    fn active_peer_can_refresh_its_remote_tip() {
        let tracker = SyncTracker::new();
        assert!(tracker.begin_or_refresh("peer-a", Some([1; 16]), 10, 20));
        assert!(tracker.begin_or_refresh("peer-a", Some([1; 16]), 11, 30));
        assert_eq!(tracker.active_remote_height(), Some(30));
    }

    #[test]
    fn replacement_connection_for_same_node_can_refresh_immediately() {
        let tracker = SyncTracker::new();
        assert!(tracker.begin_or_refresh("old", Some([1; 16]), 10, 20));
        assert!(tracker.begin_or_refresh("new", Some([1; 16]), 11, 30));
        assert_eq!(
            tracker
                .inner
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .active_peer
                .as_deref(),
            Some("new")
        );
    }

    #[test]
    fn another_node_waits_until_the_dev_takeover_interval() {
        let tracker = SyncTracker::new();
        assert!(tracker.begin_or_refresh("peer-a", Some([1; 16]), 10, 20));
        assert!(!tracker.begin_or_refresh("peer-b", Some([2; 16]), 10, 20));

        tracker.inner.lock().unwrap().as_mut().unwrap().updated_at -= Duration::from_secs(10);
        assert!(tracker.begin_or_refresh("peer-b", Some([2; 16]), 10, 20));
    }
}
