use std::any::Any;
use std::sync::Arc;

use base::{with_exec_from, Context, ExecFrom, GasBuckets, IntentScope};
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
    use crate::machine::test_ctx::{TestCtx, STUB_VM_PARAMS};
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
    fn crypto_natives_execute_through_the_real_vm_entry() {
        let merkle_root = run_arithmetic_source("return merkle_root(0, 0, sha2(\"\"), \"\", 0)")
            .expect("merkle_root must execute through VM");
        assert!(matches!(merkle_root, Value::Bytes(ref bytes) if bytes.len() == 32));

        let multi_root =
            run_arithmetic_source("return merkle_multi_root(0, sha2(\"\"), \"\", \"\")")
                .expect("merkle_multi_root must execute through VM");
        assert_eq!(multi_root, merkle_root);

        let bitmap = run_arithmetic_source("return bitmap_find(\"\", 0, 0, 0, false)")
            .expect("bitmap_find must execute through VM");
        assert!(matches!(bitmap, Value::Tuple(_)));

        let p256 = run_arithmetic_source(
            "return p256_verify(sha2(\"\"), sha2(\"\") ++ sha2(\"\"), sha2(\"\") ++ sha2(\"\"))",
        )
        .expect("p256_verify must execute through VM");
        assert_eq!(p256, Value::Bool(false));

        let secp_error = run_arithmetic_source(
            "return secp256k1_recover(sha2(\"\"), sha2(\"\") ++ sha2(\"\"), 0)",
        )
        .expect_err("invalid secp256k1 signature must be rejected by VM");
        assert!(secp_error.to_string().contains("secp256k1_recover"));
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

    #[test]
    fn memory_init_executes_in_edit_only_and_prices_the_new_key() {
        use std::sync::Arc;

        use crate::frame::IntentScopeState;
        use crate::interpreter::execute_code_in_frame;
        use crate::rt::{
            Bytecode, CallExit, EffectMode, ExecCtx, FrameBindings, GasExtra, GasTable, ItrErrCode,
            NativeCtl,
        };
        use crate::value::Value;
        use field::Address;

        let mut raw = [0u8; 21];
        raw[0] = Address::VERSION_CONTRACT;
        raw[20] = 7;
        let who = Address::from(raw);

        fn run(
            vm: &mut NativeVm,
            ctx: &mut TestCtx,
            who: Address,
            effect: EffectMode,
            source: &str,
        ) -> Result<(CallExit, Value, i64), ItrErr> {
            let codes = crate::lang::lang_to_bytecode(source).expect(source);
            let cap = vm.runtime.warm.space_cap.clone();
            let mut ops = vm.runtime.stack_allocat();
            ops.reset(cap.stack_slot);
            let mut locals = vm.runtime.stack_allocat();
            locals.reset(cap.local_slot);
            let mut heap = vm.runtime.heap_allocat();
            heap.reset(cap.heap_segment);
            let mut bindings = FrameBindings::root(who, Arc::from([]));
            let mut intent = IntentScopeState::default();
            let mut pc = 0usize;
            let before = ctx.gas;
            let exit = execute_code_in_frame(
                &mut pc,
                &codes,
                ExecCtx::new(EntryKind::Main, effect, 1),
                &mut ops,
                &mut locals,
                &mut heap,
                &mut bindings,
                &mut intent,
                &who,
                &who,
                vm,
                ctx,
            );
            let spent = before - ctx.gas;
            let top = ops.pop().unwrap_or(Value::Nil);
            vm.runtime.stack_reclaim(ops);
            vm.runtime.stack_reclaim(locals);
            vm.runtime.heap_reclaim(heap);
            exit.map(|e| (e, top, spent))
        }

        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        let g = GasExtra::new(0, &STUB_VM_PARAMS);

        let (exit, v, insert_spent) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "return memory_init(0)",
        )
        .unwrap();
        assert!(matches!(exit, CallExit::Return));
        assert_eq!(v, Value::Bool(true));

        let (exit, v, hit_spent) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "return memory_init(0)",
        )
        .unwrap();
        assert!(matches!(exit, CallExit::Return));
        assert_eq!(v, Value::Bool(false));
        assert_eq!(
            insert_spent - hit_spent,
            g.memory_key_cost + g.stack_write(Value::Bool(true).val_size())
        );

        let codes = crate::lang::lang_to_bytecode("return memory_init(0)").unwrap();
        let table = GasTable::new(1);
        let mut compute = 0i64;
        let mut i = 0usize;
        while i < codes.len() {
            let op = codes[i];
            let meta = Bytecode::try_from_u8(op).unwrap().metadata();
            compute += table.gas(op);
            i += 1 + meta.param as usize;
        }
        let klen = Value::U8(0)
            .extract_key_bytes_with_error_code(ItrErrCode::MemoryError)
            .unwrap()
            .len();
        let hit_resource = NativeCtl::memory_init.gas_of()
            + g.stack_write(klen)
            + g.nt_bytes(Value::Bool(false).val_size());
        assert_eq!(hit_spent, compute + hit_resource);

        let (_, got, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "return memory_get(0)",
        )
        .unwrap();
        assert_eq!(got, Value::Bool(true));

        let (_, v, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "memory_put(2, false)\nreturn memory_init(2)",
        )
        .unwrap();
        assert_eq!(v, Value::Bool(false));
        let (_, got, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "return memory_get(2)",
        )
        .unwrap();
        assert_eq!(got, Value::Bool(false));

        let (_, v, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Edit,
            "memory_put(0, nil)\nreturn memory_init(0)",
        )
        .unwrap();
        assert_eq!(v, Value::Bool(true));

        let err = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::Pure,
            "return memory_init(9)",
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::InstDisabled);
        let err = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::View,
            "return memory_init(8)",
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::InstDisabled);
        let (_, got, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::View,
            "return memory_get(9)",
        )
        .unwrap();
        assert!(got.is_nil());
        let (_, got, _) = run(
            &mut vm,
            &mut ctx,
            who,
            EffectMode::View,
            "return memory_get(8)",
        )
        .unwrap();
        assert!(got.is_nil());
    }

    #[test]
    fn require_evaluates_error_first_and_rejects_non_scalar_error_values() {
        // Same lightweight harness as the memory_init test: a Main frame over
        // lang_to_bytecode output — no deployed contract needed. Pure mode also
        // pins that `require` performs no host effects.
        use std::sync::Arc;

        use crate::frame::IntentScopeState;
        use crate::interpreter::execute_code_in_frame;
        use crate::rt::{Bytecode, CallExit, EffectMode, ExecCtx, FrameBindings, GasTable};
        use crate::value::Value;
        use field::Address;

        let mut raw = [0u8; 21];
        raw[0] = Address::VERSION_CONTRACT;
        raw[20] = 9;
        let who = Address::from(raw);

        fn run(
            vm: &mut NativeVm,
            ctx: &mut TestCtx,
            who: Address,
            source: &str,
        ) -> Result<(CallExit, Value, i64), ItrErr> {
            let codes = crate::lang::lang_to_bytecode(source).expect(source);
            let cap = vm.runtime.warm.space_cap.clone();
            let mut ops = vm.runtime.stack_allocat();
            ops.reset(cap.stack_slot);
            let mut locals = vm.runtime.stack_allocat();
            locals.reset(cap.local_slot);
            let mut heap = vm.runtime.heap_allocat();
            heap.reset(cap.heap_segment);
            let mut bindings = FrameBindings::root(who, Arc::from([]));
            let mut intent = IntentScopeState::default();
            let mut pc = 0usize;
            let before = ctx.gas;
            let exit = execute_code_in_frame(
                &mut pc,
                &codes,
                ExecCtx::new(EntryKind::Main, EffectMode::Pure, 1),
                &mut ops,
                &mut locals,
                &mut heap,
                &mut bindings,
                &mut intent,
                &who,
                &who,
                vm,
                ctx,
            );
            let spent = before - ctx.gas;
            let top = ops.pop().unwrap_or(Value::Nil);
            vm.runtime.stack_reclaim(ops);
            vm.runtime.stack_reclaim(locals);
            vm.runtime.heap_reclaim(heap);
            exit.map(|e| (e, top, spent))
        }

        let mut vm = NativeVm::new(1, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();

        // True condition: the statement is a no-op and execution continues.
        let (exit, _, spent) = run(
            &mut vm,
            &mut ctx,
            who,
            "require \"denied\", 1 == 1\nreturn 7",
        )
        .unwrap();
        assert!(matches!(exit, CallExit::Return));
        assert!(spent > 0);

        // False condition: fatal user abort carrying a serializable scalar.
        let err = run(&mut vm, &mut ctx, who, "require \"denied\", 1 == 2").unwrap_err();
        assert_eq!(err.0, ItrErrCode::UserAbort, "{err}");
        assert!(err.1.contains("denied"), "{err}");
        let external: sys::Error = err.into();
        assert!(external.is_fault(), "{external}");
        for source in ["require 0, false", "require true, false"] {
            let err = run(&mut vm, &mut ctx, who, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::UserAbort, "{source}: {err}");
            assert!(err.1.starts_with("require failed: "), "{source}: {err}");
        }

        for source in [
            "require [1, 2], false",
            "require [1, 2], true",
            "require nil, false",
            "require nil, true",
            "throw [1, 2]",
            "throw nil",
        ] {
            let err = run(&mut vm, &mut ctx, who, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CastBeValueFail, "{source}: {err}");
        }
        let (exit, value, _) = run(&mut vm, &mut ctx, who, "throw 0").unwrap();
        assert!(matches!(exit, CallExit::Throw));
        assert_eq!(value, Value::U8(0));

        // The error expression is evaluated even when the condition is true.
        let err = run(&mut vm, &mut ctx, who, "require (1 / 0), true").unwrap_err();
        assert_ne!(err.0, ItrErrCode::UserAbort, "{err}");

        let table = GasTable::new(1);
        let mut gas_ctx = TestCtx::new();
        let (_, _, baseline) = run(&mut vm, &mut gas_ctx, who, "return 7").unwrap();
        let (_, _, with_req) = run(&mut vm, &mut gas_ctx, who, "require 0, true\nreturn 7").unwrap();
        assert_eq!(
            with_req - baseline,
            table.gas(Bytecode::P0 as u8)
                + table.gas(Bytecode::PTRUE as u8)
                + table.gas(Bytecode::REQ as u8),
        );
        let before_failure = gas_ctx.gas;
        let err = run(&mut vm, &mut gas_ctx, who, "require 0, false").unwrap_err();
        assert_eq!(err.0, ItrErrCode::UserAbort);
        assert_eq!(
            before_failure - gas_ctx.gas,
            table.gas(Bytecode::P0 as u8)
                + table.gas(Bytecode::PFALSE as u8)
                + table.gas(Bytecode::REQ as u8),
        );

        // Pure compute: identical runs spend identical gas (no native resources).
        let mut ctx2 = TestCtx::new();
        let (_, _, spent2) = run(
            &mut vm,
            &mut ctx2,
            who,
            "require \"denied\", 1 == 1\nreturn 7",
        )
        .unwrap();
        assert_eq!(spent, spent2);
    }

    fn verified(source: &str) -> Vec<u8> {
        let codes = crate::lang::lang_to_bytecode(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        crate::rt::verify_bytecodes(&codes).unwrap_or_else(|e| panic!("{source}\nverify: {e}"));
        codes
    }

    fn jmp_count(codes: &[u8]) -> usize {
        let mut i = 0;
        let mut n = 0;
        while i < codes.len() {
            let inst = crate::rt::Bytecode::try_from_u8(codes[i]).unwrap();
            if inst == crate::rt::Bytecode::JMPSL {
                n += 1;
            }
            let mut step = 1 + inst.metadata().param as usize;
            if inst == crate::rt::Bytecode::PBUF {
                step += codes[i + 1] as usize;
            } else if inst == crate::rt::Bytecode::PBUFL {
                let len = u16::from_be_bytes([codes[i + 1], codes[i + 2]]) as usize;
                step += len;
            }
            i += step;
        }
        n
    }

    #[test]
    fn else_exit_drops_dead_jump_and_keeps_both_results() {
        let both_true = "if true { return 1 } else { return 2 }";
        let both_false = "if false { return 1 } else { return 2 }";
        assert_eq!(jmp_count(&verified(both_true)), 0);
        assert_eq!(run_arithmetic_source(both_true).unwrap(), Value::U8(1));
        assert_eq!(run_arithmetic_source(both_false).unwrap(), Value::U8(2));

        let else_only = "if true { } else { return 7 }\nreturn 5";
        assert_eq!(jmp_count(&verified(else_only)), 0);
        assert_eq!(run_arithmetic_source(else_only).unwrap(), Value::U8(5));

        let then_only = "if false { return 4 } else { }\nreturn 6";
        assert_eq!(jmp_count(&verified(then_only)), 1);
        assert_eq!(run_arithmetic_source(then_only).unwrap(), Value::U8(6));

        let nested = "if false { return 1 } else { if true { return 2 } else { return 3 } }";
        assert_eq!(jmp_count(&verified(nested)), 0);
        assert_eq!(run_arithmetic_source(nested).unwrap(), Value::U8(2));

        let codecall = "lib C = 1\nif true { return 1 } else { codecall C.probe }";
        assert_eq!(jmp_count(&verified(codecall)), 0);
        assert_eq!(run_arithmetic_source(codecall).unwrap(), Value::U8(1));
    }

    #[test]
    fn loop_if_relocates_break_continue_when_else_returns() {
        let source = r#"
            var s = 0
            var i = 0
            while i < 10 {
                i = i + 1
                if i < 4 {
                    if i > 1 {
                        if i < 3 { continue } else { s = s + i }
                    } else {
                        s = s + i
                    }
                } else {
                    return s
                }
            }
            return 99
        "#;
        verified(source);
        assert_eq!(run_arithmetic_source(source).unwrap(), Value::U8(4));
    }

    /// Else returns, so the skip-then `JMPSL` is omitted. `break` sits in the
    /// then branch; a `then_br_start` that still counts that jump lands on the
    /// loop-back `JMPSL` and the loop exits via `return 9` instead of `3`.
    #[test]
    fn loop_if_break_in_then_when_else_returns() {
        let source = r#"
            var n = 0
            while n < 10 {
                n = n + 1
                if n < 4 {
                    if n > 2 { break } else { }
                } else {
                    return 9
                }
            }
            return n
        "#;
        verified(source);
        assert_eq!(run_arithmetic_source(source).unwrap(), Value::U8(3));
    }

    /********** ITEMSET / BUFSET / ITEMCAT (docs.loc/fitsh-rv64-opcode.md task 4) **********/

    /// Same lightweight Main-frame harness as the memory_init test, with the
    /// machine height as a parameter (upgrade-window tests below).
    fn fitsh_opcode_run(
        height: u64,
        source: &str,
    ) -> Result<(crate::rt::CallExit, Value, i64), ItrErr> {
        use std::sync::Arc;

        use crate::frame::IntentScopeState;
        use crate::interpreter::execute_code_in_frame;
        use crate::rt::{EffectMode, ExecCtx, FrameBindings};
        use field::Address;

        let mut raw = [0u8; 21];
        raw[0] = Address::VERSION_CONTRACT;
        raw[20] = 3;
        let who = Address::from(raw);

        let mut vm = NativeVm::new(height, STUB_VM_PARAMS);
        let mut ctx = TestCtx::new();
        let codes = crate::lang::lang_to_bytecode(source).expect(source);
        let cap = vm.runtime.warm.space_cap.clone();
        let mut ops = vm.runtime.stack_allocat();
        ops.reset(cap.stack_slot);
        let mut locals = vm.runtime.stack_allocat();
        locals.reset(cap.local_slot);
        let mut heap = vm.runtime.heap_allocat();
        heap.reset(cap.heap_segment);
        let mut bindings = FrameBindings::root(who, Arc::from([]));
        let mut intent = IntentScopeState::default();
        let mut pc = 0usize;
        let before = ctx.gas;
        let exit = execute_code_in_frame(
            &mut pc,
            &codes,
            ExecCtx::new(EntryKind::Main, EffectMode::Edit, 1),
            &mut ops,
            &mut locals,
            &mut heap,
            &mut bindings,
            &mut intent,
            &who,
            &who,
            &mut vm,
            &mut ctx,
        );
        let spent = before - ctx.gas;
        let top = ops.pop().unwrap_or(Value::Nil);
        vm.runtime.stack_reclaim(ops);
        vm.runtime.stack_reclaim(locals);
        vm.runtime.heap_reclaim(heap);
        exit.map(|e| (e, top, spent))
    }

    #[test]
    fn itemset_error_order_and_writes() {
        // Check order: 83 -> 76 -> 80 -> 72 -> 85.
        // Bytes receiver: CompoOpNotMatch 83, and it fires before the tuple
        // value's CastBeValueFail 76 (receiver shape is checked first).
        for source in [
            "var b = 0x0102\nb[0] = 9",
            "var b = 0x0102\nb[0] = tuple(1, 2)",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CompoOpNotMatch, "{source}: {err}");
        }
        // check_scalar 76 fires before the map's CompoOpInvalid 80: a map
        // receiver with a tuple / sub-list value is a value-shape failure.
        for source in [
            "var m = map {1: 2}\nm[1] = tuple(1, 2)",
            "var m = map {1: 2}\nm[1] = [3]",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CastBeValueFail, "{source}: {err}");
        }
        // Map + scalar value: CompoOpInvalid 80 (map writes stay `insert`).
        let err = fitsh_opcode_run(1, "var m = map {1: 2}\nm[1] = 3").unwrap_err();
        assert_eq!(err.0, ItrErrCode::CompoOpInvalid, "{err}");
        // check_scalar 76 (tuple / sub-list value) fires before the index's 72.
        for source in [
            "var l = [1, 2]\nl[nil] = tuple(1, 2)",
            "var l = [1, 2]\nl[nil] = [3]",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CastBeValueFail, "{source}: {err}");
        }
        // Index CastParamFail 72 (Nil, or >= 2^32) fires before the range check.
        for source in [
            "var l = [1, 2]\nl[nil] = 9",
            "var l = [1, 2]\nl[4294967296u64] = 9",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CastParamFail, "{source}: {err}");
        }
        // idx >= len (including idx == len) is CompoNoFindItem 85: no growth.
        for source in [
            "var l = [1, 2]\nl[2] = 9",
            "var l = [1, 2]\nl[7] = 9",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CompoNoFindItem, "{source}: {err}");
        }
        // The write replaces one item: neighbours and the length do not move.
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2, 3]\nl[1] = 9\nreturn l[0]").unwrap();
        assert_eq!(v, Value::U8(1));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2, 3]\nl[1] = 9\nreturn l[2]").unwrap();
        assert_eq!(v, Value::U8(3));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2, 3]\nl[1] = 9\nreturn length(l)").unwrap();
        assert_eq!(v, Value::U32(3));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2, 3]\nl[1] = 9\nreturn l[1]").unwrap();
        assert_eq!(v, Value::U8(9));
        // Nested write l[i][j] = v: a compo cannot hold a list (check_scalar),
        // so the inner receiver rides a tuple; when the inner get yields a
        // non-list (bytes here) the receiver rule gives 83.
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var t = tuple([1, 2], 0)\nt[0][1] = 9\nreturn t[0][1]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(9));
        let err = fitsh_opcode_run(1, "var l = [0x0102]\nl[0][1] = 9").unwrap_err();
        assert_eq!(err.0, ItrErrCode::CompoOpNotMatch, "inner bytes receiver: {err}");
        // Nil / bool / address values are writable scalars.
        let (exit, v, _) = fitsh_opcode_run(1, "var l = [1]\nl[0] = nil\nreturn l[0]").unwrap();
        assert!(matches!(exit, crate::rt::CallExit::Return));
        assert!(v.is_nil());
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1]\nl[0] = true\nreturn l[0]").unwrap();
        assert_eq!(v, Value::Bool(true));
        use field::Address;
        let mut raw = [0u8; 21];
        raw[0] = Address::VERSION_CONTRACT;
        raw[20] = 5;
        let addr = Address::from(raw);
        let (_, v, _) = fitsh_opcode_run(
            1,
            &format!("var l = [1]\nl[0] = {}\nreturn l[0]", addr.to_readable()),
        )
        .unwrap();
        assert_eq!(v, Value::Address(addr));
    }

    #[test]
    fn bufset_writes_and_error_order() {
        // BUFSET mutates the receiver's own Vec on the stack: the expression
        // result is the written buffer (a local's copy keeps the old bytes,
        // covered below). u8 writes exactly one byte.
        let (_, v, _) = fitsh_opcode_run(1, "return buf_set(0x01020304, 1, 0xff)").unwrap();
        assert_eq!(v, Value::Bytes(vec![0x01, 0xff, 0x03, 0x04]));
        // u64 writes its 8 canonical big-endian bytes.
        let (_, v, _) = fitsh_opcode_run(
            1,
            &format!(
                "return buf_set(0x{}, 0, {}u64)",
                "00".repeat(16),
                0x0706050403020100u64
            ),
        )
        .unwrap();
        assert_eq!(
            v,
            Value::Bytes(vec![0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        // bool writes its single canonical byte.
        let (_, v, _) = fitsh_opcode_run(1, "return buf_set(0x0000, 1, true)").unwrap();
        assert_eq!(v, Value::Bytes(vec![0x00, 0x01]));
        // Nil source is BytesHandle 91 and is checked before the start's 72.
        for source in [
            "var b = 0x0102\nbuf_set(b, nil, nil)",
            "var b = 0x0102\nbuf_set(b, 65536u64, nil)",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::BytesHandle, "{source}: {err}");
        }
        // Start past u16 is CastParamFail 72; a non-bytes receiver is 91.
        let err = fitsh_opcode_run(1, "var b = 0x0102\nbuf_set(b, 65536u64, 0xff)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::CastParamFail, "{err}");
        let err = fitsh_opcode_run(1, "var l = [1]\nbuf_set(l, 0, 0xff)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::BytesHandle, "{err}");
        // start + source length == buffer length is legal; past it is StackError 42.
        let (_, v, _) = fitsh_opcode_run(1, "return buf_set(0x01020304, 2, 0xaabb)").unwrap();
        assert_eq!(v, Value::Bytes(vec![0x01, 0x02, 0xaa, 0xbb]));
        let err = fitsh_opcode_run(1, "return buf_set(0x01020304, 3, 0xaabb)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::StackError, "{err}");
        // The length never changes.
        let (_, v, _) =
            fitsh_opcode_run(1, "var b = buf_set(0x01020304, 1, 0xff)\nreturn size(b)").unwrap();
        assert_eq!(v, Value::U16(4));
        // Other slots keep their old copy: the interpreter's byte string is
        // the one Vec, and `var c = <bytes>` holds an independent copy.
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var c = 0x01020304\nvar b = buf_set(c, 0, 0xffff)\nreturn c",
        )
        .unwrap();
        assert_eq!(v, Value::Bytes(vec![0x01, 0x02, 0x03, 0x04]));
    }

    #[test]
    fn itemcat_concat_and_errors() {
        // Non-list receiver is 83; a map is 80.
        let err = fitsh_opcode_run(1, "var b = 0x0102\nreturn item_concat(b)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::CompoOpNotMatch, "{err}");
        let err = fitsh_opcode_run(1, "var m = map {1: 2}\nreturn item_concat(m)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::CompoOpInvalid, "{err}");
        // u8 and bytes mix equals each element's extract_bytes in order.
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, \"ab\"]\nreturn item_concat(l)").unwrap();
        let mut want = Value::U8(1).extract_bytes().unwrap();
        want.extend_from_slice(b"ab");
        assert_eq!(v, Value::Bytes(want));
        // u64 contributes 8 bytes; bool contributes its 1 canonical byte.
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1u64]\nreturn item_concat(l)").unwrap();
        assert_eq!(
            v,
            Value::Bytes(Value::U64(1).extract_bytes().unwrap())
        );
        let (_, v, _) = fitsh_opcode_run(1, "var l = [true, false]\nreturn item_concat(l)").unwrap();
        assert_eq!(v, Value::Bytes(vec![1, 0]));
        // A Nil element is BytesHandle 91.
        let err = fitsh_opcode_run(1, "var l = [1, nil]\nreturn item_concat(l)").unwrap_err();
        assert_eq!(err.0, ItrErrCode::BytesHandle, "{err}");
        // An empty list yields empty bytes.
        let (_, v, _) = fitsh_opcode_run(1, "var l = []\nreturn item_concat(l)").unwrap();
        assert_eq!(v, Value::Bytes(vec![]));
        // Exactly value_size (1280) bytes in one element is the legal boundary.
        let source = format!("var l = [0x{}]\nreturn item_concat(l)", "11".repeat(1280));
        let (_, v, _) = fitsh_opcode_run(1, &source).unwrap();
        match v {
            Value::Bytes(bytes) => {
                assert_eq!(bytes.len(), 1280);
                assert!(bytes.iter().all(|b| *b == 0x11));
            }
            other => panic!("expected 1280 bytes, got {other:?}"),
        }
        // Past value_size (1280) is OutOfValueSize 39.
        let source = format!(
            "var l = [0x{}, 0x01]\nreturn item_concat(l)",
            "11".repeat(1280)
        );
        let err = fitsh_opcode_run(1, &source).unwrap_err();
        assert_eq!(err.0, ItrErrCode::OutOfValueSize, "{err}");
    }

    /// The three opcodes run below the window and from its end; inside
    /// [800_000, 802_000) every dispatch is refused with InstDisabled (an
    /// upgrade-window refusal, not an opcode semantic).
    #[test]
    fn fitsh_opcodes_refuse_only_inside_the_upgrade_window() {
        for source in [
            "var l = [1, 2]\nl[1] = 9\nreturn l[1]",
            "var b = 0x01020304\nbuf_set(b, 1, 0xff)\nreturn b",
            "var l = [1, \"ab\"]\nreturn item_concat(l)",
        ] {
            for height in [1u64, 799_999, 802_000, 9_999_999] {
                let ran: Result<(crate::rt::CallExit, Value, i64), ItrErr> =
                    fitsh_opcode_run(height, source);
                assert!(ran.is_ok(), "height {height} {source}: {:?}", ran.err());
            }
            for height in [800_000u64, 800_001, 801_111, 801_999] {
                let err = fitsh_opcode_run(height, source).unwrap_err();
                assert_eq!(
                    err.0,
                    ItrErrCode::InstDisabled,
                    "height {height} {source}: {err}"
                );
            }
        }
    }

    /// ITEMSET spends exactly what INSERT spends on the same list:
    /// compo_items_edit(len) + compo_bytes(value), no key byte fee.
    #[test]
    fn itemset_gas_matches_insert_on_a_list() {
        let set = "var l = [1, 2, 3]\nitem_set(l, 1, 9)\nreturn l[1]";
        let ins = "var l = [1, 2, 3]\ninsert(l, 1, 9)\nreturn l[1]";
        let (_, _, set_spent) = fitsh_opcode_run(1, set).unwrap();
        let (_, _, ins_spent) = fitsh_opcode_run(1, ins).unwrap();
        assert_eq!(
            set_spent, ins_spent,
            "ITEMSET must bill like INSERT on a list"
        );
    }

    /// ITEMCAT charges the list-read component before the concat. Compare each
    /// `return item_concat(l)` against `return l` on the same list: the only
    /// difference is ITEMCAT, so the delta is exactly
    /// `gas(ITEMCAT) + compo_items_read(len) + stack_op(result_len)`. The two
    /// lists produce the same 5 output bytes but hold 1 vs 5 items.
    #[test]
    fn itemcat_gas_charges_the_list_read_component() {
        use crate::rt::{Bytecode, GasExtra, GasTable};

        let base = GasTable::new(1);
        let extra = GasExtra::new(1, &STUB_VM_PARAMS);
        let spent = |source: &str| fitsh_opcode_run(1, source).unwrap().2;

        let one = "var l = [0x0102030405]\n";
        let five = "var l = [1u8, 2u8, 3u8, 4u8, 5u8]\n";
        let one_delta = spent(&format!("{one}return item_concat(l)")) - spent(&format!("{one}return l"));
        let five_delta =
            spent(&format!("{five}return item_concat(l)")) - spent(&format!("{five}return l"));

        assert_eq!(extra.compo_items_read(5), 2, "guard: 5 items are two read units");
        assert_eq!(
            one_delta,
            base.gas(Bytecode::ITEMCAT as u8)
                + extra.compo_items_read(1)
                + extra.stack_op(5),
            "one-element list must bill base + read(1) + stack_op(5)"
        );
        assert_eq!(
            five_delta,
            base.gas(Bytecode::ITEMCAT as u8)
                + extra.compo_items_read(5)
                + extra.stack_op(5),
            "five-element list must bill base + read(5) + stack_op(5)"
        );
        assert!(
            five_delta > one_delta,
            "same 5 output bytes, but the longer list must cost more read gas"
        );
    }

    /// F2: an ITEMSET / compound item assignment used as a ternary (CHOOSE)
    /// condition is parenthesized by the decompiler and recompiles exactly.
    /// The assignment returns the container, so both forms fail the CHOOSE
    /// condition cast with CastFail 71.
    #[test]
    fn itemset_and_compound_condition_decompile_with_parentheses() {
        for (source, condition) in [
            (
                "var l = [1, 2]\nreturn item_set(l, 0, 9) ? 10 : 20",
                "(l[0] = 9) ?",
            ),
            (
                "var l = [1, 2]\nreturn (l[0] += 1) ? 10 : 20",
                "(l[0] += 1) ?",
            ),
            (
                "var l = [1, 2]\nreturn (l[0] += 1) + 2",
                "(l[0] += 1) +",
            ),
        ] {
            let codes = crate::lang::lang_to_bytecode(source).expect(source);
            let (ircode, smap) =
                crate::lang::lang_to_ircode_with_sourcemap(source).expect(source);
            let printed = crate::lang::format_ircode_to_lang(&ircode, Some(&smap)).expect(source);
            assert!(
                printed.contains(condition),
                "{source} => decompiled condition must be parenthesized: {printed}"
            );
            let recompiled =
                crate::lang::lang_to_bytecode(&printed).unwrap_or_else(|e| panic!("{printed}: {e}"));
            assert_eq!(
                recompiled, codes,
                "{source} => decompile/recompile changed the bytecode:\n{printed}"
            );
            let original_err = fitsh_opcode_run(1, source).unwrap_err();
            let recompiled_err = fitsh_opcode_run(1, &printed).unwrap_err();
            assert_eq!(
                original_err.0,
                ItrErrCode::CastFail,
                "{source}: {original_err}"
            );
            assert_eq!(
                original_err.0, recompiled_err.0,
                "{source} => original and recompiled must fail alike: {recompiled_err}"
            );
        }
    }

    /// F6: compound item assignment is stack-only and RHS-first. The RHS may
    /// rebind the receiver or mutate the element; receiver and index are read
    /// only after the RHS has run.
    #[test]
    fn compound_item_assignment_is_rhs_first() {
        // The RHS rebinds `l` to a fresh list: the old list is untouched...
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [1]\nvar old = l\nl[0] += { l = [2]; 5 }\nreturn old[0]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(1));
        // ...and the fresh list is the receiver that gets written.
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [1]\nvar old = l\nl[0] += { l = [2]; 5 }\nreturn l[0]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(7));
        // The RHS mutates the element: the element read happens after it.
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [1]\nl[0] += { l[0] = 10; 5 }\nreturn l[0]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(15));
        // The index is evaluated after the RHS: the RHS sets i=0, then the
        // index expression sets i=1.
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [10, 20]\nvar i = 0\nl[{ i = 1; 0 }] += { i = 0; 5 }\nreturn i",
        )
        .unwrap();
        assert_eq!(v, Value::U8(1));
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [10, 20]\nvar i = 0\nl[{ i = 1; 0 }] += { i = 0; 5 }\nreturn l[0]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(15));
    }

    /// F8: above slot 63 the local compound fallback is still RHS-first, uses
    /// raw GET/SWAP/op/PUT (no XOP), decompiles to `x += ...`, and recompiles
    /// byte-for-byte.
    #[test]
    fn compound_local_assignment_above_slot_63_is_rhs_first() {
        let prefix: String = (0..64).map(|i| format!("var v{i} = 0\n")).collect();
        let source = format!("{prefix}var x = 1\nx += {{ x = 2; 5 }}\nreturn x");
        let codes = crate::lang::lang_to_bytecode(&source).unwrap();
        assert!(
            codes.windows(6).any(|w| w
                == [
                    crate::rt::Bytecode::GET as u8,
                    64,
                    crate::rt::Bytecode::SWAP as u8,
                    crate::rt::Bytecode::ADD as u8,
                    crate::rt::Bytecode::PUT as u8,
                    64,
                ]),
            "slot 64 must use raw GET/SWAP/ADD/PUT: {codes:02x?}"
        );
        let (_, v, _) = fitsh_opcode_run(1, &source).unwrap();
        assert_eq!(v, Value::U8(7));

        // Same result below slot 64 through XOP.
        let low = "var x = 1\nx += { x = 2; 5 }\nreturn x";
        let (_, v, _) = fitsh_opcode_run(1, low).unwrap();
        assert_eq!(v, Value::U8(7));

        let (ircode, smap) = crate::lang::lang_to_ircode_with_sourcemap(&source).unwrap();
        let printed = crate::lang::format_ircode_to_lang(&ircode, Some(&smap)).unwrap();
        assert!(
            printed.contains("x += "),
            "decompile lost the high-slot compound assignment: {printed}"
        );
        let recompiled = crate::lang::lang_to_bytecode(&printed)
            .unwrap_or_else(|e| panic!("{printed}\n{e}"));
        assert_eq!(
            recompiled, codes,
            "high-slot compound assignment must roundtrip:\n{printed}"
        );
    }

    /// F6 no-hidden-slot budget: 100 `l[0] += 1` statements compile (no
    /// per-statement spill slots) and all increments take effect.
    #[test]
    fn compound_item_assignment_has_no_hidden_slot_budget() {
        let mut source = String::from("var l = [0]\n");
        for _ in 0..100 {
            source.push_str("l[0] += 1\n");
        }
        source.push_str("return l[0]");
        let (_, v, _) = fitsh_opcode_run(1, &source).unwrap();
        assert_eq!(v, Value::U8(100));
    }

    /// F10: ITEMSET / INSERT / APPEND return the receiver container, so a
    /// second alias observes later writes to it.
    #[test]
    fn insert_append_and_item_set_return_the_receiver_container() {
        for source in [
            "var l = [1, 2]\nvar x = item_set(l, 0, 9)\nx[1] = 7\nreturn l[1]",
            "var l = [1, 2]\nvar x = insert(l, 0, 9)\nx[1] = 7\nreturn l[1]",
            "var l = [1, 2]\nvar x = append(l, 3)\nx[1] = 7\nreturn l[1]",
        ] {
            let (_, v, _) = fitsh_opcode_run(1, source).unwrap();
            assert_eq!(v, Value::U8(7), "{source}");
        }
    }

    /// F9: `item_get(...)` is an ITEMGET target, so plain and compound
    /// assignment through the call form work like `l[i]`.
    #[test]
    fn item_get_call_form_is_an_assignment_target() {
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2]\nitem_get(l, 0) = 9\nreturn l[0]")
            .unwrap();
        assert_eq!(v, Value::U8(9));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [1, 2]\nitem_get(l, 0) += 1\nreturn l[0]")
            .unwrap();
        assert_eq!(v, Value::U8(2));
    }

    /// F7: arithmetic follows the interpreter's existing local ADD/XOP rule.
    /// Mixed uint widths widen to the wider width and succeed; non-uint values
    /// fail CastFail 71; same-width overflow is Arithmetic 90.
    #[test]
    fn compound_arithmetic_matches_local_width_and_cast_rules() {
        let (_, v, _) =
            fitsh_opcode_run(1, "var l = [1u8]\nl[0] += 2u16\nreturn l[0]").unwrap();
        assert_eq!(v, Value::U16(3));
        let (_, v, _) = fitsh_opcode_run(1, "var x = 1u8\nx += 2u16\nreturn x").unwrap();
        assert_eq!(v, Value::U16(3));
        for source in [
            "var l = [1u8]\nl[0] += true\nreturn l[0]",
            "var x = 1u8\nx += true\nreturn x",
        ] {
            let err = fitsh_opcode_run(1, source).unwrap_err();
            assert_eq!(err.0, ItrErrCode::CastFail, "{source}: {err}");
        }
        let err = fitsh_opcode_run(1, "var l = [250u8]\nl[0] += 10u8").unwrap_err();
        assert_eq!(err.0, ItrErrCode::Arithmetic, "{err}");
        let err = fitsh_opcode_run(1, "var x = 250u8\nx += 10u8\nreturn x").unwrap_err();
        assert_eq!(err.0, ItrErrCode::Arithmetic, "{err}");
    }

    /// A compound item assignment in expression context returns the same
    /// container, so writes through the result are visible on the receiver.
    #[test]
    fn compound_item_assignment_expression_returns_the_container() {
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var l = [10, 20]\nvar x = (l[0] += 5)\nx[1] = 9\nreturn l[1]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(9));
    }

    /// `+= -= *= /=` on an ITEMGET target reuse the arithmetic opcodes the
    /// locals path uses, so results (and failures) match `XOP` semantics.
    #[test]
    fn compound_item_assignment_matches_local_arithmetic() {
        let (_, v, _) = fitsh_opcode_run(1, "var l = [10, 20]\nvar i = 1\nl[i] += 5\nreturn l[1]").unwrap();
        assert_eq!(v, Value::U8(25));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [10, 20]\nl[1] -= 5\nreturn l[1]").unwrap();
        assert_eq!(v, Value::U8(15));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [10, 20]\nl[1] *= 2\nreturn l[1]").unwrap();
        assert_eq!(v, Value::U8(40));
        let (_, v, _) = fitsh_opcode_run(1, "var l = [10, 20]\nl[1] /= 2\nreturn l[1]").unwrap();
        assert_eq!(v, Value::U8(10));
        // Nested target (the inner list rides a tuple: compos cannot nest).
        let (_, v, _) = fitsh_opcode_run(
            1,
            "var t = tuple([1, 2], 0)\nt[0][1] += 7\nreturn t[0][1]",
        )
        .unwrap();
        assert_eq!(v, Value::U8(9));
        // Same-width overflow is the interpreter's Arithmetic failure; the
        // parser does not pre-restrict operand types.
        let err = fitsh_opcode_run(1, "var l = [250u8]\nl[0] += 10u8").unwrap_err();
        assert_eq!(err.0, ItrErrCode::Arithmetic, "{err}");
        // The stack-only form survives a decompile roundtrip byte-for-byte.
        let source = "var l = [10, 20]\nvar i = 1\nl[i] += 5\nreturn l[1]";
        let codes = crate::lang::lang_to_bytecode(source).unwrap();
        let ircode = crate::lang::lang_to_ircode(source).unwrap();
        let printed = crate::lang::format_ircode_to_lang(&ircode, None).unwrap();
        let recompiled = crate::lang::lang_to_bytecode(&printed).unwrap();
        assert_eq!(
            recompiled, codes,
            "compound item assignment must survive a decompile roundtrip:\n{printed}"
        );
    }
}
