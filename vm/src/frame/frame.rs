use super::*;

#[derive(Debug, Default)]
pub struct CallFrame {
    pub(crate) frames: Vec<Frame>,
}

impl CallFrame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn pop(&mut self) -> Option<Frame> {
        self.frames.pop()
    }

    pub fn push(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    pub fn increase<M: VmMachine + ?Sized>(&mut self, machine: &mut M) -> VmrtRes<Frame> {
        Ok(match self.frames.last() {
            Some(f) => f.next(machine),
            None => Frame::new(machine),
        })
    }

    pub fn reclaim<M: VmMachine + ?Sized>(mut self, machine: &mut M) {
        while let Some(frame) = self.pop() {
            frame.reclaim(machine)
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct IntentScopeState {
    base: IntentScope,
    stack: Vec<BoundIntentId>,
}

impl IntentScopeState {
    pub fn current_scope(&self) -> IntentScope {
        if self.stack.is_empty() {
            self.base
        } else {
            self.stack.last().cloned()
        }
    }

    pub fn current_bound_intent_id(&self) -> BoundIntentId {
        self.current_scope().flatten()
    }

    pub fn base_scope(&self) -> IntentScope {
        self.base
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn reset(&mut self, base: IntentScope) {
        self.base = base;
        self.stack.clear();
    }

    pub fn push(&mut self, binding: BoundIntentId) {
        self.stack.push(binding);
    }

    pub fn pop(&mut self) -> Option<BoundIntentId> {
        self.stack.pop()
    }
}

#[derive(Debug, Default)]
pub struct Frame {
    pub pc: usize,
    pub exec: ExecCtx,
    pub bindings: FrameBindings,
    pub intent_state: IntentScopeState,
    pub call_argv: Value,
    pub types: Option<FuncArgvTypes>,
    pub codes: ByteView,
    pub oprnds: Stack,
    pub locals: Stack,
    pub heap: Heap,
    ir_format_fee_pending: i64,
}

impl Frame {
    pub fn reclaim<M: VmMachine + ?Sized>(self, machine: &mut M) {
        machine.stack_reclaim(self.oprnds);
        machine.stack_reclaim(self.locals);
        machine.heap_reclaim(self.heap);
    }

    pub fn new<M: VmMachine + ?Sized>(machine: &mut M) -> Self {
        let mut f = Self {
            oprnds: machine.stack_allocat(),
            locals: machine.stack_allocat(),
            heap: machine.heap_allocat(),
            ..Default::default()
        };
        let cap = machine.space_cap();
        f.oprnds.reset(cap.stack_slot);
        f.locals.reset(cap.local_slot);
        f.heap.reset(cap.heap_segment);
        f
    }

    pub fn next<M: VmMachine + ?Sized>(&self, machine: &mut M) -> Self {
        let mut f = Self::new(machine);
        let stks = self.oprnds.limit() - self.oprnds.len();
        let locs = self.locals.limit() - self.locals.len();
        f.oprnds.reset(stks);
        f.locals.reset(locs);
        f.bindings = self.bindings.clone();
        f
    }

    pub fn pop_value(&mut self) -> VmrtRes<Value> {
        self.oprnds.pop()
    }

    pub fn push_value(&mut self, v: Value) -> VmrtErr {
        self.oprnds.push(v)
    }

    pub fn check_output_type(&self, v: &mut Value, cap: &SpaceCap) -> VmrtErr {
        v.check_func_retv()?;
        v.check_boundary_value_cap(cap)?;
        v.check_container_cap(cap)?;
        match &self.types {
            Some(ty) => ty.check_output(v),
            None => Ok(()),
        }
    }

    fn clear_runtime_state(&mut self) {
        self.oprnds.clear();
        self.locals.clear();
        self.heap.reset(self.heap.limit());
    }

    fn prepare_common(
        &mut self,
        exec: ExecCtx,
        bindings: FrameBindings,
        fnobj: &FnObj,
        height: u64,
        gas_extra: &GasExtra,
        mut argv: Value,
        have_param: bool,
        cap: &SpaceCap,
    ) -> VmrtErr {
        self.clear_runtime_state();
        self.ir_format_fee_pending = 0;
        if have_param {
            if let Some(vtys) = &fnobj.agvty {
                vtys.check_params(&mut argv)?;
            }
            argv.check_boundary_value_cap(cap)?;
            argv.check_container_cap(cap)?;
            self.oprnds.push(argv.clone())?;
        }
        self.bindings = bindings;
        self.call_argv = argv;
        self.types = fnobj.agvty.clone();
        self.pc = 0;
        self.exec = exec;
        self.codes = fnobj.exec_bytecodes(height, gas_extra)?;
        self.ir_format_fee_pending = if matches!(fnobj.ctype, CodeType::IRNode) {
            gas_extra.ir_format_bytes(fnobj.codes.len())
        } else {
            0
        };
        Ok(())
    }

    pub fn prepare_invoke_unchecked_shape(
        &mut self,
        exec: ExecCtx,
        bindings: FrameBindings,
        fnobj: &FnObj,
        height: u64,
        gas_extra: &GasExtra,
        param: Value,
        cap: &SpaceCap,
    ) -> VmrtErr {
        self.intent_state.reset(bindings.intent_scope);
        self.prepare_common(exec, bindings, fnobj, height, gas_extra, param, true, cap)
    }

    pub fn prepare(
        &mut self,
        exec: ExecCtx,
        bindings: FrameBindings,
        fnobj: &FnObj,
        height: u64,
        gas_extra: &GasExtra,
        param: Option<Value>,
        cap: &SpaceCap,
    ) -> VmrtErr {
        let have_param = param.is_some();
        let argv = param.unwrap_or(Value::Nil);
        if have_param {
            argv.check_func_argv()?;
        }
        self.intent_state.reset(bindings.intent_scope);
        self.prepare_common(
            exec, bindings, fnobj, height, gas_extra, argv, have_param, cap,
        )
    }

    pub fn prepare_splice(
        &mut self,
        exec: ExecCtx,
        bindings: FrameBindings,
        fnobj: &FnObj,
        height: u64,
        gas_extra: &GasExtra,
        param: Value,
        cap: &SpaceCap,
    ) -> VmrtErr {
        self.ir_format_fee_pending = 0;
        param.check_func_argv()?;
        param.check_boundary_value_cap(cap)?;
        param.check_container_cap(cap)?;
        let caller_output = match &self.types {
            Some(types) => types
                .output_type()
                .map_err(|e| ItrErr::new(ItrErrCode::CallArgvTypeFail, &e.to_string()))?,
            None => None,
        };
        let callee_params = match &fnobj.agvty {
            Some(types) => types
                .param_types()
                .map_err(|e| ItrErr::new(ItrErrCode::CallArgvTypeFail, &e.to_string()))?,
            None => vec![],
        };
        self.types = if caller_output.is_none() && callee_params.is_empty() {
            None
        } else {
            Some(
                FuncArgvTypes::from_types(caller_output, callee_params)
                    .map_err(|e| ItrErr::new(ItrErrCode::CallArgvTypeFail, &e.to_string()))?,
            )
        };
        self.bindings = bindings;
        self.pc = 0;
        self.exec = exec;
        self.call_argv = param;
        self.codes = fnobj.exec_bytecodes(height, gas_extra)?;
        self.ir_format_fee_pending = if matches!(fnobj.ctype, CodeType::IRNode) {
            gas_extra.ir_format_bytes(fnobj.codes.len())
        } else {
            0
        };
        Ok(())
    }

    pub fn execute<M: VmMachine + ?Sized, H: VmHost + base::Context + ?Sized>(
        &mut self,
        machine: &mut M,
        host: &mut H,
    ) -> VmrtRes<CallExit> {
        let context_addr = self.bindings.context_addr;
        let current_addr = self
            .bindings
            .code_contract
            .as_ref()
            .map(ContractAddress::to_addr)
            .unwrap_or(context_addr);
        if self.ir_format_fee_pending > 0 {
            let fee = self.ir_format_fee_pending;
            machine.settle_resource_gas(host, fee)?;
            self.ir_format_fee_pending = 0;
        }
        execute_code_in_frame(
            &mut self.pc,
            self.codes.as_slice(),
            self.exec,
            &mut self.oprnds,
            &mut self.locals,
            &mut self.heap,
            &mut self.bindings,
            &mut self.intent_state,
            &context_addr,
            &current_addr,
            machine,
            host,
        )
    }
}
