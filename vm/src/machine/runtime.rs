use base::GasBuckets;
use std::collections::HashMap;
use std::sync::Arc;

use super::DeferredRegistry;
use super::VmHost;
use super::{IntentRuntime, IntentRuntimeLimits};
use crate::contract::ContractObj;
use crate::rt::{GasExtra, GasTable, SpaceCap, VmrtRes};
use crate::rt::{ItrErr, ItrErrCode, VmrtErr};
use crate::space::{CtcKVMap, GKVMap, Heap, Stack};
use crate::value::ContractAddress;

const UPGRADE_HEIGHTS: &[u64] = &[];

#[derive(Default)]
pub struct WarmState {
    pub gas_table: GasTable,
    pub gas_extra: GasExtra,
    pub space_cap: SpaceCap,
    pub contracts: HashMap<ContractAddress, Arc<ContractObj>>,
    pub gas_use: GasBuckets,
    pub log_bytes_total: usize,
    pub stack_pool: Vec<Stack>,
    pub heap_pool: Vec<Heap>,
}

#[derive(Clone)]
pub struct VolatileState {
    pub global_map: GKVMap,
    pub memory_map: CtcKVMap,
    pub intents: IntentRuntime,
    pub deferred_registry: DeferredRegistry,
}

impl Default for VolatileState {
    fn default() -> Self {
        Self {
            global_map: GKVMap::default(),
            memory_map: CtcKVMap::default(),
            intents: IntentRuntime::default(),
            deferred_registry: DeferredRegistry::default(),
        }
    }
}

#[derive(Default)]
pub struct Runtime {
    cfg_height: u64,
    next_upgrade: u64,
    /// Number of active VM entry frames (gas-independent); guards synchronous
    /// contract-to-contract transfer callbacks from exhausting the native call stack.
    reentry_depth: usize,
    pub warm: WarmState,
    pub volatile: VolatileState,
}

impl Runtime {
    pub fn create(height: u64) -> Self {
        let cap = SpaceCap::new(height);
        Self {
            cfg_height: height,
            next_upgrade: Self::next_upgrade_after(height),
            reentry_depth: 0,
            warm: WarmState {
                space_cap: cap.clone(),
                gas_extra: GasExtra::new(height),
                gas_table: GasTable::new(height),
                ..Default::default()
            },
            volatile: VolatileState {
                global_map: GKVMap::with_key_max(cap.global, cap.kv_key_size),
                memory_map: CtcKVMap::with_key_max(cap.memory, cap.kv_key_size),
                intents: IntentRuntime::new(IntentRuntimeLimits::from_space_cap(&cap)),
                deferred_registry: DeferredRegistry::new(),
            },
        }
    }

    pub fn reclaim(&mut self) {
        self.warm.gas_use = GasBuckets::default();
        self.warm.log_bytes_total = 0;
        self.reentry_depth = 0;
        self.volatile.global_map.clear();
        self.volatile.memory_map.clear();
        self.volatile.intents.clear();
        self.volatile.deferred_registry.clear();
        self.warm.contracts.clear();
    }

    pub fn reset(&mut self, height: u64) {
        if height >= self.cfg_height && height < self.next_upgrade {
            return;
        }
        self.reset_gascap(height);
    }

    pub fn enter_reentry(&mut self) -> VmrtRes<()> {
        let next = self
            .reentry_depth
            .checked_add(1)
            .ok_or_else(|| ItrErr::new(ItrErrCode::OutOfCallDepth, "vm re-entry depth overflow"))?;
        let max = self.warm.space_cap.reentry_level.saturating_add(1) as usize;
        if next > max {
            return itr_err_fmt!(
                ItrErrCode::OutOfCallDepth,
                "vm re-entry depth {} exceeded limit {}",
                next.saturating_sub(1),
                self.warm.space_cap.reentry_level
            );
        }
        self.reentry_depth = next;
        Ok(())
    }

    pub fn leave_reentry(&mut self) -> VmrtRes<()> {
        self.reentry_depth = self.reentry_depth.checked_sub(1).ok_or_else(|| {
            ItrErr::new(ItrErrCode::OutOfCallDepth, "vm re-entry depth underflow")
        })?;
        Ok(())
    }

