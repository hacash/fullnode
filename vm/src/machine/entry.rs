use std::any::Any;
use std::sync::Arc;

use base::{Context, ExecFrom, GasBuckets, IntentScope, with_exec_from};
use sys::Ret;

use crate::frame::CallFrame;
use crate::rt::{AbstCall, CodeType, EntryKind, FnObj, FrameBindings, ItrErr};
use crate::value::Value;

use super::{Runtime, StubVm, VmHost, VmRequest};

impl StubVm {
    pub fn new(height: u64, host_action_count: usize) -> Self {
        Self {
            runtime: Runtime::create(height),
            entries: Vec::new(),
            host_action_count,
            deadline: None,
        }
    }

    pub fn height(&self) -> u64 {
        self.runtime.cfg_height()
    }

    pub fn host_action_count(&self) -> usize {
        self.host_action_count
    }

    fn push_entry(&mut self, kind: EntryKind) -> Ret<()> {
        self.runtime.enter_reentry().map_err(sys::Error::from)?;
        self.entries.push(super::EntryFrame {
            kind,
            gas_base: self.runtime.gas_use(),
            call_base: kind.call_base(&self.runtime.warm.gas_extra),
        });
        Ok(())
    }

    fn pop_entry(&mut self) -> Ret<super::EntryFrame> {
        let entry = self
            .entries
            .pop()
            .ok_or_else(|| sys::Error::fault("vm entry stack empty"))?;
        self.runtime.leave_reentry().map_err(sys::Error::from)?;
        Ok(entry)
    }

    fn settle_entry_return_cost(
        &mut self,
        ctx: &mut dyn Context,
        entry: super::EntryFrame,
    ) -> Ret<GasBuckets> {
        let mut cost = self
            .runtime
            .gas_use()
            .checked_sub(entry.gas_base)
            .ok_or_else(|| {
                sys::Error::fault(format!(
                    "gas cost underflow: total={:?}, base={:?}",
                    self.runtime.gas_use(),
                    entry.gas_base
                ))
            })?;
        if entry.call_base > 0 {
            self.runtime
                .settle_compute_gas(ctx, entry.call_base)
                .map_err(sys::Error::from)?;
            cost.compute = cost.compute.saturating_add(entry.call_base);
        }
        if cost.total() <= 0 {
            return sys::errf!("{:?} gas cost invalid: {}", entry.kind, cost.total());
        }
        Ok(cost)
    }

    pub(super) fn run_entry(
        &mut self,
        ctx: &mut dyn Context,
        kind: EntryKind,
        run: impl FnOnce(&mut Self, &mut dyn Context) -> Result<Value, ItrErr>,
    ) -> Ret<(GasBuckets, Value)> {
        self.push_entry(kind)?;
        // Match dev entry semantics: every VM entry executes under `ExecFrom::Call`
        // (dev wraps `run_vm_entry_ret/xret` in `with_exec_from(ctx, ExecFrom::Call, ..)`).
        // Keeps ctx.exec_from() observable by VM-hosted code identical to fullnodedev.
        let result =
            with_exec_from(ctx, ExecFrom::Call, |ctx| run(self, ctx)).map_err(sys::Error::from);
        let entry = self.pop_entry()?;
        let settle = self.settle_entry_return_cost(ctx, entry);
        match (result, settle) {
            (Err(exec_err), Err(settle_err)) => Err(sys::Error::fault(format!(
                "{} | secondary: {}",
                exec_err, settle_err
            ))),
            (Err(exec_err), _) => Err(exec_err),
            (Ok(_), Err(settle_err)) => Err(settle_err),
            (Ok(retv), Ok(cost)) => Ok((cost, retv)),
        }
    }

    pub(super) fn do_call(
        &mut self,
        ctx: &mut dyn Context,
        exec: crate::rt::ExecCtx,
        code: &FnObj,
        bindings: FrameBindings,
        param: Option<Value>,
    ) -> Result<Value, ItrErr> {
        let mut frame = CallFrame::new();
        let res = frame.start_call(self, ctx, exec, code, bindings, param);
        frame.reclaim(self);
        res
    }

