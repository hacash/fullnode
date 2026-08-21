//! `node` — P2P admission + mempool + listener; storage lives outside this crate.
//! Wire is `magic` + `[u32 len][u8 ty][crc32c][body]` + VERSION/VERACK; live peers use fullnodedev's DHT backbone/offshoot tables, bulk sync is a pipelined `GET_BLOCKS`/`BLOCKS` window over one `BlockStream`, and broadcast is deduped via Knowledge (global 2000 + per-peer 500).

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
