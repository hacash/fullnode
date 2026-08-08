//! `node` —— P2P admission + mempool + listener.
//!
//! - Storage lives outside node; this crate is P2P and admission only.
//! - P2P wire: `magic` + `[u32 len][u8 ty][crc32c][body]` + VERSION/VERACK.
//! - Live peers use fullnodedev's DHT tables: public peers prefer **backbone**;
//!   inbound publics can be demoted to **offshoot** and promoted when a slot opens.
//! - `{data_dir}/stable.nodes` is persisted from the live backbone table.
//! - Dial returns after handshake+insert; the session read loop runs in a spawned task,
//!   matching fullnodedev's `insert_peer` lifecycle.
//! - Bulk sync uses a pipelined `GET_BLOCKS`/`BLOCKS` window over one
//!   `BlockStream` → background `run_sync`; tip blocks still use `discover_block`.
//! - Broadcast uses Knowledge (global 2000 + per-peer 500) via `broadcast_unaware`.
//! - Inbound tx/block go through a dedicated msg-handler queue (non-blocking read loop).
//! - Prefer `start_p2p_on(Handle, Waiter)` under a shared runtime.
//! - `submit_transaction` / admit flow: see base `Node` trait.
//! - Mint hooks (`check_tx`, `check_block_data`, `on_p2p_connect`, …) are called here;
//!   mint may leave default no-ops until filled in.

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
pub(crate) mod stable_nodes;
pub(crate) mod submit;
pub(crate) mod sync_pipeline;
pub(crate) mod sync_tracker;
pub(crate) mod topology;
pub(crate) mod transport;
pub(crate) mod txpool;
pub(crate) mod txpool_maintainer;

pub use p2p::msg::{
    MSG_BLOCK_DISCOVER, MSG_BLOCK_HASH, MSG_REQ_BLOCK_HASH, MSG_REQ_STATUS, MSG_STATUS,
    MSG_TX_SUBMIT, P2P_MSG_DATA_MAX_SIZE,
};
pub use p2pnode::{CustomMessageHandler, P2PNode};
pub use txpool::MemTxPool;
pub use txpool_maintainer::TxPoolMaintainer;