    fn main_call_raw(
        &mut self,
        ctx: &mut dyn Context,
        code_type: CodeType,
        codes: Arc<[u8]>,
    ) -> Result<Value, ItrErr> {
        let fnobj = FnObj::plain(code_type, codes, 0, None);
        let bindings = ctx.main_entry_bindings();
        let rv = self.do_call(ctx, EntryKind::Main.root_exec(), &fnobj, bindings, None)?;
        rv.check_vm_boundary_retv()?;
        Ok(rv)
    }

    fn abst_call_raw(
        &mut self,
        ctx: &mut dyn Context,
        kind: AbstCall,
        contract_addr: crate::value::ContractAddress,
        intent_scope: IntentScope,
        param: Value,
    ) -> Result<Value, ItrErr> {
        let exec = EntryKind::Abst.root_exec();
        exec.ensure_call_depth(&self.runtime.warm.space_cap)?;
        param.check_vm_boundary_argv()?;
        param.check_boundary_value_cap(&self.runtime.warm.space_cap)?;
        param.check_container_cap(&self.runtime.warm.space_cap)?;
        let hit = self
            .runtime
            .resolve_abstfn(ctx, &contract_addr, kind)?
            .ok_or_else(|| {
                ItrErr::new(
                    crate::rt::ItrErrCode::CallNotExist,
                    &format!("abst call {:?} not found in {}", kind, contract_addr),
                )
            })?;
        let defer_auth = kind.can_register_defer().then_some(contract_addr);
        let old_defer_auth = self
            .runtime
            .volatile
            .deferred_registry
            .replace_defer_auth(defer_auth);
        let rv = self.do_call(
            ctx,
            exec,
            hit.fnobj.as_ref(),
            FrameBindings::contract(contract_addr, hit.owner, hit.lib_table)
                .with_intent_scope(intent_scope),
            Some(param),
        );
        self.runtime
            .volatile
            .deferred_registry
            .replace_defer_auth(old_defer_auth);
        rv
    }

    pub(super) fn check_vm_return_value(rv: &Value, err_msg: &str) -> Ret<()> {
        rv.check_vm_boundary_retv().map_err(sys::Error::from)?;
        let failed = match rv {
            Value::Nil => None,
            Value::Bool(false) => None,
            Value::Bool(true) => Some("code 1".to_owned()),
            Value::U8(n) => (*n != 0).then(|| format!("code {}", n)),
            Value::U16(n) => (*n != 0).then(|| format!("code {}", n)),
            Value::U32(n) => (*n != 0).then(|| format!("code {}", n)),
            Value::U64(n) => (*n != 0).then(|| format!("code {}", n)),
            Value::U128(n) => (*n != 0).then(|| format!("code {}", n)),
            Value::Bytes(buf) => (!crate::value::buf_is_empty_or_all_zero(buf))
                .then(|| format!("bytes 0x{}", hex::encode(buf))),
            Value::Address(addr) => (!crate::value::buf_is_empty_or_all_zero(addr.as_bytes()))
                .then(|| format!("address {}", addr.to_readable())),
            Value::Tuple(_) | Value::Compo(_) => Some(format!("object {}", rv.to_json())),
            Value::Handle(_) => Some("handle".to_owned()),
        };
        match failed {
            None => Ok(()),
            Some(detail) => Err(sys::Error::revert(format!(
                "{} return error {}",
                err_msg, detail
            ))),
        }
    }

    fn run_main_entry_value(
        &mut self,
        ctx: &mut dyn Context,
        code_type: CodeType,
        codes: Arc<[u8]>,
    ) -> Ret<(GasBuckets, Value)> {
        self.run_entry(ctx, EntryKind::Main, move |vm, ctx| {
            vm.main_call_raw(ctx, code_type, codes)
        })
    }

    fn run_main_entry(
        &mut self,
        ctx: &mut dyn Context,
        code_type: CodeType,
        codes: Arc<[u8]>,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        let (cost, rv) = self.run_main_entry_value(ctx, code_type, codes)?;
        Self::check_vm_return_value(&rv, "main call")?;
        Ok((cost, Box::new(rv)))
    }

