//! Live connections: backbone + offshoot, using fullnodedev's DHT table rules.

use std::collections::HashMap;
use std::sync::{Arc, RwLock as StdRwLock};

use tokio::sync::RwLock;

use crate::p2p::peer::RemotePeer;
use crate::topology::{PeerKey, insert_ordered};

#[derive(Default)]
struct Tables {
    by_id: HashMap<String, Arc<RemotePeer>>,
    backbones: Vec<Arc<RemotePeer>>,
    offshoots: Vec<Arc<RemotePeer>>,
}

pub struct PeerTable {
    inner: RwLock<Tables>,
    /// Synchronous broadcast readers use this published snapshot instead of
    /// dropping delivery when the async table lock is briefly contended.
    snapshot: StdRwLock<Vec<Arc<RemotePeer>>>,
    my_key: PeerKey,
    backbone_max: usize,
    offshoot_max: usize,
}

pub struct InsertOutcome {
    /// The exact connection instance replaced for the same stable node key.
    pub replaced: Option<Arc<RemotePeer>>,
    pub drop_later: Vec<Arc<RemotePeer>>,
    pub backbone_changed: bool,
}

impl PeerTable {
    pub fn new(my_key: PeerKey, backbone_max: usize, offshoot_max: usize) -> Self {
        Self {
            inner: RwLock::new(Tables::default()),
            snapshot: StdRwLock::new(Vec::new()),
            my_key,
            backbone_max,
            offshoot_max,
        }
    }

    pub async fn backbones(&self) -> Vec<Arc<RemotePeer>> {
        self.inner.read().await.backbones.clone()
    }

    pub async fn offshoots(&self) -> Vec<Arc<RemotePeer>> {
        self.inner.read().await.offshoots.clone()
    }

    pub async fn all_peers(&self) -> Vec<Arc<RemotePeer>> {
        self.inner.read().await.by_id.values().cloned().collect()
    }

    /// Publics for node discovery include both tables, as in fullnodedev.
    pub async fn publics(&self) -> Vec<Arc<RemotePeer>> {
        let t = self.inner.read().await;
        t.backbones
            .iter()
            .chain(t.offshoots.iter())
            .filter(|p| p.is_public() && !p.addr.ip().is_loopback())
            .cloned()
            .collect()
    }

    /// Remove only this connection instance. A replaced connection has the
    /// same node id and must not remove its replacement when its reader exits.
    pub async fn remove(&self, peer: &Arc<RemotePeer>) -> bool {
        let mut t = self.inner.write().await;
        if !t
            .by_id
            .get(&peer.id)
            .is_some_and(|current| Arc::ptr_eq(current, peer))
        {
            return false;
        }
        t.by_id.remove(&peer.id);
        t.backbones.retain(|p| !Arc::ptr_eq(p, peer));
        t.offshoots.retain(|p| !Arc::ptr_eq(p, peer));
        self.publish_snapshot(&t);
        true
    }

