use std::collections::{BTreeMap, HashMap, HashSet};

use field::Address;

use crate::rt::{ItrErr, ItrErrCode, SpaceCap, VmrtErr, VmrtRes};
use crate::space::{MKVMap, VolatileKvLimits, validate_volatile_kv_put};
use crate::value::{ContractAddress, Value, value_content_eq};

#[derive(Clone, Copy, Debug)]
pub struct IntentRuntimeLimits {
    pub create_limit: usize,
    pub keys_per_intent: usize,
    pub val_size_limit: usize,
    pub key_max_bytes: usize,
}

impl IntentRuntimeLimits {
    pub fn from_space_cap(cap: &SpaceCap) -> Self {
        Self {
            create_limit: cap.intent_new,
            keys_per_intent: cap.intent_key,
            val_size_limit: cap.value_size,
            key_max_bytes: cap.kv_key_size,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IntentEntry {
    pub kind: Vec<u8>,
    pub data: MKVMap,
}

#[derive(Clone, Debug, Default)]
struct IntentBucketMap {
    datas: HashMap<Address, HashMap<usize, IntentEntry>>,
}

impl IntentBucketMap {
    fn clear(&mut self) {
        self.datas.clear();
    }

    fn entry_mut(&mut self, owner: &ContractAddress) -> &mut HashMap<usize, IntentEntry> {
        self.datas.entry(owner.to_addr()).or_default()
    }

    fn get(&self, owner: &ContractAddress) -> Option<&HashMap<usize, IntentEntry>> {
        self.datas.get(&owner.to_addr())
    }

    fn get_mut(&mut self, owner: &ContractAddress) -> Option<&mut HashMap<usize, IntentEntry>> {
        self.datas.get_mut(&owner.to_addr())
    }

    fn remove(&mut self, owner: &ContractAddress, id: usize) -> Option<IntentEntry> {
        self.datas
            .get_mut(&owner.to_addr())
            .and_then(|m| m.remove(&id))
    }
}

#[derive(Clone, Debug)]
pub struct IntentRuntime {
    next_id: usize,
    id_generation: usize,
    total_created: usize,
    create_limit: usize,
    key_limit: usize,
    key_max_bytes: usize,
    val_size_limit: usize,
    intent_key_limit: usize,
    owners: HashMap<usize, ContractAddress>,
    buckets: IntentBucketMap,
}

impl Default for IntentRuntime {
    fn default() -> Self {
        Self::new(IntentRuntimeLimits {
            create_limit: 0,
            keys_per_intent: 0,
            val_size_limit: 0,
            key_max_bytes: 128,
        })
    }
}

impl IntentRuntime {
    pub fn new(limits: IntentRuntimeLimits) -> Self {
        Self {
            next_id: 0,
            id_generation: 0,
            total_created: 0,
            create_limit: limits.create_limit,
            key_limit: limits.keys_per_intent,
            key_max_bytes: limits.key_max_bytes,
            val_size_limit: limits.val_size_limit,
            intent_key_limit: limits.keys_per_intent,
            owners: HashMap::new(),
            buckets: IntentBucketMap::default(),
        }
    }

    pub fn clear(&mut self) {
        self.next_id = 0;
        self.total_created = 0;
        self.owners.clear();
        self.buckets.clear();
    }

    pub fn reset(&mut self, limits: IntentRuntimeLimits) {
        self.create_limit = limits.create_limit;
        self.key_limit = limits.keys_per_intent;
        self.key_max_bytes = limits.key_max_bytes;
        self.val_size_limit = limits.val_size_limit;
        self.intent_key_limit = limits.keys_per_intent;
        self.clear();
    }

    fn next_intent_id(&self) -> VmrtRes<usize> {
        let mut id = self.id_generation.checked_add(1).unwrap_or(1);
        let mut steps = 0usize;
        while self.owners.contains_key(&id) {
            steps += 1;
            if steps > self.create_limit {
                return itr_err_fmt!(
                    ItrErrCode::IntentError,
                    "intent id allocation failed (invariant broken)"
                );
            }
            id = id.checked_add(1).unwrap_or(1);
        }
        Ok(id)
    }

    pub fn create(&mut self, owner: ContractAddress, kind: Vec<u8>) -> VmrtRes<usize> {
        if kind.is_empty() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent kind cannot be empty");
        }
        self.check_size_limit(kind.len(), "kind")?;
        if self.total_created >= self.create_limit {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent creation limit {} exceeded",
                self.create_limit
            );
        }
        let next_gen = self.next_intent_id()?;
        self.id_generation = next_gen;
        self.next_id = next_gen;
        self.total_created += 1;
        self.owners.insert(next_gen, owner);
        self.buckets.entry_mut(&owner).insert(
            next_gen,
            IntentEntry {
                kind,
                data: MKVMap::with_key_max(self.key_limit, self.key_max_bytes),
            },
        );
        Ok(next_gen)
    }

    fn intent_not_found(id: usize) -> ItrErr {
        ItrErr::new(ItrErrCode::IntentError, &format!("intent {} not found", id))
    }

    fn key_not_found() -> ItrErr {
        ItrErr::new(ItrErrCode::IntentError, "intent key not found")
    }

    fn check_size_limit(&self, size: usize, label: &str) -> VmrtErr {
        if size > self.val_size_limit {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent {} size {} exceeds limit {}",
                label,
                size,
                self.val_size_limit
            );
        }
        Ok(())
    }

