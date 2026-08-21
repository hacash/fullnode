//! P2P message type constants: application messages share the same u16 ids
//! throughout node APIs and are encoded as top-level frame types on the wire.

// ===================================================================
// Transport
// ===================================================================

/// TCP handshake magic (big-endian on wire).
pub const P2P_MAGIC: u32 = 2480137569;

/// Max frame body bytes.
/// Large enough for 10k-block sync batches (~31.6 MiB).
pub const P2P_MSG_DATA_MAX_SIZE: usize = 1012 * 1024 * 32;

/// Node identity key size.
pub const PEER_KEY_SIZE: usize = 16;

// ===================================================================
// Core application message types
// ===================================================================
// Sent with `peer.send_msg(ty, body)`; the peer writer performs framing.

pub const MSG_REQ_STATUS: u16 = 1;
pub const MSG_STATUS: u16 = 2;
pub const MSG_REQ_BLOCK_HASH: u16 = 3;
pub const MSG_BLOCK_HASH: u16 = 4;
/// New transaction push. Canonical definition in `base::MSG_TX_SUBMIT`;
/// re-exported here so node-internal `msg::MSG_TX_SUBMIT` callers are unaffected.
pub use base::MSG_TX_SUBMIT;
/// New block push / announce.
pub const MSG_BLOCK_DISCOVER: u16 = 8;

// ===================================================================
// System message types (u8)
// ===================================================================
// Values below 100 are system-reserved; above are custom and negotiated during VERSION.

/// `100` is permanently invalid. Values below it are system-reserved;
/// values above it are custom and require explicit session negotiation.
pub const MSG_RESERVED: u8 = 100;
/// Handshake VERSION (carries identity + genesis + services).
pub const MSG_VERSION: u8 = 16;
/// Handshake VERACK (empty ack).
pub const MSG_VERACK: u8 = 17;

/// Liveness.
pub const MSG_PING: u8 = 18;
pub const MSG_PONG: u8 = 19;

/// Peer discovery (IPv4 and IPv6 capable).
pub const MSG_GETADDR: u8 = 20;
pub const MSG_ADDR: u8 = 21;

/// Graceful close.
pub const MSG_CLOSE: u8 = 22;

/// Public-reachability probe (short connection).
/// Request body empty; response body = 16-byte node_key.
pub const MSG_CHECK_PUBLIC: u8 = 23;

/// Pipelined block download request.
/// Body: `[u64 request_id][u64 start][u32 max_blocks][u32 max_bytes]`.
pub const MSG_GET_BLOCKS: u8 = 25;
/// Pipelined block download response.
/// Body: 44-byte header + concatenated block blobs.
pub const MSG_BLOCKS: u8 = 26;
// ===================================================================
// Services bitfield (advertised in VERSION)
// ===================================================================

pub mod services {
    /// Full node: serves blocks and history.
    pub const NODE_NETWORK: u64 = 1 << 0;
    /// Publicly reachable backbone (self-reported; verified by random probe).
    pub const NODE_PUBLIC: u64 = 1 << 1;
    /// Willing to serve historical sync (heavy; high-load nodes may clear).
    pub const NODE_SYNC: u64 = 1 << 2;
    // Bits >= 1 << 3 are business-specific relay channels declared by the
    // consensus layer via `TxPolicy::tx_pool_groups`; the node never names/interpret them.
}

/// Current protocol version advertised in VERSION message.
pub const PROTOCOL_VERSION: u16 = 2;

/// Frame header size: 4 (length) + 1 (ty) + 4 (crc32c) = 9 bytes.
pub const FRAME_HEADER_SIZE: usize = 9;

// ===================================================================
