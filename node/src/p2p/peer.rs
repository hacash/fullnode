//! Remote peer: single writer queue.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, mpsc};

use sys::Rerr;

use super::codec::create_transport_frame;
use super::msg::{MSG_CLOSE, MSG_RESERVED};
use crate::knowledge::Knowledge;

pub(crate) const PEER_WRITER_CAPACITY: usize = 128;
static PEER_AUTO_ID_INCREASE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn next_peer_id() -> String {
    (PEER_AUTO_ID_INCREASE.fetch_add(1, Ordering::Relaxed) + 1).to_string()
}

pub(crate) enum PeerWriteCmd {
    Send(Vec<u8>),
    Close(Vec<u8>),
}

#[allow(dead_code)] // fields reserved for future P2P feature gating
pub(crate) struct RemotePeer {
    /// Unique TCP connection instance id, equivalent to fullnodedev's auto id.
    pub id: String,
    /// Stable node identity used for DHT placement and same-node replacement.
    pub node_key: [u8; 16],
    pub name: String,
    pub addr: SocketAddr,
    pub listen_port: u16,
    pub is_public: AtomicBool,
    pub is_inbound: AtomicBool,
    pub last_active: Mutex<Instant>,
    /// Business relay channels the peer opted into via its advertised services
    /// mask captured from VERSION.services.
    /// Channel bits are named by the consensus layer; the node only checks
    /// membership via `relays_channel(bit)`.
    pub service_mask: std::sync::atomic::AtomicU64,
    pub relay: AtomicBool,
    pub custom_types: Vec<u8>,
    pub writer_tx: mpsc::Sender<PeerWriteCmd>,
    pub close_notify: Arc<Notify>,
    pub closed: Arc<AtomicBool>,
    pub knows: Knowledge,
}

impl base::Peer for RemotePeer {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn send_msg(&self, ty: u16, body: Vec<u8>) -> Rerr {
        let ty = u8::try_from(ty)
            .map_err(|_| sys::Error::fault(format!("message type out of range: {}", ty)))?;
        if ty == MSG_RESERVED {
            return sys::errf!("message type 100 is reserved");
        }
        if ty > MSG_RESERVED && !self.supports_custom_type(ty) {
            return sys::errf!("peer {} did not negotiate custom message {}", self.id, ty);
        }
        let frame = create_transport_frame(ty, &body)?;
        self.send_frame(frame)
    }
    fn disconnect(&self) {
        RemotePeer::disconnect(self);
    }
}

#[allow(dead_code)] // methods reserved for future P2P feature gating
impl RemotePeer {
    pub(crate) fn nick(&self) -> String {
        if self.is_public() {
            format!("{}<{}>", self.name, self.addr)
        } else {
            self.name.clone()
        }
    }

    pub(crate) fn send_transport(&self, frame: Vec<u8>) -> Rerr {
        self.send_frame(frame)
    }

    pub(crate) fn send_message(&self, ty: u8, body: &[u8]) -> Rerr {
        if ty == MSG_RESERVED {
            return sys::errf!("message type 100 is reserved");
        }
        let frame = create_transport_frame(ty, body)?;
        self.send_frame(frame)
    }

    fn send_frame(&self, frame: Vec<u8>) -> Rerr {
        if self.closed.load(Ordering::Acquire) {
            return sys::errf!("peer may be closed");
        }
        match self.writer_tx.try_send(PeerWriteCmd::Send(frame)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => sys::errf!("peer writer queue full"),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.signal_close();
                sys::errf!("peer may be closed")
            }
        }
    }

    pub(crate) fn touch(&self) {
        if let Ok(mut g) = self.last_active.lock() {
            *g = Instant::now();
        }
    }

    pub(crate) fn is_public(&self) -> bool {
        self.is_public.load(Ordering::Acquire)
    }

    pub(crate) fn set_public(&self, v: bool) {
        self.is_public.store(v, Ordering::Release);
    }

    pub(crate) fn is_inbound(&self) -> bool {
        self.is_inbound.load(Ordering::Acquire)
    }

    pub(crate) fn wants_relay(&self) -> bool {
        self.relay.load(Ordering::Acquire)
    }

    pub(crate) fn supports_custom_type(&self, ty: u8) -> bool {
        self.custom_types.binary_search(&ty).is_ok()
    }

    /// Whether this peer has opted into the given business relay channel
    /// (declared via `TxPolicy::tx_pool_groups`). Channel bits are
    /// consensus-defined service bits; the node only inspects membership.
    pub(crate) fn relays_channel(&self, channel_bit: u64) -> bool {
        self.service_mask.load(Ordering::Acquire) & channel_bit != 0
    }

    pub(crate) fn disconnect(&self) {
        let close_frame = create_transport_frame(MSG_CLOSE, &[]);
        self.closed.store(true, Ordering::Release);
        if let Ok(frame) = close_frame {
            let _ = self.writer_tx.try_send(PeerWriteCmd::Close(frame));
        }
        self.close_notify.notify_waiters();
    }

    pub(crate) fn signal_close(&self) {
        self.closed.store(true, Ordering::Release);
        self.close_notify.notify_waiters();
    }
}

/// Spawn fullnodedev's single peer writer queue.
pub(crate) fn spawn_writer(
    mut writer_rx: mpsc::Receiver<PeerWriteCmd>,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    close_notify: Arc<Notify>,
    closed: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(cmd) = writer_rx.recv().await {
            let (frame, close_after) = match cmd {
                PeerWriteCmd::Send(frame) => (frame, false),
                PeerWriteCmd::Close(frame) => (frame, true),
            };
            if writer.write_all(&frame).await.is_err() {
                break;
            }
            if close_after {
                break;
            }
        }
        closed.store(true, Ordering::Release);
        close_notify.notify_waiters();
    })
}

// AsyncWriteExt used by spawn_writer

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::next_peer_id;

    #[test]
    fn connection_ids_are_unique() {
        let ids = (0..256).map(|_| next_peer_id()).collect::<HashSet<_>>();
        assert_eq!(ids.len(), 256);
    }
}
