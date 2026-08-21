use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use base::{DiskDB, MemDB};
use sys::Rerr;

/// In-memory KV (`HashMap` + `RwLock`) for tests / no-disk mode.
/// Lock poison panics so a crashed writer cannot silently drop later writes.
#[derive(Default)]
pub struct MemDiskDB {
    inner: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
}

impl MemDiskDB {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl DiskDB for MemDiskDB {
    fn read(&self, key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        // Reads are recoverable and poison-tolerant (a crashed writer must not
        // wedge later reads); writes below still panic on poison.
        Ok(self
            .inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned())
    }

    fn save(&self, key: &[u8], val: &[u8]) {
        self.inner
            .write()
            .unwrap()
            .insert(key.to_vec(), val.to_vec());
    }

    fn remove(&self, key: &[u8]) {
        self.inner.write().unwrap().remove(key);
    }

    fn try_write(&self, memkv: &dyn MemDB) -> Rerr {
        let mut inner = self.inner.write().unwrap();
        memkv.for_each(&mut |key, value| match value {
            Some(value) => {
                inner.insert(key.to_vec(), value.to_vec());
            }
            None => {
                inner.remove(key);
            }
        });
        Ok(())
    }

    fn for_each(&self, f: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        for (k, v) in self.inner.read().unwrap().iter() {
            f(k, v);
        }
        Ok(())
    }

    fn clear(&self) -> Rerr {
        self.inner.write().unwrap().clear();
        Ok(())
    }
}
