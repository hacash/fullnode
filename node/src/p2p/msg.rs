//! P2P message type constants for both v2 and legacy (v1) protocols.
//!
//! ## Shared core application message types
//! The wire *values* of the core application messages (1..=8) are identical
//! across v1 and v2. In v1 they ride inside `MSG_CUSTOMER(0xFF)` as a u16
//! `app_ty`; in v2 they are top-level u16 transport types (no CUSTOMER
//! wrapper). This means callers can pass the same constant (e.g.
//! [`MSG_TX_SUBMIT`] = 7) to `peer.send_msg(...)` regardless of the peer's
//! negotiated version - `RemotePeer::send_msg` picks the framing.
//!
//! ## v2-only message types
//! Handshake (VERSION/VERACK), discovery (GETADDR/ADDR), and v2 control
//! use type values that do not collide with the shared core range (1..=8)
//! or with v1's transport-layer u8 codes when viewed as u16.
//!
//! ## v1 (legacy) protocol
//! - Magic: `P2P_MAGIC_V1` (compatible with mainnet `fullnodedev`).
//! - Frame: `[u32BE size=1+body.len][u8 ty][body]`.
//! - Two-tier dispatch: `MSG_CUSTOMER(0xFF)` wraps `[u16BE app_ty][body]`.
//! - Handshake: REPORT_PEER/ANSWER_PEER.
//! All v1-only constants live in `legacy.rs`.

// ===================================================================
// Shared (v1 + v2)
// ===================================================================

/// Mainnet v1 TCP handshake magic (big-endian on wire).
/// Kept here (not legacy.rs) because v2 accept-side detection reads it.
pub const P2P_MAGIC_V1: u32 = 3648109527;

/// v2 TCP handshake magic (big-endian on wire). Distinct from v1 so the
/// accept side identifies new vs old peer in a single 4-byte read.
pub const P2P_MAGIC_V2: u32 = 2480137569;

/// Max frame body bytes (shared ceiling; both v1 and v2 use this).
/// Large enough for 10k-block sync batches (~31.6 MiB).
pub const P2P_MSG_DATA_MAX_SIZE: usize = 1012 * 1024 * 32;

/// Node identity key size (both versions).
pub const PEER_KEY_SIZE: usize = 16;

// ===================================================================
// Shared core application message types (u16 in v1 CUSTOMER, u8 in v2)
// ===================================================================
//
// In v1 these are the `app_ty` carried inside `MSG_CUSTOMER(0xFF)`.
// In v2 these are top-level u16 transport types.
// Callers use these names with `peer.send_msg(ty, body)`; framing is
// chosen by `RemotePeer` based on negotiated `ProtocolVersion`.

pub const MSG_REQ_STATUS: u16 = 1;
pub const MSG_STATUS: u16 = 2;
pub const MSG_REQ_BLOCK_HASH: u16 = 3;
pub const MSG_BLOCK_HASH: u16 = 4;
pub const MSG_REQ_BLOCK: u16 = 5;
pub const MSG_BLOCK: u16 = 6;
/// New transaction push (v1 app_ty / v2 top-level).
/// Canonical definition lives in `base::MSG_TX_SUBMIT`; re-exported here
/// so node-internal `msg::MSG_TX_SUBMIT` callers are unaffected.
pub use base::MSG_TX_SUBMIT;
/// New block push / announce (v1 app_ty / v2 top-level).
pub const MSG_BLOCK_DISCOVER: u16 = 8;

// ===================================================================
// v2-only message types (u8)
// ===================================================================
//
// Values chosen to avoid colliding with shared core (1..=8) and with v1's
// transport u8 codes when widened to u16 (e.g. v1 MSG_PING=3 is already
// occupied by MSG_REQ_BLOCK_HASH=3 in the shared core - but v1 PING only
// exists as a transport-layer u8, never as an app_ty, so v2's PING needs a
// distinct value here).

pub mod v2 {
    /// `100` is permanently invalid. Values below it are system-reserved;
    /// values above it are custom and require explicit session negotiation.
    pub const MSG_RESERVED: u8 = 100;
    /// v2 handshake: VERSION (carries identity + genesis + services).
    pub const MSG_VERSION: u8 = 16;
    /// v2 handshake: VERACK (empty ack).
    pub const MSG_VERACK: u8 = 17;

    /// v2 liveness (distinct from v1 transport PING=3).
    pub const MSG_PING: u8 = 18;
    pub const MSG_PONG: u8 = 19;

    /// v2 peer discovery (replaces v1 MSG 202; IPv6-capable).
    pub const MSG_GETADDR: u8 = 20;
    pub const MSG_ADDR: u8 = 21;

    /// v2 graceful close (distinct from v1 transport CLOSE=254).
    pub const MSG_CLOSE: u8 = 22;

    /// v2 public-reachability probe (short connection).
    /// Request body empty; response body = 16-byte node_key.
    pub const MSG_CHECK_PUBLIC: u8 = 23;

    /// v2 pipelined block download request.
    /// Body: `[u64 request_id][u64 start][u32 max_blocks][u32 max_bytes]`.
    pub const MSG_GET_BLOCKS: u8 = 25;
    /// v2 pipelined block download response.
    /// Body: 44-byte header + concatenated block blobs.
    pub const MSG_BLOCKS: u8 = 26;
}

// ===================================================================
// Services bitfield (v2 only; advertised in VERSION)
// ===================================================================

pub mod services {
    /// Full node: serves blocks and history.
    pub const NODE_NETWORK: u64 = 1 << 0;
    /// Publicly reachable backbone (self-reported; verified by random probe).
    /// Replaces v1's MSG 201 reachability probe.
    pub const NODE_PUBLIC: u64 = 1 << 1;
    /// Willing to serve historical sync (heavy; high-load nodes may clear).
    pub const NODE_SYNC: u64 = 1 << 2;
    // Bits >= 1 << 3 are business-specific relay channels, declared by the
    // consensus layer via `TxPolicy::tx_pool_groups` and aggregated
    // into the advertised services mask by the node. The node itself does not
    // name or interpret them on the v2 path.

    /// Legacy v1 service bit for diamond-mint tx relay. v1 has no real service
    /// negotiation; this constant is only used to construct the synthesized
    /// v1 `PeerIdentity`. The whole v1 wire path is scheduled for removal once
    /// the network fully switches to v2, so it is kept verbatim rather than
    /// generalized.
    pub const NODE_DIAMOND: u64 = 1 << 3;
}

/// v2 protocol version advertised in VERSION message.
pub const PROTOCOL_VERSION_V2: u16 = 2;

/// v2 frame header size: 4 (length) + 1 (ty) + 4 (crc32c) = 9 bytes.
pub const V2_FRAME_HEADER_SIZE: usize = 9;

// ===================================================================
// v1 (legacy) re-exports
// ===================================================================
// Defined in `legacy.rs` to keep the v1 codepath self-contained. Re-exported
// here so v1 session code can import from `msg` for ergonomics.

pub use super::legacy::{
    MSG_ANSWER_PEER as V1_MSG_ANSWER_PEER, MSG_CLOSE as V1_MSG_CLOSE,
    MSG_CUSTOMER as V1_MSG_CUSTOMER, MSG_PING as V1_MSG_PING, MSG_PONG as V1_MSG_PONG,
    MSG_REMIND_ME_IS_PUBLIC as V1_MSG_REMIND_ME_IS_PUBLIC, MSG_REPORT_PEER as V1_MSG_REPORT_PEER,
    MSG_REQUEST_NEAREST_PUBLIC_NODES as V1_MSG_REQUEST_NEAREST_PUBLIC_NODES,
    MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK as V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK,
};
