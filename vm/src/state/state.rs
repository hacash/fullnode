use base::{StateLayer, StateRead, numeric_state_key, numeric_state_prefix};
use field::*;
use field::{Address, Hash, Uint4};

use crate::contract::{ContractEdition, ContractSto};
use crate::rt::{GasExtra, ItrErr, ItrErrCode::*, MapItrStrErr, SpaceCap, VmrtErr, VmrtRes};
use crate::space::{VolatileKvLimits, validate_scalar_payload_len};
use crate::state::status::{StatusMap, StatusSto};
use crate::state::storage::{
    ValueSto, clamp_credit_to_cap, credit_cap_for_blocks, parse_period, period_credit,
    refund_for_live_credit, u64_to_i64_sat,
};
use crate::value::{ContractAddress, Value, ValueKey};

const KEY_CONTRACT: u8 = numeric_state_prefix(0xc9);
const KEY_CONTRACT_EDITION: u8 = numeric_state_prefix(0xca);
const KEY_CONTRACT_KV: u8 = numeric_state_prefix(0xcd);
const KEY_CONTRACT_STATUS: u8 = numeric_state_prefix(0xce);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageDebug {
    pub value: Value,
    pub live_blocks: u64,
    pub recover_blocks: u64,
    pub active: bool,
    pub recoverable: bool,
}

pub struct VMStateRead<'a> {
    sta: &'a dyn StateRead,
}

pub struct VMState<'a> {
    sta: &'a mut dyn StateLayer,
}

impl<'a> VMStateRead<'a> {
    pub fn wrap(sta: &'a dyn StateRead) -> Self {
        Self { sta }
    }

    pub fn contract(&self, key: &ContractAddress) -> Option<ContractSto> {
        state_read_get(self.sta, KEY_CONTRACT, key)
    }

    pub fn contract_edition(&self, key: &ContractAddress) -> Option<ContractEdition> {
        state_read_get(self.sta, KEY_CONTRACT_EDITION, key)
    }

    pub fn ctrtkvdb(&self, key: &ValueKey) -> Option<ValueSto> {
        state_read_get(self.sta, KEY_CONTRACT_KV, key)
    }

    pub fn ctrtstatus(&self, key: &ContractAddress) -> Option<StatusSto> {
        state_read_get(self.sta, KEY_CONTRACT_STATUS, key)
    }

    pub fn debug_storage_get(
        &self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: &Value,
    ) -> VmrtRes<Option<StorageDebug>> {
        let sk = VMState::skey(cadr, k, cap.kv_key_size)?;
        let Some(mut v) = self.ctrtkvdb(&sk) else {
            return Ok(None);
        };
        v.settle(curhei, gst)?;
        if v.is_absent() {
            return Ok(None);
        }
        let live = v.live_rest_blocks(gst)?;
        let recover = v.recover_rest_blocks(gst)?;
        Ok(Some(StorageDebug {
            value: v.data.clone(),
            live_blocks: live,
            recover_blocks: recover,
            active: v.is_active(),
            recoverable: v.is_recoverable(),
        }))
    }

    pub fn debug_status_get(&self, cap: &SpaceCap, cadr: &Address, k: &Value) -> VmrtRes<Value> {
        let caddr = VMState::status_contract_addr(cadr)?;
        let status = match self.ctrtstatus(&caddr) {
            Some(sto) => {
                let status = sto.to_status_map()?;
                status.validate_key_lengths(VMState::status_key_max(cap), StorageError)?;
                status
            }
            None => StatusMap::default(),
        };
        let key = VMState::status_key_bytes(cap, k)?;
        Ok(status.get(&key))
    }
}

impl<'a> VMState<'a> {
    pub fn wrap(sta: &'a mut dyn StateLayer) -> Self {
        Self { sta }
    }

    pub fn contract(&self, key: &ContractAddress) -> Option<ContractSto> {
        state_get(self.sta, KEY_CONTRACT, key)
    }

    pub fn contract_set(&mut self, key: &ContractAddress, v: &ContractSto) {
        state_set(self.sta, KEY_CONTRACT, key, v);
    }

