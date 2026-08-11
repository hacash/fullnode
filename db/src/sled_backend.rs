use std::path::Path;

use base::{DiskDB, MemDB};
use sys::{Rerr, Ret};

use crate::config::{db_sled_small_machine_enabled, db_sync_enabled};

pub struct SledDisk {
    db: sled::Db,
}

impl SledDisk {
    pub fn open(dir: &Path) -> Ret<Self> {
        let mut cfg = sled::Config::new().path(dir);
        if db_sled_small_machine_enabled() {
            cfg = cfg
                .cache_capacity(32 * 1024 * 1024)
                .mode(sled::Mode::LowSpace)
                .flush_every_ms(Some(1000));
        }
        let db = cfg
            .open()
            .map_err(|e| sys::Error::fault(format!("open sled db: {}", e)))?;
        Ok(Self { db })
    }
}

impl DiskDB for SledDisk {
    fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
        // IO/corruption must NOT map to "missing" — that silently diverges state.
        match self.db.get(key) {
            Ok(v) => v.map(|v| v.to_vec()),
            Err(e) => panic!("sled read failed for key len={}: {}", key.len(), e),
        }
    }
    fn save(&self, key: &[u8], val: &[u8]) {
        self.db.insert(key, val).expect("sled save");
        if db_sync_enabled() {
            self.db.flush().expect("sled flush");
        }
    }
    fn remove(&self, key: &[u8]) {
        self.db.remove(key).expect("sled remove");
        if db_sync_enabled() {
            self.db.flush().expect("sled flush");
        }
    }
    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let mut wb = sled::Batch::default();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => wb.insert(key, value),
            None => wb.remove(key),
        });
        self.db
            .apply_batch(wb)
            .map_err(|e| sys::Error::fault(format!("sled write batch: {e}")))?;
        if db_sync_enabled() {
            self.db
                .flush()
                .map_err(|e| sys::Error::fault(format!("sled flush: {e}")))?;
        }
        Ok(())
    }
    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        for item in self.db.iter() {
            let (k, v) = item.map_err(|e| sys::Error::fault(e.to_string()))?;
            f(k.as_ref(), v.as_ref());
        }
        Ok(())
    }

    fn clear(&self) -> Rerr {
        self.db
            .clear()
            .map_err(|e| sys::Error::fault(format!("sled clear: {e}")))?;
        if db_sync_enabled() {
            self.db
                .flush()
                .map_err(|e| sys::Error::fault(format!("sled flush after clear: {e}")))?;
        }
        Ok(())
    }
}