    fn run_sandbox_main_entry(
        &mut self,
        ctx: &mut dyn Context,
        code_type: CodeType,
        codes: Arc<[u8]>,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        let (cost, rv) = self.run_main_entry_value(ctx, code_type, codes)?;
        Ok((cost, Box::new(rv)))
    }

    pub(super) fn run_abst_entry(
        &mut self,
        ctx: &mut dyn Context,
        kind: AbstCall,
        contract_addr: crate::value::ContractAddress,
        intent_scope: IntentScope,
        param: Value,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        let label = format!("call {}.{:?}", contract_addr, kind);
        let (cost, rv) = self.run_entry(ctx, EntryKind::Abst, move |vm, ctx| {
            vm.abst_call_raw(ctx, kind, contract_addr, intent_scope, param)
        })?;
        Self::check_vm_return_value(&rv, &label)?;
        Ok((cost, Box::new(rv)))
    }

    pub(super) fn run_request(
        &mut self,
        ctx: &mut dyn Context,
        req: VmRequest,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        match req {
            VmRequest::Main { code_type, codes } => self.run_main_entry(ctx, code_type, codes),
            VmRequest::SandboxMain { code_type, codes } => {
                self.run_sandbox_main_entry(ctx, code_type, codes)
            }
            VmRequest::Abst {
                kind,
                contract_addr,
                intent_scope,
                param,
            } => self.run_abst_entry(ctx, kind, contract_addr, intent_scope, param),
        }
    }
}

#[cfg(test)]
mod entry_semantics_tests {
    use super::*;
    use crate::machine::test_ctx::TestCtx;
    use crate::rt::{ItrErr, ItrErrCode};
    use base::ExecFrom;

    /// Dev entry semantics: every VM entry executes under `ExecFrom::Call`
    /// (dev wraps `run_vm_entry_ret/xret` in `with_exec_from(ctx, Call, ..)`),
    /// and the caller's exec_from is restored afterwards.
    #[test]
    fn run_entry_executes_under_exec_from_call_and_restores() {
        let mut vm = StubVm::new(1, 0);
        let mut ctx = TestCtx::new();
        assert_eq!(ctx.exec_from(), ExecFrom::Top);
        let (_, rv) = vm
            .run_entry(&mut ctx, EntryKind::Main, |vm, ctx| {
                assert_eq!(ctx.exec_from(), ExecFrom::Call);
                vm.runtime.settle_compute_gas(ctx, 5).unwrap();
                Ok(Value::Nil)
            })
            .unwrap();
        assert!(rv.is_nil());
        assert_eq!(ctx.exec_from(), ExecFrom::Top);
    }

    #[test]
    fn run_entry_restores_exec_from_on_error() {
        let mut vm = StubVm::new(1, 0);
        let mut ctx = TestCtx::new();
        let err = vm
            .run_entry(&mut ctx, EntryKind::Main, |vm, ctx| {
                assert_eq!(ctx.exec_from(), ExecFrom::Call);
                vm.runtime.settle_compute_gas(ctx, 5).unwrap();
                Err(ItrErr::new(ItrErrCode::ThrowAbort, "boom"))
            })
            .unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        assert_eq!(ctx.exec_from(), ExecFrom::Top);
    }

    /// Nested entries (contract-to-contract / transfer recursion) keep
    /// `ExecFrom::Call` at every level and unwind back to the caller's value.
    #[test]
    fn nested_entries_keep_exec_from_call_and_restore_outer() {
        let mut vm = StubVm::new(1, 0);
        let mut ctx = TestCtx::new();
        vm.run_entry(&mut ctx, EntryKind::Main, |vm, ctx| {
            assert_eq!(ctx.exec_from(), ExecFrom::Call);
            let inner = vm.run_entry(ctx, EntryKind::Abst, |vm, ctx| {
                assert_eq!(ctx.exec_from(), ExecFrom::Call);
                vm.runtime.settle_compute_gas(ctx, 5).unwrap();
                Ok(Value::Nil)
            });
            assert!(inner.is_ok());
            vm.runtime.settle_compute_gas(ctx, 5).unwrap();
            Ok(Value::Nil)
        })
        .unwrap();
        assert_eq!(ctx.exec_from(), ExecFrom::Top);
    }
}
