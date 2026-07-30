//! `node` —— P2P admission + mempool + listener.
//!
//! - Storage lives outside node; this crate is P2P and admission only.
//! - Dual wire: **v1** mainnet (`magic` + `[u32][u8 ty][body]` + CUSTOMER) and **v2**
//!   (`magic` + `[u32 len][u8 ty][crc32c][body]` + VERSION/VERACK). Dial prefers v2
//!   with v1 fallback; accept detects magic first.
//! - Live peers: **backbone** (public, small) + **offshoot** (private only, large).
//! - Publics never enter offshoot; ejected publics go to **AddrBook** (≤200, DHT order),
//!   persisted as `{data_dir}/stable.nodes`. Differs from older mainnet which kept
//!   demoted publics as live offshoot connections.
//! - Dial returns after handshake+insert; the session read loop runs in a spawned task
//!   (same as mainnet `insert_peer`), so find_nodes / AddrBook refill can dial many peers.
//! - Bulk sync (v1 serial + v2 window) shares one `BlockStream` → background `run_sync`;
//!   tip blocks still use `discover_block`. v1 wire (`REQ_BLOCK`/`MSG_BLOCK`) unchanged.
//! - Broadcast uses Knowledge (global 2000 + per-peer 500) via `broadcast_unaware`;
//!   control frames use a dedicated writer queue that is never dropped under load.
//! - Inbound tx/block go through a dedicated msg-handler queue (non-blocking read loop).
//! - Prefer `start_p2p_on(Handle, Waiter)` under a shared runtime.
//! - `submit_transaction` / admit flow: see base `Node` trait.
//! - Mint hooks (`check_tx`, `check_block_data`, `on_p2p_connect`, …) are called here;
//!   mint may leave default no-ops until filled in.

pub(crate) mod addrbook;
pub(crate) mod discovery;
pub(crate) mod keepalive;
pub(crate) mod knowledge;
pub(crate) mod msgqueue;
pub(crate) mod p2p;
pub(crate) mod p2pnode;
pub(crate) mod peermgr;
pub(crate) mod peertable;
pub(crate) mod protocol;
pub(crate) mod publiccheck;
pub(crate) mod submit;
pub(crate) mod sync_pipeline;
pub(crate) mod sync_tracker;
pub(crate) mod topology;
pub(crate) mod transport;
pub(crate) mod txpool;
pub(crate) mod txpool_maintainer;

pub use p2p::msg::{
    MSG_BLOCK, MSG_BLOCK_DISCOVER, MSG_BLOCK_HASH, MSG_REQ_BLOCK, MSG_REQ_BLOCK_HASH,
    MSG_REQ_STATUS, MSG_STATUS, MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE,
};
pub use p2pnode::{CustomMessageHandler, P2PNode};
pub use txpool::MemTxPool;
pub use txpool_maintainer::TxPoolMaintainer;
