//! Ping, idle eviction, and public offshoot promotion.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::P2PNode;
use crate::p2p::codec::create_transport_frame;
use crate::p2p::msg::MSG_PING;
use crate::p2p::peer::RemotePeer;

const PING_IDLE: Duration = Duration::from_secs(60 * 5);
const EVICT_IDLE: Duration = Duration::from_secs(60 * 20);

pub async fn ping_backbones(node: &P2PNode) {
    let now = Instant::now();
    for peer in node.peertable.backbones().await {
        let idle = peer
            .last_active
            .lock()
            .map(|g| now.saturating_duration_since(*g))
            .unwrap_or_default();
        if idle > PING_IDLE {
            let frame = create_transport_frame(MSG_PING, &[]);
            if let Ok(frame) = frame {
                let _ = peer.send_transport(frame);
            }
        }
    }
}

pub async fn check_active(node: &P2PNode) {
    let now = Instant::now();
    let mut stale = Vec::new();
    for peer in node
        .peertable
        .backbones()
        .await
        .into_iter()
        .chain(node.peertable.offshoots().await.into_iter())
    {
        let idle = peer
            .last_active
            .lock()
            .map(|g| now.saturating_duration_since(*g))
            .unwrap_or_default();
        if idle > EVICT_IDLE {
            stale.push(peer);
        }
    }
    for peer in stale {
        peer.disconnect();
    }
}

pub async fn boost_public(node: &Arc<P2PNode>) {
    if node.peertable.boost_public().await {
        node.persist_stable_backbones().await;
    }
}

impl P2PNode {
    pub(crate) async fn delay_close_peers(&self, peers: Vec<Arc<RemotePeer>>, delay_secs: u64) {
        if peers.is_empty() {
            return;
        }
        if delay_secs == 0 {
            for p in peers {
                p.disconnect();
            }
            return;
        }
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            for p in peers {
                p.disconnect();
            }
        });
    }
}