    pub fn contract_del(&mut self, key: &ContractAddress) {
        state_del(self.sta, KEY_CONTRACT, key);
    }

    pub fn contract_edition(&self, key: &ContractAddress) -> Option<ContractEdition> {
        state_get(self.sta, KEY_CONTRACT_EDITION, key)
    }

    pub fn contract_edition_set(&mut self, key: &ContractAddress, v: &ContractEdition) {
        state_set(self.sta, KEY_CONTRACT_EDITION, key, v);
    }

    pub fn contract_edition_del(&mut self, key: &ContractAddress) {
        state_del(self.sta, KEY_CONTRACT_EDITION, key);
    }

    pub fn ctrtkvdb(&self, key: &ValueKey) -> Option<ValueSto> {
        state_get(self.sta, KEY_CONTRACT_KV, key)
    }

    pub fn ctrtkvdb_set(&mut self, key: &ValueKey, v: &ValueSto) {
        state_set(self.sta, KEY_CONTRACT_KV, key, v);
    }

    pub fn ctrtkvdb_del(&mut self, key: &ValueKey) {
        state_del(self.sta, KEY_CONTRACT_KV, key);
    }

    pub fn ctrtstatus(&self, key: &ContractAddress) -> Option<StatusSto> {
        state_get(self.sta, KEY_CONTRACT_STATUS, key)
    }

    pub fn ctrtstatus_set(&mut self, key: &ContractAddress, v: &StatusSto) {
        state_set(self.sta, KEY_CONTRACT_STATUS, key, v);
    }

    pub fn ctrtstatus_del(&mut self, key: &ContractAddress) {
        state_del(self.sta, KEY_CONTRACT_STATUS, key);
    }

    pub fn contract_set_sync_edition(&mut self, addr: &ContractAddress, sto: &ContractSto) {
        self.contract_set(addr, sto);
        self.contract_edition_set(addr, &sto.calc_edition());
    }

    fn status_key_max(cap: &SpaceCap) -> usize {
        VolatileKvLimits::from_space_cap(cap).key_max_bytes
    }

    fn status_contract_addr(cadr: &Address) -> VmrtRes<ContractAddress> {
        ContractAddress::from_addr(*cadr).map_ires(
            StorageError,
            format!(
                "status storage must be in contract address but got {}",
                cadr.to_readable()
            ),
        )
    }

    fn status_key_bytes(cap: &SpaceCap, key: &Value) -> VmrtRes<Vec<u8>> {
        let key = key.extract_key_bytes_with_error_code(StorageKeyInvalid)?;
        let key_max = Self::status_key_max(cap);
        if key.len() > key_max {
            return itr_err_fmt!(
                StorageKeyInvalid,
                "status key too long, max {} bytes but got {}",
                key_max,
                key.len()
            );
        }
        Ok(key)
    }

    fn status_load_by_contract(
        &self,
        cap: &SpaceCap,
        caddr: &ContractAddress,
    ) -> VmrtRes<StatusMap> {
        match self.ctrtstatus(caddr) {
            Some(sto) => {
                let status = sto.to_status_map()?;
                status.validate_key_lengths(Self::status_key_max(cap), StorageError)?;
                Ok(status)
            }
            None => Ok(StatusMap::default()),
        }
    }

    fn status_load(&self, cap: &SpaceCap, cadr: &Address) -> VmrtRes<StatusMap> {
        let caddr = Self::status_contract_addr(cadr)?;
        self.status_load_by_contract(cap, &caddr)
    }

    fn status_save_by_contract(
        &mut self,
        cap: &SpaceCap,
        caddr: &ContractAddress,
        status: &StatusMap,
    ) -> VmrtRes<()> {
        if status.is_empty() {
            self.ctrtstatus_del(caddr);
            return Ok(());
        }
        status.ensure_save_bounds(cap)?;
        let sto = StatusSto::from_status_map(status)
            .map_ires(StorageError, "serialize status object failed".to_owned())?;
        self.ctrtstatus_set(caddr, &sto);
        Ok(())
    }

    pub(crate) fn status_get_gas(gst: &GasExtra, value: &Value) -> i64 {
        if matches!(value, Value::Nil) {
            0
        } else {
            gst.status_read(value.val_size())
        }
    }

