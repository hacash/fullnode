//! KV-backed `LogBackend`: per-height VM log entries stored as length-prefixed items.

use std::sync::Arc;

use base::{DiskDB, LogBackend, LogEntry, MemKV};
use sys::{Rerr, Ret};

use crate::store_mem::read_u64_be_prefix;

const KEY_TAG_LOG_LEN: u8 = 0x01;
const KEY_TAG_LOG_ITEM: u8 = 0x02;

fn log_len_key(height: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(9);
    v.push(KEY_TAG_LOG_LEN);
    v.extend_from_slice(&height.to_be_bytes());
    v
}

fn log_item_key(height: u64, index: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(17);
    v.push(KEY_TAG_LOG_ITEM);
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(&index.to_be_bytes());
    v
}

fn encode_log_entry(entry: &LogEntry) -> Ret<Vec<u8>> {
    let topic = entry.topic.as_bytes();
    if topic.len() > u16::MAX as usize {
        return sys::errf!("log topic too long: {}", topic.len());
    }
    let mut out = Vec::with_capacity(2 + topic.len() + entry.data.len());
    out.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    out.extend_from_slice(topic);
    out.extend_from_slice(&entry.data);
    Ok(out)
}

fn decode_log_entry(raw: &[u8]) -> Ret<LogEntry> {
    if raw.len() < 2 {
        return Err(sys::Error::fault("persisted log entry is truncated"));
    }
    let topic_len = u16::from_be_bytes([raw[0], raw[1]]) as usize;
    if raw.len() < 2 + topic_len {
        return Err(sys::Error::fault("persisted log topic is truncated"));
    }
    let topic = std::str::from_utf8(&raw[2..2 + topic_len])
        .map_err(|_| sys::Error::fault("persisted log topic is not utf-8"))?
        .to_owned();
    Ok(LogEntry {
        topic,
        data: raw[2 + topic_len..].to_vec(),
    })
}

pub(crate) struct KvLogBackend {
    disk: Arc<dyn DiskDB>,
}

impl KvLogBackend {
    pub(crate) fn new(disk: Arc<dyn DiskDB>) -> Self {
        Self { disk }
    }
}

impl LogBackend for KvLogBackend {
    fn append_block_logs(&self, height: u64, logs: &[LogEntry]) -> Rerr {
        self.remove_block_logs(height)?;
        if logs.is_empty() {
            return Ok(());
        }
        let mut batch = MemKV::new();
        let len = logs.len() as u64;
        batch.put(log_len_key(height), len.to_be_bytes().to_vec());
        for (idx, entry) in logs.iter().enumerate() {
            batch.put(log_item_key(height, idx as u64), encode_log_entry(entry)?);
        }
        self.disk.try_write(&batch)
    }

    fn load_block_logs(&self, height: u64) -> Ret<Vec<LogEntry>> {
        let Some(raw_len) = self.disk.read(&log_len_key(height))? else {
            return Ok(Vec::new());
        };
        let len = read_u64_be_prefix(&raw_len)
            .ok_or_else(|| sys::Error::fault("persisted log length is invalid"))?;
        let mut logs = Vec::with_capacity(len as usize);
        for idx in 0..len {
            let key = log_item_key(height, idx);
            let Some(raw) = self.disk.read(&key)? else {
                return Err(sys::Error::fault(format!(
                    "persisted log item {} missing",
                    idx
                )));
            };
            logs.push(decode_log_entry(&raw)?);
        }
        Ok(logs)
    }

    fn remove_block_logs(&self, height: u64) -> Rerr {
        let Some(raw_len) = self.disk.read(&log_len_key(height))? else {
            return Ok(());
        };
        let len = read_u64_be_prefix(&raw_len)
            .ok_or_else(|| sys::Error::fault("persisted log length is invalid"))?;
        let mut batch = MemKV::new();
        for idx in 0..len {
            batch.del(log_item_key(height, idx));
        }
        batch.del(log_len_key(height));
        self.disk.try_write(&batch)
    }
}
