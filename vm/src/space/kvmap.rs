use std::collections::HashMap;
use std::collections::hash_map::Entry;

use field::Address;

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::Value;

/// Fallback key-byte cap when constructors omit `with_key_max`; aligned with default [`crate::rt::SpaceCap::kv_key_size`].
pub const DEFAULT_KV_KEY_MAX: usize = 128;

macro_rules! memories_kvmap_define {
    ($class:ident, $er1:expr, $er2:expr) => {
        #[allow(dead_code)]
        #[derive(Default, Clone, Debug)]
        pub struct $class {
            limit: usize,
            key_max: usize,
            datas: HashMap<Vec<u8>, Value>,
        }

        impl $class {
            pub fn new(limit: usize) -> Self {
                Self::with_key_max(limit, DEFAULT_KV_KEY_MAX)
            }

            pub fn with_key_max(limit: usize, key_max: usize) -> Self {
                Self {
                    limit,
                    key_max,
                    datas: HashMap::new(),
                }
            }

            pub fn reset(&mut self, lmt: usize) {
                self.reset_with_key_max(lmt, DEFAULT_KV_KEY_MAX);
            }

            pub fn reset_with_key_max(&mut self, lmt: usize, key_max: usize) {
                self.limit = lmt;
                self.key_max = key_max;
                self.clear();
            }

            pub fn clear(&mut self) {
                self.datas.clear();
            }

            fn key_raw(k: &Value) -> VmrtRes<Vec<u8>> {
                k.extract_key_bytes_with_error_code($er1)
            }

            fn key(&self, k: &Value) -> VmrtRes<Vec<u8>> {
                let key = Self::key_raw(k)?;
                if key.len() > self.key_max {
                    return itr_err_fmt!(
                        $er1,
                        "key too long, max {} bytes but got {}",
                        self.key_max,
                        key.len()
                    );
                }
                Ok(key)
            }

            pub fn key_len(&self, k: &Value) -> VmrtRes<usize> {
                Ok(self.key(k)?.len())
            }

            pub(crate) fn put(&mut self, k: Value, v: Value) -> VmrtErr {
                self.put_with_stats(k, v).map(|_| ())
            }

            pub(crate) fn put_with_stats(&mut self, k: Value, v: Value) -> VmrtRes<(usize, bool)> {
                v.check_scalar()?;
                let key = self.key(&k)?;
                let key_len = key.len();
                let full = self.datas.len() >= self.limit;
                match self.datas.entry(key) {
                    Entry::Occupied(mut hit) => {
                        hit.insert(v);
                        Ok((key_len, false))
                    }
                    Entry::Vacant(slot) => {
                        if full {
                            return itr_err_code!($er2); // out of limit
                        }
                        slot.insert(v);
                        Ok((key_len, true))
                    }
                }
            }

            pub fn get(&self, k: &Value) -> VmrtRes<Value> {
                Ok(match self.datas.get(&self.key(k)?) {
                    Some(v) => v.clone(),
                    None => Value::Nil,
                })
            }

            pub fn remove(&mut self, k: &Value) -> VmrtErr {
                self.datas.remove(&self.key(k)?);
                Ok(())
            }

            pub fn contains_key(&self, k: &Value) -> VmrtRes<bool> {
                Ok(self.datas.contains_key(&self.key(k)?))
            }

            pub fn len(&self) -> usize {
                self.datas.len()
            }

            pub fn keys_sorted(&self) -> Vec<Vec<u8>> {
                let mut keys = self.datas.keys().cloned().collect::<Vec<_>>();
                keys.sort_unstable();
                keys
            }
        }
    };
}

/*  */
memories_kvmap_define! { GKVMap, GlobalError, OutOfGlobal }
memories_kvmap_define! { MKVMap, MemoryError, OutOfMemory }

/*  */

#[derive(Default, Clone)]
pub struct CtcKVMap {
    limit: usize,
    key_max: usize,
    datas: HashMap<Address, MKVMap>,
}

impl CtcKVMap {
    #[inline(always)]
    fn check_addr(addr: &Address) -> VmrtErr {
        if !addr.is_supported() {
            return itr_err_fmt!(
                MemoryError,
                "memory use must be in effective address but version {}",
                addr.version()
            );
        }
        Ok(())
    }

    pub fn new(limit: usize) -> Self {
        Self::with_key_max(limit, DEFAULT_KV_KEY_MAX)
    }

    pub fn with_key_max(limit: usize, key_max: usize) -> Self {
        Self {
            limit,
            key_max,
            datas: HashMap::new(),
        }
    }

    pub fn reset(&mut self, lmt: usize) {
        self.reset_with_key_max(lmt, DEFAULT_KV_KEY_MAX);
    }

    pub fn reset_with_key_max(&mut self, lmt: usize, key_max: usize) {
        self.limit = lmt;
        self.key_max = key_max;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.datas.clear();
    }

    pub fn entry(&self, addr: &Address) -> VmrtRes<Option<&MKVMap>> {
        Self::check_addr(addr)?;
        Ok(self.datas.get(addr))
    }

    pub fn contains_addr(&self, addr: &Address) -> VmrtRes<bool> {
        Self::check_addr(addr)?;
        Ok(self.datas.contains_key(addr))
    }

    pub fn addr_len(&self) -> usize {
        self.datas.len()
    }

    pub fn entry_mut(&mut self, addr: &Address) -> VmrtRes<&mut MKVMap> {
        Self::check_addr(addr)?;
        Ok(self
            .datas
            .entry(addr.clone())
            .or_insert_with(|| MKVMap::with_key_max(self.limit, self.key_max)))
    }

    pub fn get(&self, addr: &Address, key: &Value) -> VmrtRes<Value> {
        Self::check_addr(addr)?;
        match self.datas.get(addr) {
            Some(mem) => mem.get(key),
            None => Ok(Value::Nil),
        }
    }

    pub fn remove(&mut self, addr: &Address, key: &Value) -> VmrtErr {
        Self::check_addr(addr)?;
        if let Some(mem) = self.datas.get_mut(addr) {
            mem.remove(key)?;
        }
        Ok(())
    }
}
