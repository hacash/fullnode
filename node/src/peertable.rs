//! Live connections only: backbone (public) + offshoot (private).
//!
//! Public peers never enter offshoot. Evicted publics are returned for AddrBook.
//! (Intentional vs mainnet: demoted publics are not kept as live offshoot links;
//! refill dials AddrBook / find_nodes nearest instead of boost_public.)

use std::collections::HashMap;
use std::net::SocketAddr;
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

/// Public peer evicted from backbone — caller must insert into AddrBook.
#[derive(Clone, Debug)]
pub struct EvictedPublic {
    pub key: PeerKey,
    pub addr: SocketAddr,
}

pub struct InsertOutcome {
    pub drop_later: Vec<Arc<RemotePeer>>,
    pub evicted_public: Vec<EvictedPublic>,
    pub backbone_changed: bool,
}

impl PeerTable {
    pub fn new(my_key: PeerKey, backbone_max: usize, offshoot_max: usize) -> Self {
        Self {
            inner: RwLock::new(Tables::default()),
            snapshot: StdRwLock::new(Vec::new()),
            my_key,
            backbone_max: backbone_max.max(1),
            offshoot_max: offshoot_max.max(1),
        }
    }

    pub async fn peer_count(&self) -> usize {
        self.inner.read().await.by_id.len()
    }

    pub async fn has_peer(&self, id: &str) -> bool {
        self.inner.read().await.by_id.contains_key(id)
    }

    #[allow(dead_code)] // reserved for future peer-key lookup
    pub async fn has_key(&self, key: &PeerKey) -> bool {
        let id = crate::p2p::handshake::peer_id_from_key(key);
        self.has_peer(&id).await
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

    /// Publics for MSG 202: backbone only (never offshoot).
    pub async fn publics(&self) -> Vec<Arc<RemotePeer>> {
        self.inner
            .read()
            .await
            .backbones
            .iter()
            .filter(|p| p.is_public() && !p.addr.ip().is_loopback())
            .cloned()
            .collect()
    }

    pub async fn remove(&self, id: &str) -> Option<Arc<RemotePeer>> {
        let mut t = self.inner.write().await;
        let peer = t.by_id.remove(id)?;
        t.backbones.retain(|p| p.id != id);
        t.offshoots.retain(|p| p.id != id);
        self.publish_snapshot(&t);
        Some(peer)
    }

    /// Insert: public → backbone only; private → offshoot only.
    pub async fn insert(&self, peer: Arc<RemotePeer>) -> InsertOutcome {
        let mut t = self.inner.write().await;
        let mut drop_later = Vec::new();
        let mut evicted_public = Vec::new();
        let mut backbone_changed = false;
        let id = peer.id.clone();

        if let Some(old) = t.by_id.remove(&id) {
            old.disconnect();
            let before = t.backbones.len();
            t.backbones.retain(|p| p.id != id);
            t.offshoots.retain(|p| p.id != id);
            if t.backbones.len() != before {
                backbone_changed = true;
            }
        }

        if peer.is_public() {
            t.by_id.insert(id.clone(), peer.clone());
            backbone_changed = true;
            if let Some(droped) = insert_ordered(
                &mut t.backbones,
                self.backbone_max,
                &self.my_key,
                peer,
                |p| p.node_key,
            ) {
                t.by_id.remove(&droped.id);
                evicted_public.push(EvictedPublic {
                    key: droped.node_key,
                    addr: droped.addr,
                });
                drop_later.push(droped);
            }
        } else if let Some(far) = {
            t.by_id.insert(id.clone(), peer.clone());
            insert_ordered(
                &mut t.offshoots,
                self.offshoot_max,
                &self.my_key,
                peer,
                |p| p.node_key,
            )
        } {
            t.by_id.remove(&far.id);
            drop_later.push(far);
        }

        self.publish_snapshot(&t);

        InsertOutcome {
            drop_later,
            evicted_public,
            backbone_changed,
        }
    }

    pub fn try_all_prints(&self) -> Vec<String> {
        let Ok(t) = self.inner.try_read() else {
            return vec![];
        };
        t.by_id
            .values()
            .map(|p| {
                if p.name.is_empty() {
                    p.id.clone()
                } else {
                    format!("{}<{}>", p.name, p.id)
                }
            })
            .collect()
    }

    pub fn try_has_peer(&self, id: &str) -> bool {
        self.inner
            .try_read()
            .map(|t| t.by_id.contains_key(id))
            .unwrap_or(false)
    }

    pub fn get_snapshot(&self, id: &str) -> Option<Arc<RemotePeer>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|peer| peer.id == id)
            .cloned()
    }

    pub fn values_snapshot(&self) -> Vec<Arc<RemotePeer>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn try_publics(&self) -> Vec<Arc<RemotePeer>> {
        self.inner
            .try_read()
            .map(|t| {
                t.backbones
                    .iter()
                    .filter(|p| p.is_public() && !p.addr.ip().is_loopback())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Prefer `prefer` if still a backbone; else first backbone (mainnet switch_peer).
    pub fn try_switch_peer(&self, prefer_id: &str) -> Option<Arc<RemotePeer>> {
        let t = self.inner.try_read().ok()?;
        if t.backbones.is_empty() {
            return None;
        }
        if let Some(p) = t.backbones.iter().find(|p| p.id == prefer_id) {
            return Some(p.clone());
        }
        t.backbones.first().cloned()
    }

    fn publish_snapshot(&self, tables: &Tables) {
        let peers = tables.by_id.values().cloned().collect();
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = peers;
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use tokio::sync::{Notify, mpsc};

    use super::PeerTable;
    use crate::knowledge::Knowledge;
    use crate::p2p::peer::{PeerWriteCmd, ProtocolVersion, RemotePeer};

    fn remote_peer(id: &str) -> Arc<RemotePeer> {
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<PeerWriteCmd>(1);
        let (tx_tx, _tx_rx) = mpsc::channel::<PeerWriteCmd>(1);
        Arc::new(RemotePeer {
            id: id.to_owned(),
            node_key: [1; 16],
            name: String::new(),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1),
            listen_port: 1,
            is_public: AtomicBool::new(false),
            is_inbound: AtomicBool::new(true),
            last_active: Mutex::new(Instant::now()),
            protocol_version: AtomicU8::new(ProtocolVersion::V2.as_u8()),
            remote_height: AtomicU64::new(0),
            service_mask: AtomicU64::new(u64::MAX),
            relay: AtomicBool::new(true),
            custom_types: Vec::new(),
            ctrl_tx,
            tx_tx,
            close_notify: Arc::new(Notify::new()),
            closed: Arc::new(AtomicBool::new(false)),
            knows: Knowledge::new(1),
            rate_count: AtomicU32::new(0),
            rate_window_start: Mutex::new(Instant::now()),
        })
    }

    #[test]
    fn broadcast_snapshot_survives_table_write_contention() {
        let table = PeerTable::new([0; 16], 1, 1);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            table.insert(remote_peer("peer-1")).await;
            let _write = table.inner.write().await;
            let peers = table.values_snapshot();
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].id, "peer-1");
            assert_eq!(table.get_snapshot("peer-1").unwrap().id, "peer-1");
        });
    }
}
