//! State KV and block-log contracts.

use sys::{Rerr, Ret};

/// Stable error code for canonical state backend read failures. Attached at
/// the first stable `StateRead`/DB boundary; consumed by engine/boot/HTTP
/// edges for classification, never synthesized by guessing from messages.
pub const STATE_READ_FAILED_CODE: &str = "storage_read_failed";
/// Stable error code for trusted persisted state bytes that fail protocol
/// decode. Never used for network/API/user-input decode (those stay `Decode`).
pub const STATE_DECODE_FAILED_CODE: &str = "state_decode_failed";

// =============================================================
// StateRead / StateLayer  KV
// =============================================================

pub trait StateRead: Send + Sync {
    fn get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>>;
}

pub trait StateLayer: StateRead {
    fn set(&mut self, key: &[u8], val: Vec<u8>);
    fn del(&mut self, key: &[u8]);
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub topic: String,
    pub data: Vec<u8>,
}

// =============================================================
// LogBackend: best-effort execution log storage
// =============================================================

pub trait LogBackend: Send + Sync {
    fn append_block_logs(&self, height: u64, logs: &[LogEntry]) -> Rerr;
    fn load_block_logs(&self, height: u64) -> Ret<Vec<LogEntry>>;
    fn remove_block_logs(&self, height: u64) -> Rerr;
}
