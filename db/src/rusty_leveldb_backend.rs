use std::path::Path;
use std::sync::Mutex;

use base::{DiskDB, MemDB};
use rusty_leveldb::LdbIterator;
use sys::{Rerr, Ret};

use crate::config::db_sync_enabled;

pub struct RustyLeveldbDisk {
    db: Mutex<rusty_leveldb::DB>,
}

impl RustyLeveldbDisk {
    pub fn open(dir: &Path) -> Ret<Self> {
        let mut opt = rusty_leveldb::Options::default();
        opt.create_if_missing = true;
        let db = rusty_leveldb::DB::open(dir, opt)
            .map_err(|e| sys::Error::fault(format!("open rusty-leveldb: {}", e)))?;
        Ok(Self { db: Mutex::new(db) })
    }
}

impl DiskDB for RustyLeveldbDisk {
    fn read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        // Poison-tolerant like the ring: a crashed writer must not wedge
        // reads of the still-valid store behind a poisoned lock.
        Ok(self.db.lock().unwrap_or_else(|e| e.into_inner()).get(key))
    }

    fn save(&self, key: &[u8], val: &[u8]) {
        let mut db = self.db.lock().unwrap();
        db.put(key, val).expect("rusty-leveldb put");
        if db_sync_enabled() {
            db.flush().expect("rusty-leveldb flush");
        }
    }

    fn remove(&self, key: &[u8]) {
        let mut db = self.db.lock().unwrap();
        db.delete(key).expect("rusty-leveldb delete");
        if db_sync_enabled() {
            db.flush().expect("rusty-leveldb flush");
        }
    }

    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let mut wb = rusty_leveldb::WriteBatch::default();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => wb.put(key, value),
            None => wb.delete(key),
        });
        let sync = db_sync_enabled();
        let mut db = self
            .db
            .lock()
            .map_err(|_| sys::Error::fault("rusty-leveldb lock poisoned"))?;
        db.write(wb, sync)
            .map_err(|e| sys::Error::fault(format!("rusty-leveldb write batch: {e}")))?;
        if sync {
            db.flush()
                .map_err(|e| sys::Error::fault(format!("rusty-leveldb flush: {e}")))?;
        }
        Ok(())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        // Collect first, then callback outside the DB lock to avoid re-entrant deadlock.
        let rows: Vec<(Vec<u8>, Vec<u8>)> = {
            let mut db = self.db.lock().unwrap();
            let mut iter = db
                .new_iter()
                .map_err(|e| sys::Error::fault(e.to_string()))?;
            let mut out = Vec::new();
            while let Some((k, v)) = iter.next() {
                out.push((k, v));
            }
            out
        };
        for (k, v) in &rows {
            f(k, v);
        }
        Ok(())
    }
}