    /// Insert using fullnodedev's rules:
    /// - replace the same node key atomically and close the old connection;
    /// - public peers enter backbone in DHT order;
    /// - an inbound public evicted from backbone is retained in offshoot;
    /// - outbound public and offshoot overflow peers are closed after 15s.
    pub async fn insert(&self, peer: Arc<RemotePeer>) -> InsertOutcome {
        let mut t = self.inner.write().await;
        let mut drop_later = Vec::new();
        let mut backbone_changed = false;
        let key = peer.node_key;

        let replaced = t
            .backbones
            .iter()
            .chain(t.offshoots.iter())
            .find(|old| old.node_key == key)
            .cloned();
        if let Some(old) = replaced.as_ref() {
            t.by_id.remove(&old.id);
            old.disconnect();
            let before = t.backbones.len();
            t.backbones.retain(|p| p.node_key != key);
            t.offshoots.retain(|p| p.node_key != key);
            if t.backbones.len() != before {
                backbone_changed = true;
            }
        }

        if peer.is_public() {
            backbone_changed = true;
            if let Some(droped) = insert_ordered(
                &mut t.backbones,
                self.backbone_max,
                &self.my_key,
                peer,
                |p| p.node_key,
            ) {
                if !droped.is_inbound() {
                    drop_later.push(droped);
                } else {
                    let duplicate = t
                        .backbones
                        .iter()
                        .chain(t.offshoots.iter())
                        .any(|p| p.node_key == droped.node_key);
                    if duplicate {
                        droped.disconnect();
                    } else if let Some(far) = insert_ordered(
                        &mut t.offshoots,
                        self.offshoot_max,
                        &self.my_key,
                        droped,
                        |p| p.node_key,
                    ) {
                        drop_later.push(far);
                    }
                }
            }
        } else if let Some(far) = insert_ordered(
            &mut t.offshoots,
            self.offshoot_max,
            &self.my_key,
            peer,
            |p| p.node_key,
        ) {
            drop_later.push(far);
        }

        t.by_id = t
            .backbones
            .iter()
            .chain(t.offshoots.iter())
            .map(|p| (p.id.clone(), p.clone()))
            .collect();
        self.publish_snapshot(&t);

        InsertOutcome {
            replaced,
            drop_later,
            backbone_changed,
        }
    }

    /// Promote the first public offshoot when backbone has an empty slot.
    pub async fn boost_public(&self) -> bool {
        let mut t = self.inner.write().await;
        if t.backbones.len() >= self.backbone_max {
            return false;
        }
        let Some(index) = t.offshoots.iter().position(|peer| peer.is_public()) else {
            return false;
        };
        let peer = t.offshoots.remove(index);
        let dropped = insert_ordered(
            &mut t.backbones,
            self.backbone_max,
            &self.my_key,
            peer,
            |p| p.node_key,
        );
        debug_assert!(dropped.is_none());
        self.publish_snapshot(&t);
        true
    }

    pub fn try_all_prints(&self) -> Vec<String> {
        self.values_snapshot()
            .into_iter()
            .map(|peer| peer.nick())
            .collect()
    }

