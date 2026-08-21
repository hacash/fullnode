//! Sync peer ownership + remote height tracking: the `sync_session` slot is the
//! source of truth; this only remembers the owning peer and its tip so a fork-hash reply can estimate it. No takeover arbitration here — an active session is never displaced, the watchdog recovers stalled ones.

use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct SyncState {
    /// Peer that owns the last sync.
    pub active_peer: Option<String>,
    pub remote_height: u64,
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

    /// Record a sync owner; the caller has already established no session is
    /// active, so this always succeeds and simply replaces the previous record.
    pub fn begin(&self, peer_id: &str, remote_height: u64) {
        *self.inner.lock().unwrap() = Some(SyncState {
            active_peer: Some(peer_id.to_string()),
            remote_height,
        });
    }

    pub fn active_remote_height(&self) -> Option<u64> {
        self.inner.lock().ok()?.as_ref().map(|s| s.remote_height)
    }

    /// Refresh the recorded remote tip after a sync run, so later fork-hash
    /// replies can still estimate the peer's height.
    pub fn finish(&self, peer_id: &str, remote_height: u64) {
        let mut sync = self.inner.lock().unwrap();
        if sync.as_ref().and_then(|s| s.active_peer.as_deref()) == Some(peer_id) {
            sync.as_mut().unwrap().remote_height =
                sync.as_ref().unwrap().remote_height.max(remote_height);
        }
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
    use super::SyncTracker;

    #[test]
    fn clear_releases_sync_for_another_peer_immediately() {
        let tracker = SyncTracker::new();
        tracker.begin("peer-a", 20);
        tracker.clear_peer("peer-a");
        tracker.begin("peer-b", 30);
        assert_eq!(tracker.active_remote_height(), Some(30));
    }

    #[test]
    fn finish_keeps_the_max_remote_tip() {
        let tracker = SyncTracker::new();
        tracker.begin("peer-a", 20);
        tracker.finish("peer-a", 30);
        assert_eq!(tracker.active_remote_height(), Some(30));
        // A stale lower refresh never lowers the recorded tip.
        tracker.finish("peer-a", 15);
        assert_eq!(tracker.active_remote_height(), Some(30));
    }

    #[test]
    fn clear_peer_ignores_a_different_owner() {
        let tracker = SyncTracker::new();
        tracker.begin("peer-a", 20);
        tracker.clear_peer("peer-b");
        assert_eq!(tracker.active_remote_height(), Some(20));
    }
}