    fn reset_gascap(&mut self, height: u64) {
        let cap = SpaceCap::new(height);
        self.cfg_height = height;
        self.next_upgrade = Self::next_upgrade_after(height);
        self.volatile
            .global_map
            .reset_with_key_max(cap.global, cap.kv_key_size);
        self.volatile
            .memory_map
            .reset_with_key_max(cap.memory, cap.kv_key_size);
        self.volatile
            .intents
            .reset(IntentRuntimeLimits::from_space_cap(&cap));
        self.warm.space_cap = cap;
        self.warm.gas_extra = GasExtra::new(height);
        self.warm.gas_table = GasTable::new(height);
        self.warm.log_bytes_total = 0;
    }

    #[inline(always)]
    pub fn cfg_height(&self) -> u64 {
        self.cfg_height
    }

    #[inline(always)]
    pub fn gas_use(&self) -> GasBuckets {
        self.warm.gas_use
    }

    #[inline(always)]
    pub fn next_compute_used(&self, add: i64) -> VmrtRes<i64> {
        self.preview_bucket_add(
            self.warm.gas_use.compute,
            add,
            self.warm.gas_extra.compute_limit,
            "compute gas overflow",
            "compute gas limit exceeded",
        )
    }

    #[inline(always)]
    pub fn next_resource_used(&self, add: i64) -> VmrtRes<i64> {
        self.preview_bucket_add(
            self.warm.gas_use.resource,
            add,
            self.warm.gas_extra.resource_limit,
            "resource gas overflow",
            "resource gas limit exceeded",
        )
    }

    #[inline(always)]
    pub fn next_storage_used(&self, add: i64) -> VmrtRes<i64> {
        self.preview_bucket_add(
            self.warm.gas_use.storage,
            add,
            self.warm.gas_extra.storage_limit,
            "storage gas overflow",
            "storage gas limit exceeded",
        )
    }

    #[inline(always)]
    pub fn commit_gas_use(&mut self, compute: i64, resource: i64, storage: i64) {
        self.warm.gas_use.compute = compute;
        self.warm.gas_use.resource = resource;
        self.warm.gas_use.storage = storage;
    }

    fn charge_and_commit_gas<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        add_compute: i64,
        add_resource: i64,
        add_storage: i64,
    ) -> VmrtErr {
        let next_compute = self.next_compute_used(add_compute)?;
        let next_resource = self.next_resource_used(add_resource)?;
        let next_storage = self.next_storage_used(add_storage)?;
        let total = add_compute
            .checked_add(add_resource)
            .and_then(|v| v.checked_add(add_storage))
            .ok_or_else(|| ItrErr::new(ItrErrCode::OutOfGas, "gas cost overflow"))?;
        host.gas_charge(total)?;
        self.commit_gas_use(next_compute, next_resource, next_storage);
        Ok(())
    }

    pub fn settle_new_contract_load_gas<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        bytes: usize,
    ) -> VmrtErr {
        let gas = self.warm.gas_extra.new_contract_load + self.warm.gas_extra.contract_bytes(bytes);
        self.charge_and_commit_gas(host, 0, gas, 0)
    }

    pub fn settle_resource_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr {
        self.charge_and_commit_gas(host, 0, gas, 0)
    }

    pub fn settle_compute_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr {
        self.charge_and_commit_gas(host, gas, 0, 0)
    }

    fn preview_bucket_add(
        &self,
        cur: i64,
        add: i64,
        limit: i64,
        overflow_msg: &str,
        limit_msg: &str,
    ) -> VmrtRes<i64> {
        use crate::rt::{ItrErr, ItrErrCode};

        if add < 0 {
            return Err(ItrErr::new(
                ItrErrCode::GasError,
                &format!("gas cost invalid: {}", add),
            ));
        }
        let next = cur
            .checked_add(add)
            .ok_or_else(|| ItrErr::new(ItrErrCode::OutOfGas, overflow_msg))?;
        if limit > 0 && next > limit {
            return Err(ItrErr::new(
                ItrErrCode::OutOfGas,
                &format!("{}: used {} > limit {}", limit_msg, next, limit),
            ));
        }
        Ok(next)
    }

    pub fn stack_allocat(&mut self) -> Stack {
        self.warm.stack_pool.pop().unwrap_or_default()
    }

    pub fn stack_reclaim(&mut self, stk: Stack) {
        self.warm.stack_pool.push(stk);
    }

    pub fn heap_allocat(&mut self) -> Heap {
        self.warm.heap_pool.pop().unwrap_or_default()
    }

    pub fn heap_reclaim(&mut self, heap: Heap) {
        self.warm.heap_pool.push(heap);
    }

    fn next_upgrade_after(height: u64) -> u64 {
        for &h in UPGRADE_HEIGHTS {
            if h > height {
                return h;
            }
        }
        u64::MAX
    }
}
