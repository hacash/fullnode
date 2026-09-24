use std::any::Any;
use std::sync::Arc;

use base::{Context, ExecFrom, GasBuckets, IntentScope, with_exec_from};
use sys::Ret;

use crate::frame::CallFrame;
use crate::rt::{AbstCall, CodeType, EntryKind, FnObj, FrameBindings, ItrErr};
use crate::value::Value;

use super::{NativeVm, Runtime, VmHost, VmRequest};

impl NativeVm {
    pub fn new(height: u64, params: base::VmExecutionParams) -> Self {
        Self {
            runtime: Runtime::create(height, params),
            entries: Vec::new(),
            deadline: None,
        }
    }

    pub fn height(&self) -> u64 {
        self.runtime.cfg_height()
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
        // Match dev: every VM entry executes under `ExecFrom::Call` (observable by VM-hosted code).
        let result =
            with_exec_from(ctx, ExecFrom::Call, |ctx| run(self, ctx)).map_err(sys::Error::from);
        let entry = self.pop_entry()?;
        let settle = self.settle_entry_return_cost(ctx, entry);
        match (result, settle) {
            // Live `merge_xret_failure`: kind only upgrades. A revert mixed with a
            // settle fault becomes fault so AstSelect cannot capture the entry.
            (Err(exec_err), Err(settle_err)) => Err(exec_err.merge_upgrade(settle_err)),
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
            Some(detail) => sys::revertf!("{} return error {}", err_msg, detail),
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
    use crate::machine::test_ctx::{STUB_VM_PARAMS, TestCtx};
    use crate::rt::{ItrErr, ItrErrCode};
    use base::ExecFrom;

    fn run_arithmetic_source(source: &str) -> Ret<Value> {
        let codes = crate::lang::lang_to_bytecode(source)?;
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        vm.run_main_entry_value(&mut ctx, CodeType::Bytecode, codes.into())
            .map(|(_, value)| value)
    }

    #[test]
    fn regression_u64_mad_preserves_base_width_through_fin3() {
        for (source, expected) in [
            ("return u64_mad(1u64, 1u128, 1u128)", Value::U64(2)),
            ("return u64_mad(1u8, 1u16, 1u16)", Value::U8(2)),
            (
                "return u64_mad(18446744073709551615u128, 1u8, 1u8)",
                Value::U128(1u128 << 64),
            ),
        ] {
            assert_eq!(run_arithmetic_source(source).unwrap(), expected, "{source}");
        }
        for source in [
            "return u64_mad(200u8, 1u16, 100u16)",
            "return u64_mad(18446744073709551615u64, 1u128, 1u128)",
        ] {
            let error = run_arithmetic_source(source).unwrap_err();
            assert!(error.contains("Arithmetic"), "{source}: {error}");
        }
    }

    #[test]
    fn regression_literal_shifts_match_runtime_width_and_errors() {
        for bits in [8u32, 16, 32, 64, 128] {
            let max = u128::MAX >> (128 - bits);
            for shift in [0, 1, bits - 1, bits, bits + 1] {
                for op in ["<<", ">>"] {
                    let literal = format!("return {max}u{bits} {op} {shift}u{bits}");
                    let runtime = format!("var x = {max}u{bits} return x {op} {shift}u{bits}");
                    assert_eq!(
                        run_arithmetic_source(&literal).map_err(|e| e.to_string()),
                        run_arithmetic_source(&runtime).map_err(|e| e.to_string()),
                        "{literal}",
                    );
                }
            }
        }
        assert_eq!(
            run_arithmetic_source("return 255u8 << 1u16").unwrap(),
            Value::U16(510),
        );
        assert_eq!(
            run_arithmetic_source("var x = 255u8 return (256 >> 1) + x").unwrap(),
            Value::U16(383),
        );
    }

    /// Folding must keep the runtime operand width. Narrowing `300 - 50` to
    /// `u8` makes the following shift fail; a `u8` sum that does not fit still widens.
    #[test]
    fn regression_literal_arithmetic_keeps_operand_width() {
        let pairs = [
            (
                "return (300 - 50) << 1",
                "var a = 300\nvar b = 50\nvar s = 1\nreturn (a - b) << s",
            ),
            ("return 400 / 3", "var a = 400\nvar b = 3\nreturn a / b"),
            ("return 300 & 15", "var a = 300\nvar b = 15\nreturn a & b"),
            ("return 300 - 300", "var a = 300\nvar b = 300\nreturn a - b"),
        ];
        for (literal, runtime) in pairs {
            assert_eq!(
                run_arithmetic_source(literal).unwrap(),
                run_arithmetic_source(runtime).unwrap(),
                "{literal}",
            );
        }
        assert_eq!(
            run_arithmetic_source("return (300 - 50) << 1").unwrap(),
            Value::U16(500)
        );
        assert_eq!(
            run_arithmetic_source("return 400 / 3").unwrap(),
            Value::U16(133)
        );
        assert_eq!(
            run_arithmetic_source("return 255u8 + 1u8").unwrap(),
            Value::U16(256)
        );
        assert!(run_arithmetic_source("var x = 255u8\nreturn x + 1u8")
            .unwrap_err()
            .contains("Arithmetic"));
    }

    /// Dev entry semantics: every VM entry executes under `ExecFrom::Call`
    /// (as in dev's `with_exec_from(ctx, Call, ..)`), and the caller's exec_from is restored afterwards.
    #[test]
    fn run_entry_executes_under_exec_from_call_and_restores() {
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
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
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
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
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
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

    /// Live `merge_xret_failure`: execute revert + settle fault → whole entry is fault
    /// (settle primary). AstSelect must not capture this combination.
    #[test]
    fn run_entry_revert_plus_settle_fault_upgrades_to_fault() {
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        ctx.gas = 10;
        let err = vm
            .run_entry(&mut ctx, EntryKind::Main, |_vm, _ctx| {
                Err(ItrErr::new(ItrErrCode::ActCallRevert, "biz fail"))
            })
            .unwrap_err();
        assert!(
            err.is_fault(),
            "revert+settle-fault must upgrade to fault: {err}"
        );
        assert!(
            !err.is_revert(),
            "AstSelect must not capture this entry: {err}"
        );
        assert!(
            err.contains("out of gas") || err.contains("OutOfGas") || err.contains("gas"),
            "settle should be the primary error: {err}"
        );
        assert!(
            err.contains("biz fail"),
            "exec revert must remain in the message: {err}"
        );
    }

    #[test]
    fn run_entry_revert_alone_stays_revert() {
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        let err = vm
            .run_entry(&mut ctx, EntryKind::Main, |vm, ctx| {
                vm.runtime.settle_compute_gas(ctx, 5).unwrap();
                Err(ItrErr::new(ItrErrCode::ActCallRevert, "biz fail"))
            })
            .unwrap_err();
        assert!(
            err.is_revert(),
            "execute revert with successful settle stays revert: {err}"
        );
        assert!(err.contains("biz fail"), "{err}");
    }

    #[test]
    fn run_entry_fault_plus_settle_fault_stays_fault() {
        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        ctx.gas = 10;
        let err = vm
            .run_entry(&mut ctx, EntryKind::Main, |_vm, _ctx| {
                Err(ItrErr::new(ItrErrCode::ThrowAbort, "boom"))
            })
            .unwrap_err();
        assert!(err.is_fault(), "{err}");
        assert!(!err.is_revert(), "{err}");
        assert!(err.contains("boom"), "{err}");
    }
}
