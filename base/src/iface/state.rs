//! State KV and block-log contracts.

use sys::{Rerr, Ret};

// =============================================================
// StateRead / StateLayer  KV
// =============================================================

pub trait StateRead: Send + Sync {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>>;
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
