use std::collections::BTreeMap;
use std::sync::RwLock;

use base::{LogBackend, LogEntry};
use sys::{Rerr, Ret};

/// In-memory `LogBackend` preserving the block-oriented 1.0 log contract.
#[derive(Default)]
pub struct MemLogs {
    entries: RwLock<BTreeMap<u64, Vec<LogEntry>>>,
}

impl MemLogs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len_at(&self, height: u64) -> usize {
        self.entries
            .read()
            .unwrap()
            .get(&height)
            .map_or(0, Vec::len)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
    }
}

impl LogBackend for MemLogs {
    fn append_block_logs(&self, height: u64, logs: &[LogEntry]) -> Rerr {
        self.entries
            .write()
            .unwrap()
            .entry(height)
            .or_default()
            .extend_from_slice(logs);
        Ok(())
    }
    fn load_block_logs(&self, height: u64) -> Ret<Vec<LogEntry>> {
        Ok(self
            .entries
            .read()
            .unwrap()
            .get(&height)
            .cloned()
            .unwrap_or_default())
    }
    fn remove_block_logs(&self, height: u64) -> Rerr {
        self.entries.write().unwrap().remove(&height);
        Ok(())
    }
}
