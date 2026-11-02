use super::*;

impl CallFrame {
    pub(crate) fn start_call<M: VmMachine + ?Sized, H: VmHost + base::Context + ?Sized>(
        &mut self,
        machine: &mut M,
        host: &mut H,
        exec: ExecCtx,
        code: &FnObj,
        bindings: FrameBindings,
        param: Option<Value>,
    ) -> VmrtRes<Value> {
        use crate::rt::CallExit::*;
        macro_rules! curr {
            () => {
                self.frames.last().unwrap()
            };
        }
        macro_rules! curr_mut {
            () => {
                self.frames.last_mut().unwrap()
            };
        }
        macro_rules! prepare_and_push {
            ($frame:ident, $prepare:expr) => {{
                if let Err(e) = $prepare {
                    $frame.reclaim(machine);
                    return Err(e);
                }
                self.push($frame);
            }};
        }
        macro_rules! settle_return {
            ($retv:expr) => {{
                let mut retv = $retv;
                let cap = machine.space_cap();
                curr!().check_output_type(&mut retv, &cap)?;
                self.pop().unwrap().reclaim(machine);
                loop {
                    let is_tail = match self.frames.last() {
                        Some(f) => f.pc == f.codes.len(),
                        None => return Ok(retv),
                    };
                    if !is_tail {
                        curr_mut!().push_value(retv)?;
                        break;
                    }
                    let cap = machine.space_cap();
                    self.frames
                        .last()
                        .unwrap()
                        .check_output_type(&mut retv, &cap)?;
                    self.pop().unwrap().reclaim(machine);
                }
            }};
        }

        assert!(self.len() == 0);
        let height = machine.height();
        let gas_extra = machine.gas_extra();
        let cap = machine.space_cap();

        exec.ensure_call_depth(&cap)?;
        let mut root = self.increase(machine)?;
        prepare_and_push!(
            root,
            root.prepare(exec, bindings, code, height, &gas_extra, param, &cap)
        );

        loop {
            machine.check_deadline()?;
            let exit = curr_mut!().execute(machine, host)?;
            match exit {
                Call(spec) => {
                    let curr_exec = curr!().exec;
                    let curr_bindings = curr!().bindings.clone();
                    let next_effect = spec.callee_effect(curr_exec.effect);
                    let cap = machine.space_cap();
                    let next_exec = curr_exec.enter_call(next_effect, &cap)?;
                    curr_mut!().oprnds.peek()?.check_func_argv()?;
                    curr_mut!().oprnds.peek()?.check_boundary_value_cap(&cap)?;
                    curr_mut!().oprnds.peek()?.check_container_cap(&cap)?;
                    let mut plan = machine.plan_user_call(host, &spec, &curr_bindings)?;
                    plan.next_bindings.intent_scope = curr!().intent_state.current_scope();

                    if let Some(calcfn) = plan.fnobj.rv64im_fn().cloned() {
                            let mut param = curr_mut!().pop_value()?;
                            let item_count = match &param {
                                Value::Compo(c) if c.is_list() => Some(c.len()),
                                _ => None,
                            };
                            if let Some(n) = item_count {
                                machine.settle_resource_gas(
                                    host,
                                    gas_extra.compo_items_read(n),
                                )?;
                            }
                            // Calc input is the same byte-string slot as
                            // ACTION / Concat-NTFUNC: `extract_call_data`
                            // flattens a scalar or a one-layer Compo list under
                            // `call_data_size` (5120, also `CALC_IO_MAX`). A
                            // kernel that needs more than the HVM scalar cap
                            // uses `list { chunk, ... }`; a Tuple is packed
                            // argv and is rejected here like everywhere else.
                            // The kernel's own `input_max` is enforced by the
                            // calc pre-flight in `run`. A List must not be
                            // force-cast back to a scalar, so it skips the
                            // declared `bytes` type check; scalars keep the
                            // normal cast.
                            if let Some(vtys) = plan.fnobj.agvty.as_ref() {
                                if !matches!(param, Value::Compo(_)) {
                                    vtys.check_params(&mut param)?;
                                }
                            }
                            let input_bytes = param.extract_call_data(&cap)?;
                            let cur = machine.gas_use();
                            let host_rem = VmHost::gas_remaining(host).max(0);
                            let in_res = crate::calc::ceil_div(input_bytes.len() as u64, 16) as i64;
                            let mem_res = calcfn.mem_kib() as i64
                                * crate::calc::CALC_MEM_RESOURCE_PER_KIB;
                            let out_res_max =
                                crate::calc::ceil_div(calcfn.output_max() as u64, 16) as i64;
                            let max_res = in_res.saturating_add(mem_res).saturating_add(out_res_max);
                            let host_compute = (host_rem.saturating_sub(max_res).max(0)) as u64;
                            let limit_compute = if gas_extra.compute_limit > 0 {
                                (gas_extra.compute_limit - cur.compute).max(0) as u64
                            } else {
                                u64::MAX
                            };
                            // No separate per-call cycle cap. The budget is the
                            // remaining compute bucket, converted at
                            // CYCLES_PER_COMPUTE, also clipped by compute_limit.
                            let cycles = host_compute
                                .min(limit_compute)
                                .saturating_mul(crate::calc::CYCLES_PER_COMPUTE);
                            let limit_resource = if gas_extra.resource_limit > 0 {
                                (gas_extra.resource_limit - cur.resource).max(0)
                            } else {
                                i64::MAX
                            };
                            let resource = limit_resource.min(host_rem);
                            let limits = crate::calc::CalcRunLimits {
                                cycles,
                                resource,
                                value_size: cap.value_size,
                                deadline: machine.deadline(),
                            };
                            let exec = calcfn.run(&input_bytes, limits);
                            // Disagreeing engines: reject before any calc gas
                            // is settled. There is no agreed charge to keep.
                            if matches!(exec.result, Err(crate::calc::CalcAbort2::EngineMismatch)) {
                                return itr_err_fmt!(
                                    ItrErrCode::CalcEngineMismatch,
                                    "calc AOT/interpreter mismatch"
                                );
                            }
                            let compute_gas = crate::calc::ceil_div(
                                exec.cycles_used,
                                crate::calc::CYCLES_PER_COMPUTE,
                            ) as i64;
                            machine.settle_calc_gas(host, compute_gas, exec.resource)?;
                            match exec.result {
                                Ok(out) => {
                                    let v = Value::Bytes(out).valid(&cap)?;
                                    if curr!().pc == curr!().codes.len() {
                                        settle_return!(v);
                                    } else {
                                        curr_mut!().push_value(v)?;
                                    }
                                    continue;
                                }
                                Err(crate::calc::CalcAbort2::Trap) => {
                                    return itr_err_fmt!(ItrErrCode::CalcTrap, "calc kernel trap");
                                }
                                Err(crate::calc::CalcAbort2::OutOfGas) => {
                                    return itr_err_fmt!(
                                        ItrErrCode::OutOfGas,
                                        "calc cycle budget exhausted"
                                    );
                                }
                                Err(crate::calc::CalcAbort2::OutOfResource) => {
                                    return itr_err_fmt!(
                                        ItrErrCode::OutOfGas,
                                        "calc resource budget exhausted"
                                    );
                                }
                                Err(crate::calc::CalcAbort2::Deadline) => {
                                    return itr_err_fmt!(
                                        ItrErrCode::ExecutionDeadline,
                                        "calc execution deadline exceeded"
                                    );
                                }
                                Err(crate::calc::CalcAbort2::ValueTooLarge) => {
                                    return itr_err_fmt!(
                                        ItrErrCode::OutOfValueSize,
                                        "calc value size exceeds cap"
                                    );
                                }
                                // Already returned above, before gas settle.
                                Err(crate::calc::CalcAbort2::EngineMismatch) => {
                                    return itr_err_fmt!(
                                        ItrErrCode::CalcEngineMismatch,
                                        "calc AOT/interpreter mismatch"
                                    );
                                }
                            }
                    } else {
                        match spec {
                            CallSpec::Splice { .. } => {
                                let mut param = curr_mut!().pop_value()?;
                                if let Some(vtys) = plan.fnobj.agvty.as_ref() {
                                    vtys.check_params(&mut param)?;
                                }
                                curr_mut!().push_value(param.clone())?;
                                curr_mut!().prepare_splice(
                                    next_exec,
                                    plan.next_bindings,
                                    plan.fnobj.as_ref(),
                                    height,
                                    &gas_extra,
                                    param,
                                    &cap,
                                )?;
                                continue;
                            }
                            CallSpec::Invoke { .. } => {
                                let param = curr_mut!().pop_value()?;
                                let mut next = self.increase(machine)?;
                                prepare_and_push!(
                                    next,
                                    next.prepare_invoke_unchecked_shape(
                                        next_exec,
                                        plan.next_bindings,
                                        plan.fnobj.as_ref(),
                                        height,
                                        &gas_extra,
                                        param,
                                        &cap,
                                    )
                                );
                            }
                        }
                    }
                }

                Abort | Throw | Finish | Return => {
                    let mut retv = Value::Nil;
                    if matches!(exit, Return | Throw) {
                        retv = curr_mut!().pop_value()?;
                    }
                    if matches!(exit, Abort | Throw) {
                        return itr_err_fmt!(ItrErrCode::ThrowAbort, "VM return failed: {}", retv);
                    }
                    settle_return!(retv);
                }
            }
        }
    }
}
