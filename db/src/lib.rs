//! `db` - disk KV backends + triple-dir [`StoreInst`] (`block` / `state` / `vmlog`).
//!
//! Ops model: wipe state (+ `vmlog`) and rebuild from local `block/` without
//! re-syncing the network. `Store::disk()` is the **state** database.

mod block_store;
mod config;
mod log_backend;
mod mem;
mod store_mem;
pub use store_mem::StoreInst;

#[cfg(feature = "db-leveldb-sys")]
mod leveldb_sys_backend;
#[cfg(feature = "db-leveldb-sys")]
mod leveldb_sys_raw;
#[cfg(feature = "db-rocksdb")]
mod rocksdb_backend;
#[cfg(feature = "db-rusty-leveldb")]
mod rusty_leveldb_backend;
#[cfg(feature = "db-sled")]
mod sled_backend;

use std::path::Path;

use base::{DiskDB, MemDB};
use sys::{Rerr, Ret};

/// Backend selected by the same feature priority used by [`DiskKV::open`].
pub const fn backend_name() -> &'static str {
    if cfg!(feature = "db-rocksdb") {
        "rocksdb"
    } else if cfg!(feature = "db-leveldb-sys") {
        "leveldb-sys"
    } else if cfg!(feature = "db-rusty-leveldb") {
        "rusty-leveldb"
    } else if cfg!(feature = "db-sled") {
        "sled"
    } else {
        "none"
    }
}

/// Disk KV facade. Feature priority: rocksdb > leveldb-sys > rusty-leveldb > sled.
pub struct DiskKV {
    inner: Box<dyn DiskDB>,
}

impl DiskKV {
    pub fn open(dir: &Path) -> Ret<Self> {
        #[cfg(feature = "db-rocksdb")]
        {
            return Ok(Self {
                inner: Box::new(rocksdb_backend::RocksdbDisk::open(dir)?),
            });
        }
        #[cfg(all(feature = "db-leveldb-sys", not(feature = "db-rocksdb")))]
        {
            return Ok(Self {
                inner: Box::new(leveldb_sys_backend::LeveldbSysDisk::open(dir)?),
            });
        }
        #[cfg(all(
            feature = "db-rusty-leveldb",
            not(feature = "db-rocksdb"),
            not(feature = "db-leveldb-sys")
        ))]
        {
            return Ok(Self {
                inner: Box::new(rusty_leveldb_backend::RustyLeveldbDisk::open(dir)?),
            });
        }
        #[cfg(all(
            feature = "db-sled",
            not(feature = "db-rocksdb"),
            not(feature = "db-leveldb-sys"),
            not(feature = "db-rusty-leveldb")
        ))]
        {
            return Ok(Self {
                inner: Box::new(sled_backend::SledDisk::open(dir)?),
            });
        }
        #[cfg(not(any(
            feature = "db-sled",
            feature = "db-rusty-leveldb",
            feature = "db-leveldb-sys",
            feature = "db-rocksdb"
        )))]
        {
            let _ = dir;
            sys::errf!("no db backend feature enabled")
        }
    }
}

impl DiskDB for DiskKV {
    fn read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        self.inner.read(key)
    }
    fn save(&self, key: &[u8], val: &[u8]) {
        self.inner.save(key, val);
    }
    fn remove(&self, key: &[u8]) {
        self.inner.remove(key);
    }
    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        self.inner.try_write(memkv)
    }
    fn try_read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        self.inner.try_read(key)
    }
    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        self.inner.for_each(f)
    }
    fn clear(&self) -> Rerr {
        self.inner.clear()
    }
}
