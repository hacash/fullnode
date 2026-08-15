use std::path::Path;

use base::{DiskDB, MemDB};
use sys::{Rerr, Ret};

use crate::config::db_sync_enabled;

pub struct RocksdbDisk {
    db: rocksdb::DB,
}

impl RocksdbDisk {
    pub fn open(dir: &Path) -> Ret<Self> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = rocksdb::DB::open(&opts, dir)
            .map_err(|e| sys::Error::fault(format!("open rocksdb: {}", e)))?;
        Ok(Self { db })
    }

    fn write_options() -> rocksdb::WriteOptions {
        let mut opts = rocksdb::WriteOptions::default();
        opts.set_sync(db_sync_enabled());
        opts
    }
}

impl DiskDB for RocksdbDisk {
    fn read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        // IO/corruption must NOT map to "missing" — that silently diverges state.
        self.db.get(key).map_err(|e| {
            sys::Error::fault(format!(
                "rocksdb read failed for key len={}: {}",
                key.len(),
                e
            ))
        })
    }

    fn save(&self, key: &[u8], val: &[u8]) {
        let opts = Self::write_options();
        self.db.put_opt(key, val, &opts).expect("rocksdb put");
    }

    fn remove(&self, key: &[u8]) {
        let opts = Self::write_options();
        self.db.delete_opt(key, &opts).expect("rocksdb delete");
    }

    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let mut wb = rocksdb::WriteBatch::default();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => wb.put(key, value),
            None => wb.delete(key),
        });
        self.db
            .write_opt(wb, &Self::write_options())
            .map_err(|e| sys::Error::fault(format!("rocksdb write batch: {e}")))
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            let (k, v) = item.map_err(|e| sys::Error::fault(e.to_string()))?;
            f(k.as_ref(), v.as_ref());
        }
        Ok(())
    }
}
