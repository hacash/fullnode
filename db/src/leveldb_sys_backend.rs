use std::path::Path;

use base::{DiskDB, MemDB};
use sys::{Rerr, Ret};

use crate::leveldb_sys_raw::{LevelDB, Writebatch};

pub struct LeveldbSysDisk {
    db: LevelDB,
}

impl LeveldbSysDisk {
    pub fn open(dir: &Path) -> Ret<Self> {
        Ok(Self {
            db: LevelDB::open(dir),
        })
    }
}

impl DiskDB for LeveldbSysDisk {
    fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.db.get(key)
    }
    fn save(&self, key: &[u8], val: &[u8]) {
        self.db.put(key, val);
    }
    fn remove(&self, key: &[u8]) {
        self.db.rm(key);
    }
    fn write(&self, memkv: &dyn MemDB) {
        let mut wb = Writebatch::new();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => wb.put(key, value),
            None => wb.delete(key),
        });
        self.db.write(&wb);
    }
    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let mut wb = Writebatch::new();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => wb.put(key, value),
            None => wb.delete(key),
        });
        self.db.try_write(&wb)
    }
    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        self.db.for_each(f)
    }
}
