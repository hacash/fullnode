use std::time::Instant;

use base::{Context, GasBuckets, IntentScope, TransferRouting};
use field::Address;

use crate::frame::IntentScopeState;
use crate::rt::{CallSpec, FrameBindings, GasExtra, GasTable, ItrErr, SpaceCap, VmrtErr, VmrtRes};
use crate::space::{CtcKVMap, GKVMap, Heap, Stack};
use crate::value::Value;

use super::{NativeVm, ResolvedCallPlan, VmHost};

/// Shared VM state access used by the interpreter, only for the duration of one instruction; no
/// Runtime reference is kept across `action_call`/`drive_transfer`, allowing synchronous recursion.
pub(crate) trait VmMachine {
    fn height(&self) -> u64;
    fn gas_table(&self) -> GasTable;
    fn gas_extra(&self) -> GasExtra;
    fn space_cap(&self) -> SpaceCap;
    fn check_deadline(&self) -> VmrtErr;
    /// Wall-clock deadline for long-running pure calls (None on consensus
    /// machines). Used to abort calc kernels mid-run.
    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn gas_use(&self) -> GasBuckets;
    fn commit_gas_use(&mut self, gas: GasBuckets);
    fn log_bytes_total(&self) -> usize;
    fn commit_log_bytes(&mut self, total: usize);

    fn global_map_mut(&mut self) -> &mut GKVMap;
    fn memory_map_mut(&mut self) -> &mut CtcKVMap;
    fn stack_allocat(&mut self) -> Stack;
    fn stack_reclaim(&mut self, stack: Stack);
    fn heap_allocat(&mut self) -> Heap;
    fn heap_reclaim(&mut self, heap: Heap);

    fn settle_resource_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr;
    fn settle_compute_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr;
    /// Execute a calc kernel of the running contract's `calcfuncs[idx]`.
    /// Returns the output and the gas consumed (charged by the caller on
    /// both success and abort). `limits` is derived from the current gas
    /// state by the caller. Machines without calc support reject:
    /// contracts can only reach this through the height-gated deployment
    /// window in `contract::mod`, so the default keeps other machine
    /// implementations unaffected.
    fn call_calc<H: VmHost + base::Context + ?Sized>(
        &mut self,
        _host: &mut H,
        _bindings: &FrameBindings,
        idx: u8,
        _input: &[u8],
        _limits: crate::calc::CalcRunLimits,
    ) -> VmrtRes<crate::calc::CalcExec> {
        itr_err_fmt!(
            crate::rt::ItrErrCode::CallNotExist,
            "calc func not supported by this machine (idx {})",
            idx
        )
    }
    fn call_ntctl(
        &mut self,
        exec: crate::rt::ExecCtx,
        cap: &SpaceCap,
        bindings: &mut FrameBindings,
        intent_state: &mut IntentScopeState,
        context_addr: &Address,
        idx: u8,
        argv: Value,
    ) -> VmrtRes<(Value, i64)>;
    fn plan_user_call<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        call: &CallSpec,
        bindings: &FrameBindings,
    ) -> VmrtRes<ResolvedCallPlan>;
    fn drive_transfer(
        &mut self,
        ctx: &mut dyn Context,
        routing: TransferRouting,
        intent_scope: IntentScope,
    ) -> VmrtRes<()>;
}

impl VmMachine for NativeVm {
    fn height(&self) -> u64 {
        self.runtime.cfg_height()
    }

    fn gas_table(&self) -> GasTable {
        self.runtime.warm.gas_table.clone()
    }

    fn gas_extra(&self) -> GasExtra {
        self.runtime.warm.gas_extra.clone()
    }

    fn space_cap(&self) -> SpaceCap {
        self.runtime.warm.space_cap.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn check_deadline(&self) -> VmrtErr {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(ItrErr::new(
                crate::rt::ItrErrCode::ExecutionDeadline,
                "VM execution deadline exceeded",
            ));
        }
        Ok(())
    }

    fn gas_use(&self) -> GasBuckets {
        self.runtime.warm.gas_use
    }

    fn commit_gas_use(&mut self, gas: GasBuckets) {
        self.runtime.warm.gas_use = gas;
    }

    fn log_bytes_total(&self) -> usize {
        self.runtime.warm.log_bytes_total
    }

    fn commit_log_bytes(&mut self, total: usize) {
        self.runtime.warm.log_bytes_total = total;
    }

    fn global_map_mut(&mut self) -> &mut GKVMap {
        &mut self.runtime.volatile.global_map
    }

    fn memory_map_mut(&mut self) -> &mut CtcKVMap {
        &mut self.runtime.volatile.memory_map
    }

    fn stack_allocat(&mut self) -> Stack {
        self.runtime.stack_allocat()
    }

    fn stack_reclaim(&mut self, stack: Stack) {
        self.runtime.stack_reclaim(stack)
    }

    fn heap_allocat(&mut self) -> Heap {
        self.runtime.heap_allocat()
    }

    fn heap_reclaim(&mut self, heap: Heap) {
        self.runtime.heap_reclaim(heap)
    }

    fn settle_resource_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr {
        self.runtime.settle_resource_gas(host, gas)
    }

    fn settle_compute_gas<H: VmHost + ?Sized>(&mut self, host: &mut H, gas: i64) -> VmrtErr {
        self.runtime.settle_compute_gas(host, gas)
    }

    fn call_calc<H: VmHost + base::Context + ?Sized>(
        &mut self,
        host: &mut H,
        bindings: &FrameBindings,
        idx: u8,
        input: &[u8],
        limits: crate::calc::CalcRunLimits,
    ) -> VmrtRes<crate::calc::CalcExec> {
        use crate::rt::ItrErrCode::*;
        let Some(addr) = bindings.code_contract.clone() else {
            return itr_err_fmt!(CallNotExist, "calc call requires a contract context");
        };
        let cobj = self.runtime.resolve_contract(host, &addr)?;
        let Some(calc) = cobj.calcfns.get(idx as usize) else {
            return itr_err_fmt!(CallNotExist, "calcfunc idx {} not found", idx);
        };
        Ok(calc.run(input, limits))
    }

    fn call_ntctl(
        &mut self,
        exec: crate::rt::ExecCtx,
        cap: &SpaceCap,
        bindings: &mut FrameBindings,
        intent_state: &mut IntentScopeState,
        context_addr: &Address,
        idx: u8,
        argv: Value,
    ) -> VmrtRes<(Value, i64)> {
        crate::native::call_ntctl(
            exec,
            cap,
            &self.runtime.warm.gas_extra,
            bindings,
            intent_state,
            context_addr,
            &mut self.runtime.volatile.intents,
            &mut self.runtime.volatile.deferred_registry,
            &mut self.runtime.volatile.memory_map,
            idx,
            argv,
        )
    }

    fn plan_user_call<H: VmHost + ?Sized>(
        &mut self,
        host: &mut H,
        call: &CallSpec,
        bindings: &FrameBindings,
    ) -> VmrtRes<ResolvedCallPlan> {
        self.runtime.plan_user_call(host, call, bindings)
    }

    fn drive_transfer(
        &mut self,
        ctx: &mut dyn Context,
        routing: TransferRouting,
        intent_scope: IntentScope,
    ) -> VmrtRes<()> {
        self.drive_transfer_inner(ctx, routing, intent_scope)
    }
}