    pub fn ensure_owner(&self, owner: &ContractAddress, id: usize) -> VmrtErr {
        let real = self.owner_of(id)?;
        if real != *owner {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent {} is not owned by contract {}",
                id,
                owner.to_readable()
            );
        }
        Ok(())
    }

    fn require_ref(&self, owner: &ContractAddress, id: usize) -> VmrtRes<&IntentEntry> {
        self.ensure_owner(owner, id)?;
        self.buckets
            .get(owner)
            .and_then(|bucket| bucket.get(&id))
            .ok_or_else(|| Self::intent_not_found(id))
    }

    fn require_mut(&mut self, owner: &ContractAddress, id: usize) -> VmrtRes<&mut IntentEntry> {
        self.ensure_owner(owner, id)?;
        self.buckets
            .get_mut(owner)
            .and_then(|bucket| bucket.get_mut(&id))
            .ok_or_else(|| Self::intent_not_found(id))
    }

    fn validate_non_nil_scalar(val: &Value) -> VmrtErr {
        val.check_non_nil_scalar(ItrErrCode::IntentError)
            .map_err(|ItrErr(_, msg)| {
                if val.is_nil() {
                    ItrErr::new(ItrErrCode::IntentError, "intent value cannot be nil")
                } else {
                    ItrErr::new(ItrErrCode::IntentError, &msg)
                }
            })
    }

    fn extract_intent_key_bytes(&self, key: &Value) -> VmrtRes<Vec<u8>> {
        let key_bytes = key.extract_key_bytes_with_error_code(ItrErrCode::IntentError)?;
        if key_bytes.len() > self.key_max_bytes {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent key too long, max {} bytes but got {}",
                self.key_max_bytes,
                key_bytes.len()
            );
        }
        Ok(key_bytes)
    }

    fn validate_intent_key(&self, key: &Value) -> VmrtErr {
        self.extract_intent_key_bytes(key)?;
        Ok(())
    }

    fn validate_key_value_for_put(&self, key: &Value, val: &Value) -> VmrtErr {
        let limits = VolatileKvLimits {
            key_max_bytes: self.key_max_bytes,
            value_max_bytes: self.val_size_limit,
        };
        validate_volatile_kv_put(key, val, &limits, false, ItrErrCode::IntentError)
    }

    fn intent_value_eq(lhs: &Value, rhs: &Value) -> VmrtRes<bool> {
        value_content_eq(lhs, rhs).map_err(|ItrErr(_, msg)| {
            let tip = if msg.is_empty() {
                "intent value comparison invalid".to_string()
            } else {
                msg
            };
            ItrErr::new(ItrErrCode::IntentError, &tip)
        })
    }

    fn uint_add_checked_with_msg(left: &Value, right: &Value, msg: &str) -> VmrtRes<Value> {
        if !left.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent value must be uint");
        }
        if !right.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent delta must be uint");
        }
        let mut lx = left.clone();
        let mut ry = right.clone();
        Value::cast_same_uint_width2(&mut lx, &mut ry)
            .map_err(|ItrErr(_, msg)| ItrErr::new(ItrErrCode::IntentError, &msg))?;
        match (lx, ry) {
            (Value::U8(a), Value::U8(b)) => a.checked_add(b).map(Value::U8),
            (Value::U16(a), Value::U16(b)) => a.checked_add(b).map(Value::U16),
            (Value::U32(a), Value::U32(b)) => a.checked_add(b).map(Value::U32),
            (Value::U64(a), Value::U64(b)) => a.checked_add(b).map(Value::U64),
            (Value::U128(a), Value::U128(b)) => a.checked_add(b).map(Value::U128),
            _ => None,
        }
        .ok_or_else(|| ItrErr::new(ItrErrCode::IntentError, msg))
    }

    fn uint_sub_checked(left: &Value, right: &Value) -> VmrtRes<Value> {
        if !left.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent value must be uint");
        }
        if !right.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent delta must be uint");
        }
        let mut lx = left.clone();
        let mut ry = right.clone();
        Value::cast_same_uint_width2(&mut lx, &mut ry)
            .map_err(|ItrErr(_, msg)| ItrErr::new(ItrErrCode::IntentError, &msg))?;
        match (lx, ry) {
            (Value::U8(a), Value::U8(b)) => a.checked_sub(b).map(Value::U8),
            (Value::U16(a), Value::U16(b)) => a.checked_sub(b).map(Value::U16),
            (Value::U32(a), Value::U32(b)) => a.checked_sub(b).map(Value::U32),
            (Value::U64(a), Value::U64(b)) => a.checked_sub(b).map(Value::U64),
            (Value::U128(a), Value::U128(b)) => a.checked_sub(b).map(Value::U128),
            _ => None,
        }
        .ok_or_else(|| ItrErr::new(ItrErrCode::IntentError, "intent sub underflow"))
    }

    fn ensure_insert_capacity(
        entry: &IntentEntry,
        key: &Value,
        intent_key_limit: usize,
    ) -> VmrtRes<bool> {
        let exists = entry.data.contains_key(key)?;
        if !exists && entry.data.len() >= intent_key_limit {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent key count {} exceeds limit {}",
                entry.data.len(),
                intent_key_limit
            );
        }
        Ok(exists)
    }

    fn prepare_put_mode(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: &Value,
        val: &Value,
    ) -> VmrtRes<bool> {
        self.validate_key_value_for_put(key, val)?;
        let intent_key_limit = self.intent_key_limit;
        let entry = self.require_mut(owner, id)?;
        Self::ensure_insert_capacity(entry, key, intent_key_limit)
    }

    pub fn put(&mut self, owner: &ContractAddress, id: usize, key: Value, val: Value) -> VmrtErr {
        self.prepare_put_mode(owner, id, &key, &val)?;
        self.require_mut(owner, id)?.data.put(key, val)
    }

    pub fn exists(&self, id: usize) -> bool {
        self.owners.contains_key(&id)
    }

    pub fn owner_of(&self, id: usize) -> VmrtRes<ContractAddress> {
        self.owners
            .get(&id)
            .copied()
            .ok_or_else(|| Self::intent_not_found(id))
    }

    pub fn is_owner(&self, owner: &ContractAddress, id: usize) -> VmrtRes<bool> {
        Ok(self.owner_of(id)? == *owner)
    }

    pub fn kind(&self, owner: &ContractAddress, id: usize) -> VmrtRes<Value> {
        Ok(Value::Bytes(self.require_ref(owner, id)?.kind.clone()))
    }

    pub fn kind_is(&self, owner: &ContractAddress, id: usize, kind: &[u8]) -> VmrtRes<bool> {
        Ok(self.require_ref(owner, id)?.kind == kind)
    }

    pub fn get(&self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtRes<Value> {
        self.validate_intent_key(key)?;
        self.require_ref(owner, id)?.data.get(key)
    }

    pub fn take(&mut self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtRes<Value> {
        let val = self.require(owner, id, key)?;
        self.require_mut(owner, id)?.data.remove(key)?;
        Ok(val)
    }

    pub fn del(&mut self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtErr {
        self.require(owner, id, key)?;
        self.require_mut(owner, id)?.data.remove(key)
    }

    pub fn has(&self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtRes<bool> {
        self.validate_intent_key(key)?;
        self.require_ref(owner, id)?.data.contains_key(key)
    }

    pub fn clear_data(&mut self, owner: &ContractAddress, id: usize) -> VmrtErr {
        self.require_mut(owner, id)?.data.clear();
        Ok(())
    }

    pub fn len(&self, owner: &ContractAddress, id: usize) -> VmrtRes<usize> {
        Ok(self.require_ref(owner, id)?.data.len())
    }

    pub fn keys_sorted(&self, owner: &ContractAddress, id: usize) -> VmrtRes<Vec<Vec<u8>>> {
        Ok(self.require_ref(owner, id)?.data.keys_sorted())
    }

    pub fn get_or(
        &self,
        owner: &ContractAddress,
        id: usize,
        key: &Value,
        def: Value,
    ) -> VmrtRes<Value> {
        self.validate_intent_key(key)?;
        let entry = self.require_ref(owner, id)?;
        if entry.data.contains_key(key)? {
            entry.data.get(key)
        } else {
            Ok(def)
        }
    }

    pub fn require(&self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtRes<Value> {
        let val = self.get(owner, id, key)?;
        if val.is_nil() {
            return Err(Self::key_not_found());
        }
        Ok(val)
    }

    pub fn require_eq(
        &self,
        owner: &ContractAddress,
        id: usize,
        key: &Value,
        expected: &Value,
    ) -> VmrtRes<Value> {
        Self::validate_non_nil_scalar(expected)?;
        let val = self.require(owner, id, key)?;
        if !Self::intent_value_eq(&val, expected)? {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent value mismatch");
        }
        Ok(val)
    }

    pub fn require_absent(&self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtErr {
        self.validate_intent_key(key)?;
        if self.require_ref(owner, id)?.data.contains_key(key)? {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent key already exists");
        }
        Ok(())
    }

    pub fn replace(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        val: Value,
    ) -> VmrtRes<Value> {
        let old = self.require(owner, id, &key)?;
        self.put(owner, id, key, val)?;
        Ok(old)
    }

    pub fn destroy(&mut self, owner: &ContractAddress, id: usize) -> VmrtErr {
        self.ensure_owner(owner, id)?;
        self.owners.remove(&id);
        self.buckets.remove(owner, id);
        Ok(())
    }

    fn add_core(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        delta: Value,
        missing_base: Option<Value>,
        delta_err: &str,
        target_err: &str,
        overflow_err: &str,
    ) -> VmrtRes<Value> {
        if !delta.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "{}", delta_err);
        }
        self.validate_intent_key(&key)?;
        let base = {
            let entry = self.require_ref(owner, id)?;
            if entry.data.contains_key(&key)? {
                let existing = entry.data.get(&key)?;
                if !existing.is_uint() {
                    return itr_err_fmt!(ItrErrCode::IntentError, "{}", target_err);
                }
                existing
            } else {
                missing_base.ok_or_else(Self::key_not_found)?
            }
        };
        let val = Self::uint_add_checked_with_msg(&base, &delta, overflow_err)?;
        self.put(owner, id, key, val.clone())?;
        Ok(val)
    }

    pub fn append(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        val: &Value,
    ) -> VmrtRes<usize> {
        let new_bytes = match val {
            Value::Bytes(buf) => buf.clone(),
            _ => return itr_err_fmt!(ItrErrCode::IntentError, "intent append value must be Bytes"),
        };
        let mut buf = match self.require(owner, id, &key)? {
            Value::Bytes(buf) => buf,
            _ => {
                return itr_err_fmt!(
                    ItrErrCode::IntentError,
                    "intent append target must be Bytes"
                );
            }
        };
        buf.extend_from_slice(&new_bytes);
        self.check_size_limit(buf.len(), "appended value")?;
        self.put(owner, id, key, Value::Bytes(buf.clone()))?;
        Ok(buf.len())
    }

    pub fn add(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        delta: Value,
    ) -> VmrtRes<Value> {
        self.add_core(
            owner,
            id,
            key,
            delta,
            None,
            "intent add delta must be uint",
            "intent add target must be uint",
            "intent add overflow",
        )
    }

    pub fn sub(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        delta: Value,
    ) -> VmrtRes<Value> {
        if !delta.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent sub delta must be uint");
        }
        self.validate_intent_key(&key)?;
        let entry = self.require_mut(owner, id)?;
        if !entry.data.contains_key(&key)? {
            return Err(Self::key_not_found());
        }
        let existing = entry.data.get(&key)?;
        if !existing.is_uint() {
            return itr_err_fmt!(ItrErrCode::IntentError, "intent sub target must be uint");
        }
        let val = Self::uint_sub_checked(&existing, &delta)?;
        entry.data.put(key, val.clone())?;
        Ok(val)
    }

    pub fn inc(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        delta: Value,
    ) -> VmrtRes<Value> {
        self.add_core(
            owner,
            id,
            key,
            delta,
            Some(Value::U64(0)),
            "intent inc delta must be uint",
            "intent inc target must be uint",
            "intent inc overflow",
        )
    }

    pub fn put_if_absent(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        val: Value,
    ) -> VmrtRes<bool> {
        if self.prepare_put_mode(owner, id, &key, &val)? {
            return Ok(false);
        }
        self.require_mut(owner, id)?.data.put(key, val)?;
        Ok(true)
    }

    fn conditional_op_core(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: &Value,
        expected: &Value,
    ) -> VmrtRes<Option<Value>> {
        Self::validate_non_nil_scalar(expected)?;
        let existing = self.require(owner, id, key)?;
        if !Self::intent_value_eq(&existing, expected)? {
            return Ok(None);
        }
        Ok(Some(existing))
    }

    pub fn replace_if(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        old_val: Value,
        new_val: Value,
    ) -> VmrtRes<bool> {
        Self::validate_non_nil_scalar(&new_val)?;
        let new_val_len = new_val.extract_bytes_len_with_error_code(ItrErrCode::IntentError)?;
        self.check_size_limit(new_val_len, "value")?;

        match self.conditional_op_core(owner, id, &key, &old_val)? {
            None => Ok(false),
            Some(_) => {
                self.require_mut(owner, id)?.data.put(key, new_val)?;
                Ok(true)
            }
        }
    }

    pub fn del_if(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        old_val: Value,
    ) -> VmrtRes<bool> {
        match self.conditional_op_core(owner, id, &key, &old_val)? {
            None => Ok(false),
            Some(_) => {
                self.require_mut(owner, id)?.data.remove(&key)?;
                Ok(true)
            }
        }
    }

    pub fn take_if(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        old_val: Value,
    ) -> VmrtRes<(bool, Value)> {
        match self.conditional_op_core(owner, id, &key, &old_val)? {
            None => {
                let existing = self.require_ref(owner, id)?.data.get(&key)?;
                Ok((false, existing))
            }
            Some(val) => {
                self.require_mut(owner, id)?.data.remove(&key)?;
                Ok((true, val))
            }
        }
    }

    pub fn destroy_if_empty(&mut self, owner: &ContractAddress, id: usize) -> VmrtRes<bool> {
        if self.len(owner, id)? > 0 {
            return Ok(false);
        }
        self.destroy(owner, id)?;
        Ok(true)
    }

    pub fn keys_page(
        &self,
        owner: &ContractAddress,
        id: usize,
        cursor: usize,
        limit: usize,
    ) -> VmrtRes<(Option<usize>, Vec<Vec<u8>>)> {
        if limit == 0 {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent keys page limit must be positive"
            );
        }
        let keys = self.keys_sorted(owner, id)?;
        if keys.is_empty() {
            if cursor == 0 {
                return Ok((None, vec![]));
            }
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent keys page cursor out of range"
            );
        }
        if cursor > keys.len() {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent keys page cursor out of range"
            );
        }
        if cursor == keys.len() {
            return Ok((None, vec![]));
        }
        let end = cursor.saturating_add(limit).min(keys.len());
        let next = if end < keys.len() { Some(end) } else { None };
        Ok((next, keys[cursor..end].to_vec()))
    }

    pub fn move_key(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        src_key: Value,
        dst_key: Value,
    ) -> VmrtErr {
        self.validate_intent_key(&src_key)?;
        self.validate_intent_key(&dst_key)?;
        let val = {
            let entry = self.require_ref(owner, id)?;
            if !entry.data.contains_key(&src_key)? {
                return itr_err_fmt!(ItrErrCode::IntentError, "intent source key not found");
            }
            if entry.data.contains_key(&dst_key)? {
                return itr_err_fmt!(
                    ItrErrCode::IntentError,
                    "intent destination key already exists"
                );
            }
            entry.data.get(&src_key)?
        };
        self.validate_key_value_for_put(&dst_key, &val)?;
        let entry = self.require_mut(owner, id)?;
        entry.data.remove(&src_key)?;
        entry.data.put(dst_key, val)?;
        Ok(())
    }

    pub fn keys_after(
        &self,
        owner: &ContractAddress,
        id: usize,
        start: Option<&Value>,
        limit: usize,
    ) -> VmrtRes<(Option<Vec<u8>>, Vec<Vec<u8>>)> {
        if limit == 0 {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent keys from limit must be positive"
            );
        }
        let start_key = match start {
            None => None,
            Some(key) => Some(self.extract_intent_key_bytes(key)?),
        };
        let keys = self.keys_sorted(owner, id)?;
        if keys.is_empty() {
            return Ok((None, vec![]));
        }
        let from = match start_key {
            None => 0usize,
            Some(key) => match keys.binary_search(&key) {
                Ok(i) => i + 1,
                Err(i) => i,
            },
        };
        let end = from.saturating_add(limit).min(keys.len());
        let page = keys[from..end].to_vec();
        let next = if end < keys.len() {
            Some(page[page.len() - 1].clone())
        } else {
            None
        };
        Ok((next, page))
    }

    fn ensure_unique_batch_keys(&self, keys: &[Value], op: &str) -> VmrtErr {
        let mut uniq = HashSet::new();
        for key in keys {
            let key_bytes = self.extract_intent_key_bytes(key)?;
            if !uniq.insert(key_bytes) {
                return itr_err_fmt!(
                    ItrErrCode::IntentError,
                    "intent {} duplicate key in batch",
                    op
                );
            }
        }
        Ok(())
    }

    pub fn put_many(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        pairs: Vec<(Value, Value)>,
    ) -> VmrtErr {
        let mut uniq = HashSet::new();
        for (key, val) in &pairs {
            self.validate_key_value_for_put(key, val)?;
            let key_bytes = self.extract_intent_key_bytes(key)?;
            if !uniq.insert(key_bytes) {
                return itr_err_fmt!(
                    ItrErrCode::IntentError,
                    "intent put_pairs duplicate key in batch"
                );
            }
        }
        let entry = self.require_ref(owner, id)?;
        let mut add = 0usize;
        for (key, _) in &pairs {
            if !entry.data.contains_key(key)? {
                add = add.checked_add(1).ok_or_else(|| {
                    ItrErr::new(ItrErrCode::IntentError, "intent key count overflow")
                })?;
            }
        }
        let total = entry
            .data
            .len()
            .checked_add(add)
            .ok_or_else(|| ItrErr::new(ItrErrCode::IntentError, "intent key count overflow"))?;
        if total > self.intent_key_limit {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent key count {} exceeds limit {}",
                total,
                self.intent_key_limit
            );
        }
        let entry = self.require_mut(owner, id)?;
        for (key, val) in pairs {
            entry.data.put(key, val)?;
        }
        Ok(())
    }

    pub fn put_if_absent_or_match(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        key: Value,
        val: Value,
    ) -> VmrtRes<bool> {
        let existed = self.prepare_put_mode(owner, id, &key, &val)?;
        let entry = self.require_mut(owner, id)?;
        if existed {
            let existing = entry.data.get(&key)?;
            if Self::intent_value_eq(&existing, &val)? {
                return Ok(false);
            }
            return itr_err_fmt!(ItrErrCode::IntentError, "intent existing value mismatch");
        }
        entry.data.put(key, val)?;
        Ok(true)
    }

    pub fn has_all(&self, owner: &ContractAddress, id: usize, keys: &[Value]) -> VmrtRes<bool> {
        self.ensure_unique_batch_keys(keys, "has_all")?;
        let entry = self.require_ref(owner, id)?;
        for key in keys {
            if !entry.data.contains_key(key)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn has_any(&self, owner: &ContractAddress, id: usize, keys: &[Value]) -> VmrtRes<bool> {
        self.ensure_unique_batch_keys(keys, "has_any")?;
        let entry = self.require_ref(owner, id)?;
        for key in keys {
            if entry.data.contains_key(key)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn batch_read_core(
        &self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
        op: &str,
    ) -> VmrtRes<Vec<(Vec<u8>, Value)>> {
        self.ensure_unique_batch_keys(keys, op)?;
        let entry = self.require_ref(owner, id)?;
        let mut pairs = Vec::with_capacity(keys.len());
        for key in keys {
            let key_bytes = self.extract_intent_key_bytes(key)?;
            let val = entry.data.get(key)?;
            if val.is_nil() {
                return Err(Self::key_not_found());
            }
            pairs.push((key_bytes, val));
        }
        Ok(pairs)
    }

    fn batch_remove_core(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
        op: &str,
    ) -> VmrtErr {
        self.ensure_unique_batch_keys(keys, op)?;
        {
            let entry = self.require_ref(owner, id)?;
            for key in keys {
                if !entry.data.contains_key(key)? {
                    return Err(Self::key_not_found());
                }
            }
        }
        let entry = self.require_mut(owner, id)?;
        for key in keys {
            entry.data.remove(key)?;
        }
        Ok(())
    }

    pub fn require_many(
        &self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<Vec<Value>> {
        let pairs = self.batch_read_core(owner, id, keys, "require_many")?;
        Ok(pairs.into_iter().map(|(_, v)| v).collect())
    }

    pub fn require_map(
        &self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<BTreeMap<Vec<u8>, Value>> {
        let pairs = self.batch_read_core(owner, id, keys, "require_map")?;
        Ok(BTreeMap::from_iter(pairs))
    }

    pub fn del_many(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<usize> {
        self.batch_remove_core(owner, id, keys, "del_many")?;
        Ok(keys.len())
    }

    pub fn take_many(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<Vec<Value>> {
        let pairs = self.batch_read_core(owner, id, keys, "take_many")?;
        self.batch_remove_core(owner, id, keys, "take_many")?;
        Ok(pairs.into_iter().map(|(_, v)| v).collect())
    }

    pub fn take_map(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<BTreeMap<Vec<u8>, Value>> {
        let pairs = self.batch_read_core(owner, id, keys, "take_map")?;
        self.batch_remove_core(owner, id, keys, "take_map")?;
        Ok(BTreeMap::from_iter(pairs))
    }

    fn destroy_if_now_empty(&mut self, owner: &ContractAddress, id: usize) -> VmrtErr {
        if self.len(owner, id)? == 0 {
            self.destroy(owner, id)?;
        }
        Ok(())
    }

    pub fn consume(&mut self, owner: &ContractAddress, id: usize, key: &Value) -> VmrtRes<Value> {
        let val = self.take(owner, id, key)?;
        self.destroy_if_now_empty(owner, id)?;
        Ok(val)
    }

    pub fn consume_many(
        &mut self,
        owner: &ContractAddress,
        id: usize,
        keys: &[Value],
    ) -> VmrtRes<Vec<Value>> {
        let vals = self.take_many(owner, id, keys)?;
        self.destroy_if_now_empty(owner, id)?;
        Ok(vals)
    }
}
