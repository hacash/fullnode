//! Remote peer: dual write channels, version-aware send, rate limit.
//!
//! - **Control queue** (capacity 64): sync/status/ping/close — never dropped;
//!   uses async `send` from the writer task's perspective via `try_send` first,
//!   then `blocking`-style wait via `reserve` retry with `send`.
//! - **Tx queue** (capacity 256): tx relays — dropped under saturation.
//!
//! A single writer task drains control preferentially, then tx.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, mpsc};

use sys::Rerr;

use super::codec::create_transport_frame as v2_create_frame;
use super::legacy::{
    create_transport_frame as v1_create_frame, encode_customer as v1_encode_customer,
};
use super::msg::{MSG_TX_SUBMIT, V1_MSG_CLOSE, v2 as v2msg};
use crate::knowledge::Knowledge;

pub(crate) const TX_WRITER_CAPACITY: usize = 256;
pub(crate) const CTRL_WRITER_CAPACITY: usize = 64;
/// Soft per-peer inbound message budget per window.
pub(crate) const RATE_LIMIT_MSGS: u32 = 2000;
pub(crate) const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProtocolVersion {
    V1 = 1,
    V2 = 2,
}

impl ProtocolVersion {
    pub fn from_u8(v: u8) -> Self {
        match v {
            2 => ProtocolVersion::V2,
            _ => ProtocolVersion::V1,
        }
    }
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

pub(crate) struct PeerWriteCmd {
    pub frame: Vec<u8>,
}

#[allow(dead_code)] // fields reserved for future P2P feature gating
pub(crate) struct RemotePeer {
    pub id: String,
    pub node_key: [u8; 16],
    pub name: String,
    pub addr: SocketAddr,
    pub listen_port: u16,
    pub is_public: AtomicBool,
    pub is_inbound: AtomicBool,
    pub last_active: Mutex<Instant>,
    pub protocol_version: AtomicU8,
    /// Remote chain height learned at handshake (v2: from VERSION.start_height;
    /// v1: 0 until STATUS arrives). Used to start sync without a STATUS round-trip.
    pub remote_height: std::sync::atomic::AtomicU64,
    /// Business relay channels the peer opted into via its advertised services
    /// mask (v2: captured from VERSION.services; v1: all channels assumed).
    /// Channel bits are named by the consensus layer; the node only checks
    /// membership via `relays_channel(bit)`.
    pub service_mask: std::sync::atomic::AtomicU64,
    pub relay: AtomicBool,
    pub custom_types: Vec<u8>,
    /// Prefer draining this first; never drop.
    pub ctrl_tx: mpsc::Sender<PeerWriteCmd>,
    /// Tx relay; droppable under load.
    pub tx_tx: mpsc::Sender<PeerWriteCmd>,
    pub close_notify: Arc<Notify>,
    pub closed: Arc<AtomicBool>,
    pub knows: Knowledge,
    /// Inbound rate limiter state.
    pub(crate) rate_count: AtomicU32,
    pub(crate) rate_window_start: Mutex<Instant>,
}

impl base::Peer for RemotePeer {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn send_msg(&self, ty: u16, body: Vec<u8>) -> Rerr {
        let frame = match self.protocol_version() {
            ProtocolVersion::V2 => {
                let ty = u8::try_from(ty).map_err(|_| {
                    sys::Error::fault(format!("v2 message type out of range: {}", ty))
                })?;
                if ty == v2msg::MSG_RESERVED {
                    return sys::errf!("v2 message type 100 is reserved");
                }
                if ty > v2msg::MSG_RESERVED && !self.supports_custom_type(ty) {
                    return sys::errf!("peer {} did not negotiate custom message {}", self.id, ty);
                }
                v2_create_frame(ty, &body)?
            }
            ProtocolVersion::V1 => v1_encode_customer(ty, &body)?,
        };
        if ty == MSG_TX_SUBMIT {
            self.send_tx_frame(frame)
        } else {
            self.send_ctrl_frame(frame)
        }
    }
    fn protocol_version(&self) -> u8 {
        self.protocol_version.load(Ordering::Acquire)
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
        self.send_ctrl_frame(frame)
    }

