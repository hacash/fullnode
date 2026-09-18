use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use base::{DiskDB, MemDB};
use sys::{Rerr, Ret};

/// Thread-safe unordered in-memory implementation of the durable KV boundary.
#[derive(Default)]
pub struct MemDiskDB {
    data: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
}

/// Deterministically ordered variant, useful when asserting persisted entries.
#[derive(Default)]
pub struct BTreeMemDiskDB {
    data: RwLock<BTreeMap<Vec<u8>, Vec<u8>>>,
}

/// Dual backend which checks every mutation and read against an ordered mirror.
#[derive(Default)]
pub struct VerifiedMemDiskDB {
    primary: MemDiskDB,
    mirror: BTreeMemDiskDB,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedMemDiskSnapshot {
    data: BTreeMap<Vec<u8>, Vec<u8>>,
}

macro_rules! impl_mem_disk {
    ($ty:ty, $map:ty) => {
        impl $ty {
            pub fn new() -> Self {
                Self::default()
            }
            pub fn len(&self) -> usize {
                self.data.read().unwrap().len()
            }
            pub fn is_empty(&self) -> bool {
                self.len() == 0
            }
            pub fn entries(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
                let mut entries: Vec<_> = self
                    .data
                    .read()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                entries
            }
        }

        impl DiskDB for $ty {
            fn read(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
                Ok(self.data.read().unwrap().get(key).cloned())
            }
            fn save(&self, key: &[u8], value: &[u8]) {
                self.data
                    .write()
                    .unwrap()
                    .insert(key.to_vec(), value.to_vec());
            }
            fn remove(&self, key: &[u8]) {
                self.data.write().unwrap().remove(key);
            }
            fn try_write(&self, mem: &dyn MemDB) -> Rerr {
                let mut data = self.data.write().unwrap();
                mem.for_each(&mut |key, value| match value {
                    Some(value) => {
                        data.insert(key.to_vec(), value.to_vec());
                    }
                    None => {
                        data.remove(key);
                    }
                });
                Ok(())
            }
            fn for_each(&self, each: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
                for (key, value) in self.data.read().unwrap().iter() {
                    each(key, value);
                }
                Ok(())
            }
        }
    };
}

impl_mem_disk!(MemDiskDB, HashMap<Vec<u8>, Vec<u8>>);
impl_mem_disk!(BTreeMemDiskDB, BTreeMap<Vec<u8>, Vec<u8>>);

impl VerifiedMemDiskDB {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        let len = self.primary.len();
        assert_eq!(len, self.mirror.len(), "verified disk length mismatch");
        len
    }
    pub fn snapshot(&self) -> VerifiedMemDiskSnapshot {
        self.assert_consistent();
        VerifiedMemDiskSnapshot {
            data: self.mirror.data.read().unwrap().clone(),
        }
    }
    pub fn restore(&self, snapshot: &VerifiedMemDiskSnapshot) {
        {
            let mut mirror = self.mirror.data.write().unwrap();
            *mirror = snapshot.data.clone();
        }
        {
            let mut primary = self.primary.data.write().unwrap();
            primary.clear();
            primary.extend(
                snapshot
                    .data
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        self.assert_consistent();
    }
    pub fn entries(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.assert_consistent();
        self.mirror.entries()
    }
    fn assert_consistent(&self) {
        let primary: BTreeMap<_, _> = self
            .primary
            .data
            .read()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let mirror = self.mirror.data.read().unwrap().clone();
        assert_eq!(primary, mirror, "verified disk mismatch");
    }
}

impl DiskDB for VerifiedMemDiskDB {
    fn read(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        let value = self.primary.read(key)?;
        assert_eq!(
            value,
            self.mirror.read(key)?,
            "verified disk read mismatch for {}",
            hex::encode(key)
        );
        Ok(value)
    }
    fn save(&self, key: &[u8], value: &[u8]) {
        self.primary.save(key, value);
        self.mirror.save(key, value);
        self.assert_consistent();
    }
    fn remove(&self, key: &[u8]) {
        self.primary.remove(key);
        self.mirror.remove(key);
        self.assert_consistent();
    }
    fn try_write(&self, mem: &dyn MemDB) -> Rerr {
        self.primary.try_write(mem)?;
        self.mirror.try_write(mem)?;
        self.assert_consistent();
        Ok(())
    }
    fn for_each(&self, each: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        self.assert_consistent();
        self.mirror.for_each(each)
    }
}