    pub fn get_snapshot(&self, id: &str) -> Option<Arc<RemotePeer>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|peer| peer.id == id)
            .cloned()
    }

    #[cfg(test)]
    pub fn get_by_key_snapshot(&self, key: &PeerKey) -> Option<Arc<RemotePeer>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|peer| peer.node_key == *key)
            .cloned()
    }

    pub fn values_snapshot(&self) -> Vec<Arc<RemotePeer>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Prefer the current public peer; otherwise choose the first public peer
    /// in backbone/offshoot order, matching fullnodedev's switch_peer.
    pub fn try_switch_peer(&self, prefer_id: &str) -> Option<Arc<RemotePeer>> {
        let peers = self.values_snapshot();
        let mut publics = peers.iter().filter(|peer| peer.is_public());
        if let Some(p) = publics.clone().find(|p| p.id == prefer_id) {
            return Some(p.clone());
        }
        publics.next().cloned()
    }

    fn publish_snapshot(&self, tables: &Tables) {
        let peers = tables
            .backbones
            .iter()
            .chain(tables.offshoots.iter())
            .cloned()
            .collect();
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = peers;
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use tokio::sync::{Notify, mpsc};

    use super::PeerTable;
    use crate::knowledge::Knowledge;
    use crate::p2p::peer::{PeerWriteCmd, ProtocolVersion, RemotePeer};

    fn remote_peer(id: &str, key: u8, is_public: bool, is_inbound: bool) -> Arc<RemotePeer> {
        let (writer_tx, _writer_rx) = mpsc::channel::<PeerWriteCmd>(1);
        Arc::new(RemotePeer {
            id: id.to_owned(),
            node_key: [key; 16],
            name: String::new(),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), key as u16 + 1),
            listen_port: 1,
            is_public: AtomicBool::new(is_public),
            is_inbound: AtomicBool::new(is_inbound),
            last_active: Mutex::new(Instant::now()),
            protocol_version: AtomicU8::new(ProtocolVersion::V2.as_u8()),
            service_mask: AtomicU64::new(u64::MAX),
            relay: AtomicBool::new(true),
            custom_types: Vec::new(),
            writer_tx,
            close_notify: Arc::new(Notify::new()),
            closed: Arc::new(AtomicBool::new(false)),
            knows: Knowledge::new(1),
        })
    }

    #[test]
    fn broadcast_snapshot_survives_table_write_contention() {
        let table = PeerTable::new([0; 16], 1, 1);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            table.insert(remote_peer("peer-1", 1, false, true)).await;
            let _write = table.inner.write().await;
            let peers = table.values_snapshot();
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].id, "peer-1");
            assert_eq!(table.get_snapshot("peer-1").unwrap().id, "peer-1");
        });
    }

    #[tokio::test]
    async fn full_table_keeps_dht_nearest_peer() {
        let table = PeerTable::new([0; 16], 1, 1);
        let far = remote_peer("far", 20, false, true);
        let near = remote_peer("near", 1, false, true);
        assert!(table.insert(far.clone()).await.drop_later.is_empty());

        let outcome = table.insert(near.clone()).await;
        assert!(
            outcome
                .drop_later
                .iter()
                .any(|peer| Arc::ptr_eq(peer, &far))
        );
        let offshoots = table.offshoots().await;
        assert_eq!(offshoots.len(), 1);
        assert!(Arc::ptr_eq(&offshoots[0], &near));
    }

    #[tokio::test]
    async fn inbound_public_demotes_and_promotes_like_fullnodedev() {
        let table = PeerTable::new([0; 16], 1, 2);
        let far = remote_peer("far", 20, true, true);
        let near = remote_peer("near", 1, true, true);
        table.insert(far.clone()).await;
        let outcome = table.insert(near.clone()).await;
        assert!(outcome.drop_later.is_empty());
        assert!(
            table
                .offshoots()
                .await
                .iter()
                .any(|peer| Arc::ptr_eq(peer, &far))
        );

        assert!(table.remove(&near).await);
        assert!(table.boost_public().await);
        let backbones = table.backbones().await;
        assert_eq!(backbones.len(), 1);
        assert!(Arc::ptr_eq(&backbones[0], &far));
    }

    #[tokio::test]
    async fn outbound_public_is_not_demoted() {
        let table = PeerTable::new([0; 16], 1, 2);
        let far = remote_peer("far", 20, true, false);
        let near = remote_peer("near", 1, true, false);
        table.insert(far.clone()).await;
        let outcome = table.insert(near).await;
        assert!(
            outcome
                .drop_later
                .iter()
                .any(|peer| Arc::ptr_eq(peer, &far))
        );
        assert!(table.offshoots().await.is_empty());
    }

    #[tokio::test]
    async fn replaced_connection_cannot_remove_replacement() {
        let table = PeerTable::new([0; 16], 1, 2);
        let old = remote_peer("old", 1, false, true);
        let replacement = remote_peer("replacement", 1, false, true);
        table.insert(old.clone()).await;
        let outcome = table.insert(replacement.clone()).await;

        assert!(
            outcome
                .replaced
                .is_some_and(|peer| Arc::ptr_eq(&peer, &old))
        );
        assert!(old.closed.load(std::sync::atomic::Ordering::Acquire));
        assert!(!table.remove(&old).await);
        assert!(
            table
                .get_by_key_snapshot(&[1; 16])
                .is_some_and(|peer| Arc::ptr_eq(&peer, &replacement))
        );
    }

    #[tokio::test]
    async fn switch_peer_keeps_a_public_offshoot() {
        let table = PeerTable::new([0; 16], 1, 2);
        let backbone = remote_peer("backbone", 1, true, false);
        let offshoot = remote_peer("offshoot", 20, true, true);
        table.insert(backbone.clone()).await;
        table.insert(offshoot.clone()).await;

        assert!(
            table
                .try_switch_peer("offshoot")
                .is_some_and(|peer| Arc::ptr_eq(&peer, &offshoot))
        );
    }
}