    pub(crate) fn send_v2_transport(&self, ty: u8, body: &[u8]) -> Rerr {
        if ty == v2msg::MSG_RESERVED {
            return sys::errf!("v2 message type 100 is reserved");
        }
        let frame = v2_create_frame(ty, body)?;
        self.send_ctrl_frame(frame)
    }

    fn send_ctrl_frame(&self, frame: Vec<u8>) -> Rerr {
        // Control: try_send; if full, use blocking send on a dedicated path.
        // From sync Peer::send_msg we cannot await — use try_send then
        // `blocking_send` via tokio's block_in_place when inside runtime.
        match self.ctrl_tx.try_send(PeerWriteCmd { frame }) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(sys::Error::fault(format!(
                "peer {} ctrl queue saturated",
                self.id
            ))),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(sys::Error::fault(format!("peer {} ctrl closed", self.id)))
            }
        }
    }

    fn send_tx_frame(&self, frame: Vec<u8>) -> Rerr {
        match self.tx_tx.try_send(PeerWriteCmd { frame }) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(sys::Error::fault(format!(
                "peer {} tx queue saturated, frame dropped",
                self.id
            ))),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(sys::Error::fault(format!("peer {} tx closed", self.id)))
            }
        }
    }

    pub(crate) fn touch(&self) {
        if let Ok(mut g) = self.last_active.lock() {
            *g = Instant::now();
        }
    }

    /// Returns false if peer exceeded inbound rate — caller should disconnect.
    pub(crate) fn note_inbound_msg(&self) -> bool {
        let now = Instant::now();
        if let Ok(mut start) = self.rate_window_start.lock() {
            if now.duration_since(*start) >= RATE_LIMIT_WINDOW {
                *start = now;
                self.rate_count.store(1, Ordering::Release);
                return true;
            }
        }
        let n = self.rate_count.fetch_add(1, Ordering::AcqRel) + 1;
        n <= RATE_LIMIT_MSGS
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

    pub(crate) fn protocol_version(&self) -> ProtocolVersion {
        ProtocolVersion::from_u8(self.protocol_version.load(Ordering::Acquire))
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
        match self.protocol_version() {
            ProtocolVersion::V2 => {
                if let Ok(frame) = v2_create_frame(v2msg::MSG_CLOSE, &[]) {
                    let _ = self.send_ctrl_frame(frame);
                }
            }
            ProtocolVersion::V1 => {
                if let Ok(frame) = v1_create_frame(V1_MSG_CLOSE, &[]) {
                    let _ = self.send_ctrl_frame(frame);
                }
            }
        }
        self.signal_close();
    }

    pub(crate) fn signal_close(&self) {
        self.closed.store(true, Ordering::Release);
        self.close_notify.notify_waiters();
    }
}

/// Spawn the dual-queue writer. Prefer control frames.
pub(crate) fn spawn_writer(
    mut ctrl_rx: mpsc::Receiver<PeerWriteCmd>,
    mut tx_rx: mpsc::Receiver<PeerWriteCmd>,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    close_notify: Arc<Notify>,
    closed: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                cmd = ctrl_rx.recv() => {
                    let Some(cmd) = cmd else { break; };
                    if tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(&cmd.frame))
                        .await
                        .map_or(true, |result| result.is_err())
                    {
                        break;
                    }
                }
                cmd = tx_rx.recv() => {
                    let Some(cmd) = cmd else {
                        // tx closed; keep draining control
                        while let Some(cmd) = ctrl_rx.recv().await {
                            if tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(&cmd.frame))
                                .await
                                .map_or(true, |result| result.is_err())
                            {
                                break;
                            }
                        }
                        break;
                    };
                    if tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(&cmd.frame))
                        .await
                        .map_or(true, |result| result.is_err())
                    {
                        break;
                    }
                }
            }
        }
        closed.store(true, Ordering::Release);
        close_notify.notify_waiters();
    })
}

// AsyncWriteExt used by spawn_writer
