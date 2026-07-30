use std::any::Any;
use std::sync::Arc;

use field::Address;
use sys::{Rerr, Ret};

use crate::iface::action::ActOut;
use crate::iface::transaction::Transaction;
use crate::iface::vm::{P2sh, Vm, VmEntry};
use crate::ledger::TexLedger;
use crate::registry::ExecutionServices;
use crate::runtime::{AddrOrPtr, Env, ExecFrom, GasBuckets};
use crate::state::{LogEntry, StateChunkRef, StateLayer};

pub trait Context {
    // Inputs: immutable services, execution environment, and transaction.
    fn services(&self) -> Arc<dyn ExecutionServices>;
    fn env(&self) -> &Env;
    fn tx(&self) -> &dyn Transaction;

    // Execution state and signature contract. Dispatchers restore `exec_from`
    // after each call; implementations must retain that state exactly.
    fn exec_from(&self) -> ExecFrom;
    fn exec_from_set(&mut self, from: ExecFrom);
    fn check_sign(&mut self, adr: &Address) -> Rerr;

    // Address, state, and execution-log access.
    fn addr(&self, ptr: &AddrOrPtr) -> Ret<Address> {
        ptr.real(&self.env().tx.addrs)
    }
    fn layer(&mut self) -> &mut dyn StateLayer;
    fn emit_log(&mut self, entry: LogEntry);

    // Gas and rollback. Volatile snapshots must preserve the implementation's
    // rollback law (currently charged gas remains while rebates roll back).
    fn gas_remaining(&self) -> i64;
    fn gas_charge(&mut self, gas: i64) -> Rerr;
    fn gas_rebate(&mut self, gas: i64) -> Rerr;
    fn gas_initialize(&mut self, budget: i64) -> Rerr;
    fn gas_refund(&mut self) -> Rerr;

    fn snapshot_volatile(&self) -> Box<dyn Any>;
    fn restore_volatile(&mut self, snap: Box<dyn Any>);

    // Action dispatch and child execution. A context without child layers may
    // explicitly report unsupported child execution through the default.
    fn action_call(&mut self, kind: u16, body: Vec<u8>) -> Ret<ActOut>;
    fn exec_ast_child(
        &mut self,
        _run: &mut dyn FnMut(&mut dyn Context) -> Ret<ActOut>,
    ) -> Ret<ActOut> {
        sys::errf!("AST child execution not supported by this context")
    }

    // VM slot. `vm_take` / `vm_put` are required so VM ownership is never
    // silently replaced by an `EmptyVm`; `None` means the VM is already active.
    fn vm_take(&mut self) -> Option<Box<dyn Vm>>;
    fn vm_put(&mut self, vm: Box<dyn Vm>);

    fn vm_call(&mut self, entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)> {
        const TYPE3: u8 = 3;
        if self.env().tx.ty < TYPE3 {
            return sys::errf!(
                "current transaction type {} too low for vm entry, requires at least {}",
                self.env().tx.ty,
                TYPE3
            );
        }
        let Some(mut vm) = self.vm_take() else {
            return sys::errf!("vm re-entered via ctx while on machine (forbidden by slot law)");
        };
        let res = vm.call(self.as_context_mut(), entry);
        self.vm_put(vm);
        res
    }
    fn as_context_mut(&mut self) -> &mut dyn Context;

    fn release_chunk(self: Box<Self>) -> Ret<StateChunkRef> {
        sys::errf!("context does not support releasing its state chunk")
    }

    fn run_deferred_phase(&mut self) -> Rerr {
        let Some(mut vm) = self.vm_take() else {
            return sys::errf!(
                "deferred phase entered while vm on machine (forbidden by slot law)"
            );
        };
        let res = vm.drain_deferred(self.as_context_mut());
        self.vm_put(vm);
        res
    }

    fn vm_peek(&mut self) -> Option<&mut (dyn Vm + 'static)> {
        None
    }

    // Settlement, transaction effects, and P2SH witnesses. P2SH lookup is an
    // optional capability; writes are required to avoid silently dropping proof.
    fn tex_ledger(&self) -> &TexLedger;
    fn tex_ledger_mut_top(&mut self) -> Ret<&mut TexLedger> {
        sys::errf!("tex ledger write only allowed in TOP context")
    }

    fn p2sh(&self, _addr: &Address) -> Ret<&dyn P2sh> {
        sys::errf!("p2sh not found")
    }
    fn p2sh_count(&self) -> usize {
        0
    }
    fn p2sh_set(&mut self, addr: Address, p2sh: Box<dyn P2sh>) -> Rerr;
}
