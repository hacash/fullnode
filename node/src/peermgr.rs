//! Peer lifecycle matching fullnodedev's insert/remove behavior.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use base::Peer;
use sys::Rerr;

use crate::P2PNode;
use crate::knowledge::KnowKey;
use crate::p2p::msg::{MSG_REQ_STATUS, MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE};
use crate::p2p::peer::RemotePeer;

impl P2PNode {
    pub(crate) async fn add_peer(self: &Arc<Self>, peer: Arc<RemotePeer>) -> bool {
        let outcome = self.peertable.insert(peer.clone()).await;
        if let Some(old) = outcome.replaced {
            let cancelled = {
                let mut slot = self.sync_session.lock().unwrap();
                if slot
                    .as_ref()
                    .is_some_and(|session| session.peer_id == old.id)
                {
                    slot.take().map(|session| session.cancel()).is_some()
                } else {
                    false
                }
            };
            self.sync_tracker.clear_peer(&old.id);
            if cancelled {
                self.doing_sync.store(0, Ordering::Release);
            }
        }
        if outcome.backbone_changed {
            self.persist_stable_backbones().await;
        }
        self.delay_close_peers(outcome.drop_later, 15).await;
        true
    }

    pub(crate) fn on_peer_connect(self: &Arc<Self>, peer: Arc<RemotePeer>) {
        let _ = peer.send_msg(MSG_REQ_STATUS, Vec::new());
        let peer_ext: Arc<dyn Peer> = peer.clone();
        if let Err(e) = self.engine.node_hooks().on_p2p_connect(
            peer_ext.clone(),
            self.engine.clone(),
            self.txpool.clone(),
        ) {
            eprintln!("[P2P] consensus on_p2p_connect failed: {}", e);
        }
        for handler in self.custom_message_handlers() {
            if let Err(e) = handler.on_connect(peer_ext.clone()) {
                eprintln!("[P2P] custom on_connect failed: {}", e);
            }
        }
    }

    pub(crate) fn on_peer_disconnect(&self, peer: Arc<RemotePeer>) {
        let peer_ext: Arc<dyn Peer> = peer;
        for handler in self.custom_message_handlers() {
            handler.on_disconnect(peer_ext.clone());
        }
    }

    pub(crate) async fn remove_peer(self: &Arc<Self>, peer: &Arc<RemotePeer>) {
        let id = &peer.id;
        let cancelled = {
            let mut g = self.sync_session.lock().unwrap();
            if g.as_ref().is_some_and(|session| session.peer_id == *id) {
                g.take().map(|session| session.cancel()).is_some()
            } else {
                false
            }
        };
        self.sync_tracker.clear_peer(id);
        if cancelled {
            self.doing_sync.store(0, Ordering::Release);
        }
        if !self
            .peertable
            .get_snapshot(id)
            .is_some_and(|current| Arc::ptr_eq(&current, peer))
        {
            return;
        }
        let was_backbone = self
            .peertable
            .backbones()
            .await
            .iter()
            .any(|current| Arc::ptr_eq(current, peer));
        if !self.peertable.remove(peer).await {
            return;
        }
        peer.signal_close();
        if was_backbone {
            self.persist_stable_backbones().await;
        }
    }

    pub(crate) async fn persist_stable_backbones(&self) {
        if self.config.use_stable_nodes {
            let addrs = self
                .peertable
                .backbones()
                .await
                .into_iter()
                .filter(|peer| peer.is_public() && !peer.addr.ip().is_loopback())
                .take(self.config.backbone_peers)
                .map(|peer| peer.addr)
                .collect::<Vec<_>>();
            let data_dir = self.config.data_dir.clone();
            tokio::task::spawn_blocking(move || {
                crate::stable_nodes::write_stable_file(&data_dir, &addrs);
            });
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

    /// Broadcast only to peers that do not yet know `key`, matching dev.
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
        let peers = self.peertable.values_snapshot();
        for peer in peers {
            if except_peer.map_or(false, |ex| peer.id == ex) {
                continue;
            }
            if peer.knows.check(&key) {
                continue;
            }
            peer.knows.add(key);
            if let Err(e) = peer.send_msg(ty, body.clone()) {
                eprintln!("[P2P] send {} to {} failed: {}", ty, peer.id, e);
            }
        }
        Ok(())
    }

    /// Keep the selective API contract used by the transaction policy, while
    /// matching dev's actual network behavior: relay to every unaware peer.
    pub(crate) fn broadcast_selective(
        &self,
        _channel_bit: u64,
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
            if peer.knows.check(&key) {
                continue;
            }
            peer.knows.add(key);
            if let Err(e) = peer.send_msg(MSG_TX_SUBMIT, body.clone()) {
                eprintln!("[P2P] send selective tx to {} failed: {}", peer.id, e);
            }
        }
        Ok(())
    }
}
