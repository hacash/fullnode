//! Peer lifecycle: peertable insert/remove, addrbook on public eject, broadcast.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use base::Peer;
use sys::Rerr;

use crate::P2PNode;
use crate::addrbook::AddrSource;
use crate::knowledge::KnowKey;
use crate::p2p::msg::{MSG_REQ_STATUS, MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE};
use crate::p2p::peer::RemotePeer;

impl P2PNode {
    pub(crate) async fn add_peer(self: &Arc<Self>, peer: Arc<RemotePeer>) -> bool {
        let cap = self.config.backbone_peers + self.config.offshoot_peers;
        if self.peertable.peer_count().await >= cap && !self.peertable.has_peer(&peer.id).await {
            eprintln!("[P2P] peer capacity {} reached, refusing {}", cap, peer.id);
            return false;
        }
        let outcome = self.peertable.insert(peer.clone()).await;
        for ev in &outcome.evicted_public {
            self.addrbook
                .insert(Some(ev.key), ev.addr, AddrSource::Eject)
                .await;
        }
        if !outcome.evicted_public.is_empty() || outcome.backbone_changed {
            self.maybe_persist_addrbook().await;
        }
        self.delay_close_peers(outcome.drop_later, 15).await;

        // v2 peers already exchanged genesis + height in VERSION; skip the
        // REQ_STATUS/STATUS round-trip and start sync directly. v1 peers
        // must request STATUS (78-byte legacy format) to learn remote height.
        match peer.protocol_version() {
            crate::p2p::peer::ProtocolVersion::V2 => {
                let remote_height = peer
                    .remote_height
                    .load(std::sync::atomic::Ordering::Acquire);
                if let Err(e) = self.maybe_sync_from_remote_height(peer.clone(), remote_height) {
                    eprintln!("[P2P] v2 sync start to {} failed: {}", peer.id, e);
                }
            }
            crate::p2p::peer::ProtocolVersion::V1 => {
                if let Err(e) = peer.send_msg(MSG_REQ_STATUS, Vec::new()) {
                    eprintln!("[P2P] request status from {} failed: {}", peer.id, e);
                }
            }
        }
        let peer_ext: Arc<dyn Peer> = peer.clone();
        // Consensus hook (default no-op; mint implements selective tx push).
        if let Err(e) = self.engine.node_hooks().on_p2p_connect(
            peer_ext.clone(),
            self.engine.clone(),
            self.txpool.clone(),
        ) {
            eprintln!("[P2P] consensus on_p2p_connect failed: {}", e);
        }
        true
    }

