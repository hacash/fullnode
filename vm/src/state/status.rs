use std::collections::BTreeMap;

use field::*;
use field::{BytesW1, Uint2};
use sys::Ret;

use crate::rt::ItrErrCode::*;
use crate::rt::{ItrErr, ItrErrCode, SpaceCap, VmrtErr, VmrtRes};
use crate::space::{VolatileKvLimits, validate_volatile_scalar_put};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusKV {
    key: BytesW1,
    value: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StatusSto {
    items: Vec<StatusKV>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StatusMap {
    items: BTreeMap<Vec<u8>, Value>,
}

impl StatusMap {
    pub fn from_storage(sto: &StatusSto) -> VmrtRes<Self> {
        let mut map = BTreeMap::new();
        for item in &sto.items {
            let value = &item.value;
            if matches!(value, Value::Nil) {
                return itr_err_fmt!(StorageError, "status value cannot be nil in storage");
            }
            value.check_scalar()?;
            let key = item.key.to_vec();
            if key.is_empty() {
                return itr_err_fmt!(StorageError, "status key cannot be empty in storage");
            }
            if map.insert(key, value.clone()).is_some() {
                return itr_err_fmt!(StorageError, "duplicate status key in storage");
            }
        }
        Ok(Self { items: map })
    }

    fn to_storage(&self) -> Ret<StatusSto> {
        let mut items = Vec::with_capacity(self.items.len());
        Uint2::from_usize(self.items.len())?;
        for (key, value) in &self.items {
            items.push(StatusKV {
                key: BytesW1::from(key.clone())?,
                value: value.clone(),
            });
        }
        Ok(StatusSto { items })
    }

    fn payload_size(&self) -> usize {
        let mut total = 0usize;
        for (k, v) in &self.items {
            total = total.saturating_add(k.len());
            total = total.saturating_add(v.val_size());
        }
        total
    }

    pub fn validate_key_lengths(&self, key_max: usize, ec: ItrErrCode) -> VmrtErr {
        for key in self.items.keys() {
            if key.len() > key_max {
                return itr_err_fmt!(
                    ec,
                    "status key too long, max {} bytes but got {}",
                    key_max,
                    key.len()
                );
            }
        }
        Ok(())
    }

    pub fn ensure_save_bounds(&self, cap: &SpaceCap) -> VmrtErr {
        self.validate_key_lengths(cap.kv_key_size, StorageKeyInvalid)?;
        for v in self.items.values() {
            validate_volatile_scalar_put(v, cap.value_size, StorageValSizeErr)?;
        }
        let payload = self.payload_size();
        if payload > cap.status_pure_size {
            return itr_err_fmt!(
                StorageValSizeErr,
                "status payload too large, max {} bytes but got {}",
                cap.status_pure_size,
                payload
            );
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, key: &[u8]) -> Value {
        self.items.get(key).cloned().unwrap_or(Value::Nil)
    }

    pub fn set_or_remove(&mut self, key: Vec<u8>, value: Value) -> VmrtErr {
        if matches!(value, Value::Nil) {
            self.items.remove(&key);
        } else {
            value.check_scalar()?;
            self.items.insert(key, value);
        }
        Ok(())
    }
}

impl StatusSto {
    pub fn from_status_map(map: &StatusMap) -> Ret<Self> {
        map.to_storage()
    }

    pub fn to_status_map(&self) -> VmrtRes<StatusMap> {
        StatusMap::from_storage(self)
    }
}

impl Encode for StatusSto {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size());
        self.encode_to(&mut out);
        out
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        Uint2::from(self.items.len() as u16).encode_to(out);
        for item in &self.items {
            item.key.encode_to(out);
            item.value.encode_to(out);
        }
    }

    fn size(&self) -> usize {
        Uint2::SIZE
            + self
                .items
                .iter()
                .map(|i| field::Encode::size(&i.key) + i.value.size())
                .sum::<usize>()
    }
}

impl Decode for StatusSto {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut cur = buf;
        let mut used = 0usize;
        let (count, n) = Uint2::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let mut items = Vec::with_capacity(count.uint() as usize);
        for _ in 0..count.uint() {
            let (key, n) = BytesW1::decode(cur)?;
            cur = &cur[n..];
            used += n;
            let (value, n) = Value::decode(cur)?;
            cur = &cur[n..];
            used += n;
            items.push(StatusKV { key, value });
        }
        Ok((Self { items }, used))
    }
}

#[allow(dead_code)]
fn _keep_imports(_: VolatileKvLimits, _: ItrErr) {}