    pub(crate) fn status_put_prepare(
        cap: &SpaceCap,
        key: &Value,
        value: &Value,
    ) -> VmrtRes<(Vec<u8>, usize)> {
        let kbytes = Self::status_key_bytes(cap, key)?;
        let vlen = if matches!(value, Value::Nil) {
            0usize
        } else {
            value.check_scalar()?;
            let vlen = value.extract_bytes_len_with_error_code(StorageValSizeErr)?;
            if !SpaceCap::scalar_field_len_fits(vlen, cap.value_size) {
                let eff_max = cap.value_size.min(SpaceCap::FIELD_BYTES_SERIALIZE_MAX);
                return itr_err_fmt!(
                    StorageValSizeErr,
                    "value too long, max {} bytes but got {}",
                    eff_max,
                    vlen
                );
            }
            vlen
        };
        Ok((kbytes, vlen))
    }

    pub(crate) fn status_put_gas(
        gst: &GasExtra,
        cap: &SpaceCap,
        key: &Value,
        value: &Value,
    ) -> VmrtRes<i64> {
        let (kbytes, vlen) = Self::status_put_prepare(cap, key, value)?;
        Ok(gst.status_write(kbytes.len(), vlen))
    }

    pub(crate) fn sget(&self, cap: &SpaceCap, cadr: &Address, k: &Value) -> VmrtRes<Value> {
        let key = Self::status_key_bytes(cap, k)?;
        Ok(self.status_load(cap, cadr)?.get(&key))
    }

    pub(crate) fn sput(&mut self, cap: &SpaceCap, cadr: &Address, k: Value, v: Value) -> VmrtErr {
        let caddr = Self::status_contract_addr(cadr)?;
        let (key, _vlen) = Self::status_put_prepare(cap, &k, &v)?;
        let mut status = self.status_load_by_contract(cap, &caddr)?;
        status.set_or_remove(key, v)?;
        self.status_save_by_contract(cap, &caddr, &status)
    }

    fn skey(cadr: &Address, key: &Value, key_max: usize) -> VmrtRes<ValueKey> {
        if !cadr.is_supported() {
            return itr_err_fmt!(
                StorageError,
                "storage must be in effective address but got {}",
                cadr.to_readable()
            );
        }
        let k = key.extract_key_bytes_with_error_code(StorageKeyInvalid)?;
        if k.len() > key_max {
            return itr_err_fmt!(
                StorageKeyInvalid,
                "storage key too long, max {} bytes but got {}",
                key_max,
                k.len()
            );
        }
        let mut k = [cadr.as_ref(), &k].concat();
        if k.len() > Hash::SIZE {
            k = sys::calculate_hash(k).to_vec();
        }
        Ok(ValueKey::from(k))
    }

    fn sfetch(&mut self, curhei: u64, gst: &GasExtra, sk: &ValueKey) -> VmrtRes<Option<ValueSto>> {
        let Some(mut v) = self.ctrtkvdb(sk) else {
            return Ok(None);
        };
        v.settle(curhei, gst)?;
        if v.is_absent() {
            self.ctrtkvdb_del(sk);
            return Ok(None);
        }
        self.ctrtkvdb_set(sk, &v);
        Ok(Some(v))
    }

    pub(crate) fn sstat(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: &Value,
    ) -> VmrtRes<Value> {
        let sk = Self::skey(cadr, k, cap.kv_key_size)?;
        let Some(v) = self.sfetch(curhei, gst, &sk)? else {
            return Ok(Value::Nil);
        };
        let live = v.live_rest_blocks(gst)?;
        let recover = v.recover_rest_blocks(gst)?;
        Value::pack_tuple([Value::U64(live), Value::U64(recover)])
    }

    pub(crate) fn sload(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: &Value,
    ) -> VmrtRes<Value> {
        let sk = Self::skey(cadr, k, cap.kv_key_size)?;
        let Some(v) = self.sfetch(curhei, gst, &sk)? else {
            return Ok(Value::Nil);
        };
        if v.is_recoverable() {
            return itr_err_code!(StorageNotActive);
        }
        Ok(v.data)
    }

