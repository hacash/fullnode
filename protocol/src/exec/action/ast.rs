//! AstSelect / AstIf execute bodies.

use base::{ActOut, ActionDispatcher, ActionExecute, ActionRef, Context};
use sys::{Rerr, Ret, errf};

use crate::codec::action::{AstIf, AstSelect};

fn validate_ast_select(min: usize, max: usize, num: usize, max_actions: usize) -> Rerr {
    if min > max {
        return sys::errf!("action ast select max cannot be less than min");
    }
    if max > num {
        return sys::errf!("action ast select max cannot exceed list num");
    }
    if num > max_actions {
        return sys::errf!("action ast select num cannot exceed {}", max_actions);
    }
    Ok(())
}

fn run_ast_child(ctx: &mut dyn Context, act: &ActionRef) -> Ret<ActOut> {
    let gas = crate::execution_params(ctx.services().as_ref())?.ast_snapshot_try_gas;
    ctx.gas_charge(gas)?;
    let gas_snap = ctx.snapshot_volatile();
    let vm_snap = ctx.vm_peek().map(|vm| vm.snapshot_volatile());
    let had_vm = ctx.vm_peek().is_some();
    let mut run = |child: &mut dyn Context| {
        let out = ActionDispatcher::dispatch_ast(child, act)?;
        if act.extra9() {
            child.gas_charge(out.0.saturating_mul(9) as i64)?;
        }
        Ok(out)
    };
    match ctx.exec_ast_child(&mut run) {
        Ok(out) => Ok(out),
        Err(e) if e.is_revert() => {
            match (vm_snap, ctx.vm_peek().is_some()) {
                (None, false) => {}
                (None, true) => {
                    if let Some(vm) = ctx.vm_peek() {
                        vm.rollback_volatile_preserve_warm_and_gas();
                    }
                }
                (Some(snap), true) => {
                    if let Some(vm) = ctx.vm_peek() {
                        vm.restore_volatile(snap);
                    }
                }
                (Some(_), false) if had_vm => {
                    return errf!("vm disappeared during AST snapshot/recover");
                }
                (Some(_), false) => {}
            }
            ctx.restore_volatile(gas_snap);
            Err(e)
        }
        Err(e) => Err(e),
    }
}

impl ActionExecute for AstSelect {
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        // AST control nodes are structural: children and snapshot boundaries are
        // metered, charging the serialized wrapper here would double-charge relative to dev.
        let gas = 0;
        let min = self.exe_min.uint() as usize;
        let max = self.exe_max.uint() as usize;
        let max_actions = crate::execution_params(ctx.services().as_ref())?.tx_actions_max;
        validate_ast_select(min, max, self.actions.length(), max_actions)?;
        let mut ok = 0usize;
        let mut last = Vec::new();
        for act in self.actions.as_list() {
            if ok >= max {
                break;
            }
            match run_ast_child(ctx, act) {
                Ok((_, ret)) => {
                    ok += 1;
                    last = ret;
                }
                Err(e) if e.is_revert() => {}
                Err(e) => return Err(e),
            }
        }
        if ok < min {
            return sys::revertf!(
                "action ast select must succeed at least {} but only {}",
                min,
                ok
            );
        }
        Ok((gas, last))
    }
}

impl ActionExecute for AstIf {
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        use std::sync::Arc;

        let gas = 0;
        let cond_ref: ActionRef = Arc::new(self.cond.clone());
        let cond_ok = match run_ast_child(ctx, &cond_ref) {
            Ok(_) => true,
            Err(e) if e.is_revert() => false,
            Err(e) => return Err(e),
        };
        let branch: ActionRef = if cond_ok {
            Arc::new(self.br_if.clone())
        } else {
            Arc::new(self.br_else.clone())
        };
        let (_, ret) = run_ast_child(ctx, &branch)?;
        Ok((gas, ret))
    }
}
