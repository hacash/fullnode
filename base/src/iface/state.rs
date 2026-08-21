//! State KV and block-log contracts.

use sys::{Rerr, Ret};

/// Stable error string for canonical state backend read failures. The string
/// is owned by the state boundary; `sys` only stores and carries it.
pub const STATE_READ_FAILED_CODE: &'static str = "storage_read_failed";
/// Stable error string for trusted persisted state bytes that fail protocol
/// decode. Never used for network/API/user-input decode (those stay `Normal`).
pub const STATE_DECODE_FAILED_CODE: &'static str = "state_decode_failed";

// =============================================================
// StateRead / StateLayer KV
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