    pub(crate) fn snew(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: Value,
        v: Value,
        p: Value,
    ) -> VmrtRes<i64> {
        v.check_non_nil_scalar(StorageNilNotAllowed)?;
        validate_scalar_payload_len(&v, cap.value_size, StorageValSizeErr)?;
        let period = parse_period(p, cap.storage_live_max_periods)?;
        let sk = Self::skey(cadr, &k, cap.kv_key_size)?;
        if self.sfetch(curhei, gst, &sk)?.is_some() {
            return itr_err_code!(StorageKeyExists);
        }
        let unit = ValueSto::unit_for(gst, &v)?;
        let live_credit = period_credit(unit, period, cap.storage_period)?;
        let vobj = ValueSto::new(curhei, v, live_credit, 0)?;
        self.ctrtkvdb_set(&sk, &vobj);
        let gas = gst
            .storage_key_cost
            .saturating_add(u64_to_i64_sat(unit).saturating_mul(period as i64));
        Ok(gas)
    }

    pub(crate) fn sedit(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: Value,
        v: Value,
    ) -> VmrtRes<(i64, i64)> {
        v.check_non_nil_scalar(StorageNilNotAllowed)?;
        validate_scalar_payload_len(&v, cap.value_size, StorageValSizeErr)?;
        let sk = Self::skey(cadr, &k, cap.kv_key_size)?;
        let Some(mut old) = self.sfetch(curhei, gst, &sk)? else {
            return itr_err_code!(StorageKeyNotFind);
        };
        if !old.is_active() {
            return itr_err_code!(StorageNotActive);
        }
        old.data = v;
        old.charge = field::BlockHeight::from(curhei);
        let unit = ValueSto::unit_for(gst, &old.data)?;
        let live_cap = credit_cap_for_blocks(
            unit,
            cap.storage_live_max_blocks(),
            "live credit cap overflow",
        )?;
        let recover_cap = credit_cap_for_blocks(
            unit,
            cap.storage_recv_max_blocks(),
            "recover credit cap overflow",
        )?;
        let (live_credit, trimmed_live) =
            clamp_credit_to_cap(old.live_credit.uint() as u64, live_cap);
        let (recover_credit, _) =
            clamp_credit_to_cap(old.recover_credit.uint() as u64, recover_cap);
        old.live_credit = Uint4::from(ValueSto::credit_u32(
            live_credit,
            "edit live credit overflow",
        )?);
        old.recover_credit = Uint4::from(ValueSto::credit_u32(
            recover_credit,
            "edit recover credit overflow",
        )?);
        self.ctrtkvdb_set(&sk, &old);
        let fee = u64_to_i64_sat(unit).saturating_mul(gst.storage_edit_mul);
        let rebate = refund_for_live_credit(trimmed_live, cap.storage_period);
        Ok((fee, rebate))
    }

    pub(crate) fn srent(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: Value,
        p: Value,
    ) -> VmrtRes<i64> {
        let period = parse_period(p, cap.storage_live_max_periods)?;
        let sk = Self::skey(cadr, &k, cap.kv_key_size)?;
        let Some(mut v) = self.sfetch(curhei, gst, &sk)? else {
            return itr_err_code!(StorageKeyNotFind);
        };
        let unit = v.unit(gst)?;
        let add_credit = period_credit(unit, period, cap.storage_period)?;
        let add_blocks = period
            .checked_mul(cap.storage_period)
            .ok_or_else(|| ItrErr::new(StorageError, "rent blocks overflow"))?;
        let cur_blocks = crate::state::storage::rest_blocks(v.live_credit.uint() as u64, unit)?;
        let next_blocks = cur_blocks
            .checked_add(add_blocks)
            .ok_or_else(|| ItrErr::new(StorageError, "rent overflow"))?;
        if next_blocks > cap.storage_live_max_blocks() {
            return itr_err_fmt!(
                StoragePeriodErr,
                "live block budget exceeded, max {} blocks",
                cap.storage_live_max_blocks()
            );
        }
        let next_credit = (v.live_credit.uint() as u64)
            .checked_add(add_credit)
            .ok_or_else(|| ItrErr::new(StorageError, "rent credit overflow"))?;
        v.live_credit = Uint4::from(ValueSto::credit_u32(next_credit, "rent credit overflow")?);
        v.charge = field::BlockHeight::from(curhei);
        self.ctrtkvdb_set(&sk, &v);
        Ok(u64_to_i64_sat(unit).saturating_mul(period as i64))
    }