    pub(crate) async fn remove_peer(self: &Arc<Self>, id: &str) {
        let resume_tip = {
            let mut g = self.sync_session.lock().unwrap();
            if g.as_ref().is_some_and(|session| session.peer_id == id) {
                g.take().map(|session| {
                    let tip = session.remote_tip;
                    session.cancel();
                    tip
                })
            } else {
                None
            }
        };
        let fast_sync_failed = resume_tip.is_some()
            && self.engine.config().fast_sync
            && !self.stopping.load(Ordering::Acquire);
        if fast_sync_failed {
            self.mark_sync_failure(id, "trusted FastSync source disconnected");
        } else {
            self.sync_tracker.clear_peer(id);
        }
        let Some(peer) = self.peertable.remove(id).await else {
            return;
        };
        peer.signal_close();
        self.boot_links
            .lock()
            .unwrap()
            .retain(|_, peer_id| peer_id != id);

        // Strict sync may resume through another backbone. FastSync deliberately
        // stops above because its single trusted source is part of the session.
        if fast_sync_failed {
            return;
        }

        // A cancelled BlockStream may still be unwinding its apply thread. Give
        // it a short opportunity to release the engine sync guard, then
        // resume from the current local tip through another backbone. The
        // persisted/local height is authoritative because v2 responses may
        // have been queued out of order when the old peer disappeared.
        if let Some(old_remote_tip) = resume_tip {
            if self.stopping.load(Ordering::Acquire) {
                return;
            }
            let next_peer = self.peertable.try_switch_peer(id);
            if let Some(next_peer) = next_peer {
                self.doing_sync.store(0, Ordering::Release);
                let remote_tip =
                    old_remote_tip.max(next_peer.remote_height.load(Ordering::Acquire));
                let node = self.clone();
                let lost_peer_id = id.to_owned();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if node.engine.latest_height() >= remote_tip {
                        return;
                    }
                    let peer: Arc<dyn Peer> = next_peer;
                    if let Err(e) = node.maybe_sync_from_remote_height(peer, remote_tip) {
                        eprintln!(
                            "[P2P] sync resume after peer {} loss failed: {}",
                            lost_peer_id, e
                        );
                    }
                });
            }
        }
    }

    pub(crate) fn note_boot_link(&self, addr: &str, peer_id: &str) {
        self.boot_links
            .lock()
            .unwrap()
            .insert(addr.to_string(), peer_id.to_string());
    }

    pub(crate) fn boot_already_linked(&self, addr: &str) -> bool {
        let links = self.boot_links.lock().unwrap();
        let Some(peer_id) = links.get(addr) else {
            return false;
        };
        self.peertable.try_has_peer(peer_id)
    }

    pub(crate) async fn maybe_persist_addrbook(&self) {
        if self.config.use_stable_nodes {
            self.addrbook.persist_async().await;
        }
    }

    /// Mark the source peer as knowing the hash and check global relay knowledge.
    /// The caller records global knowledge only after admission accepts the
    /// item; rejected and transient failures remain retryable.
    pub(crate) fn check_know(
        &self,
        hx: &field::Hash,
        peer: Option<&RemotePeer>,
    ) -> (bool, KnowKey) {
        let key = hx.into_array();
        if let Some(p) = peer {
            p.knows.add(key);
        }
        if self.knows.check(&key) {
            return (true, key);
        }
        (false, key)
    }

    pub(crate) fn check_block_know(
        &self,
        hx: &field::Hash,
        peer: Option<&RemotePeer>,
    ) -> (bool, KnowKey) {
        let key = hx.into_array();
        if let Some(p) = peer {
            p.knows.add(key);
        }
        (self.knows.check(&key), key)
    }

    pub(crate) fn remember_know(&self, key: KnowKey) {
        self.knows.add(key);
    }

    /// Broadcast only to peers that do not yet know `key`.
    /// Tx messages skip peers with `relay=false` (v2 VERSION flag).
    pub(crate) fn broadcast_unaware(
        &self,
        key: KnowKey,
        ty: u16,
        body: Vec<u8>,
        except_peer: Option<&str>,
    ) -> Rerr {
        if body.len() > P2P_MSG_DATA_MAX_SIZE.saturating_sub(3) {
            return sys::errf!("p2p message {} too large: {}", ty, body.len());
        }
        let is_tx = ty == MSG_TX_SUBMIT;
        let peers = self.peertable.values_snapshot();
        for peer in peers {
            if except_peer.map_or(false, |ex| peer.id == ex) {
                continue;
            }
            if is_tx && !peer.wants_relay() {
                continue;
            }
            if peer.knows.check(&key) {
                continue;
            }
            if let Err(e) = peer.send_msg(ty, body.clone()) {
                eprintln!("[P2P] send {} to {} failed: {}", ty, peer.id, e);
            } else {
                peer.knows.add(key);
            }
        }
        Ok(())
    }

    /// Broadcast a tx only to peers that opted into the given business relay
    /// channel. The channel bit is named by the consensus layer (see
    /// `TxPolicy::tx_pool_groups`); the node does not interpret it.
    pub(crate) fn broadcast_selective(
        &self,
        channel_bit: u64,
        key: KnowKey,
        body: Vec<u8>,
        except_peer: Option<&str>,
    ) -> Rerr {
        if body.len() > P2P_MSG_DATA_MAX_SIZE.saturating_sub(3) {
            return sys::errf!("p2p selective tx too large: {}", body.len());
        }
        let peers = self.peertable.values_snapshot();
        for peer in peers {
            if except_peer.map_or(false, |ex| peer.id == ex) {
                continue;
            }
            if !peer.wants_relay() || !peer.relays_channel(channel_bit) {
                continue;
            }
            if peer.knows.check(&key) {
                continue;
            }
            if let Err(e) = peer.send_msg(MSG_TX_SUBMIT, body.clone()) {
                eprintln!("[P2P] send selective tx to {} failed: {}", peer.id, e);
            } else {
                peer.knows.add(key);
            }
        }
        Ok(())
    }
}
