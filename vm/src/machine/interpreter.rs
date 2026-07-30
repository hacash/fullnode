use std::time::Instant;

use base::{Context, GasBuckets, IntentScope, TransferRouting};
use field::Address;

use crate::frame::IntentScopeState;
use crate::rt::{CallSpec, FrameBindings, GasExtra, GasTable, ItrErr, SpaceCap, VmrtErr, VmrtRes};
use crate::space::{CtcKVMap, GKVMap, Heap, Stack};
use crate::value::Value;

use super::{ResolvedCallPlan, StubVm, VmHost};

/// Shared VM state access used by the interpreter.
///
/// The interpreter owns frame-local data and calls these methods only for the
/// duration of one instruction. In particular, no returned Runtime field
/// reference is kept across `action_call` or `drive_transfer`, which permits
/// synchronous `StubVm` recursion without unsafe aliasing.
pub(crate) trait VmMachine {
    fn height(&self) -> u64;
    fn gas_table(&self) -> GasTable;
    fn gas_extra(&self) -> GasExtra;
    fn space_cap(&self) -> SpaceCap;
    fn check_deadline(&self) -> VmrtErr;

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

impl VmMachine for StubVm {
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
            bindings,
            intent_state,
            context_addr,
            &mut self.runtime.volatile.intents,
            &mut self.runtime.volatile.deferred_registry,
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