    pub(crate) fn srecv(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: Value,
        p: Value,
    ) -> VmrtRes<i64> {
        let period = parse_period(p, cap.storage_recv_max_periods)?;
        let sk = Self::skey(cadr, &k, cap.kv_key_size)?;
        let Some(mut v) = self.sfetch(curhei, gst, &sk)? else {
            return itr_err_code!(StorageKeyNotFind);
        };
        let unit = v.unit(gst)?;
        let add_credit = period_credit(unit, period, cap.storage_period)?;
        let add_blocks = period
            .checked_mul(cap.storage_period)
            .ok_or_else(|| ItrErr::new(StorageError, "recover blocks overflow"))?;
        let cur_blocks = crate::state::storage::rest_blocks(v.recover_credit.uint() as u64, unit)?;
        let next_blocks = cur_blocks
            .checked_add(add_blocks)
            .ok_or_else(|| ItrErr::new(StorageError, "recover overflow"))?;
        if next_blocks > cap.storage_recv_max_blocks() {
            return itr_err_fmt!(
                StoragePeriodErr,
                "recover block budget exceeded, max {} blocks",
                cap.storage_recv_max_blocks()
            );
        }
        let next_credit = (v.recover_credit.uint() as u64)
            .checked_add(add_credit)
            .ok_or_else(|| ItrErr::new(StorageError, "recover credit overflow"))?;
        v.recover_credit = Uint4::from(ValueSto::credit_u32(
            next_credit,
            "recover credit overflow",
        )?);
        v.charge = field::BlockHeight::from(curhei);
        self.ctrtkvdb_set(&sk, &v);
        Ok(u64_to_i64_sat(unit)
            .saturating_mul(period as i64)
            .saturating_div(3))
    }

    pub(crate) fn sdel(
        &mut self,
        gst: &GasExtra,
        cap: &SpaceCap,
        curhei: u64,
        cadr: &Address,
        k: Value,
    ) -> VmrtRes<i64> {
        let sk = Self::skey(cadr, &k, cap.kv_key_size)?;
        let Some(mut v) = self.ctrtkvdb(&sk) else {
            return Ok(0);
        };
        v.settle(curhei, gst)?;
        if v.is_absent() {
            self.ctrtkvdb_del(&sk);
            return Ok(0);
        }
        let refund = refund_for_live_credit(v.live_credit.uint() as u64, cap.storage_period);
        self.ctrtkvdb_del(&sk);
        let refund = refund
            .checked_add(gst.storage_key_cost)
            .ok_or_else(|| ItrErr::new(StorageError, "delete refund overflow"))?;
        Ok(refund)
    }
}

fn state_key<K: Encode>(idx: u8, key: &K) -> Vec<u8> {
    numeric_state_key(idx, key)
}

fn state_get<K: Encode, V: Decode + Default>(sta: &dyn StateLayer, idx: u8, key: &K) -> Option<V> {
    let k = state_key(idx, key);
    sta.get(&k).and_then(|b| {
        let (v, _) = V::decode(b.as_ref()).ok()?;
        Some(v)
    })
}

fn state_read_get<K: Encode, V: Decode + Default>(
    sta: &dyn StateRead,
    idx: u8,
    key: &K,
) -> Option<V> {
    let k = state_key(idx, key);
    sta.get(&k).and_then(|b| {
        let (v, _) = V::decode(b.as_ref()).ok()?;
        Some(v)
    })
}

fn state_set<K: Encode, V: Encode>(sta: &mut dyn StateLayer, idx: u8, key: &K, v: &V) {
    let k = state_key(idx, key);
    sta.set(&k, v.encode());
}

fn state_del<K: Encode>(sta: &mut dyn StateLayer, idx: u8, key: &K) {
    let k = state_key(idx, key);
    sta.del(&k);
}
