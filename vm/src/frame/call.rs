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
