use field::Address;
use sys::Ret;

use crate::iface::action::{ActOut, ActionRef, resolve_transfer_routing};
use crate::iface::context::Context;
use crate::iface::vm::VmEntry;
use crate::runtime::ExecFrom;

pub struct ActionDispatcher;

impl ActionDispatcher {
    pub fn dispatch_top(ctx: &mut dyn Context, action: &ActionRef) -> Ret<ActOut> {
        let (gas, ret) = Self::dispatch(ctx, action, ExecFrom::Top)?;
        Self::charge_top_extra9(ctx, action, gas)?;
        Ok((gas, ret))
    }

    pub fn dispatch_top_without_extra9(ctx: &mut dyn Context, action: &ActionRef) -> Ret<ActOut> {
        Self::dispatch(ctx, action, ExecFrom::Top)
    }

    pub fn dispatch_call(ctx: &mut dyn Context, action: &ActionRef) -> Ret<ActOut> {
        let (gas, ret) = Self::dispatch(ctx, action, ExecFrom::Call)?;
        Ok((Self::returned_call_extra9(action, gas), ret))
    }

    pub fn dispatch_ast(ctx: &mut dyn Context, action: &ActionRef) -> Ret<ActOut> {
        Self::dispatch(ctx, action, ExecFrom::Ast)
    }

    fn dispatch(ctx: &mut dyn Context, action: &ActionRef, from: ExecFrom) -> Ret<ActOut> {
        if !action.scope().allows(from) {
            return sys::errf!(
                "action kind {} scope does not allow {}",
                action.kind(),
                from
            );
        }
        if ctx.env().tx.ty < action.min_tx_type() {
            return sys::errf!(
                "action kind {} requires tx type >= {} but got {}",
                action.kind(),
                action.min_tx_type(),
                ctx.env().tx.ty
            );
        }
        let need = action.required_flags();
        if need & !ctx.env().chain.consensus_flags != 0 {
            return sys::errf!(
                "action kind {} not activated (flags need {:#x} have {:#x})",
                action.kind(),
                need,
                ctx.env().chain.consensus_flags
            );
        }
        let mut seen = Vec::<Address>::new();
        for ptr in action.req_sign() {
            let adr = ctx.addr(&ptr)?;
            if seen.contains(&adr) {
                continue;
            }
            seen.push(adr);
            if adr.is_privkey() {
                ctx.check_sign(&adr)?;
            }
        }

        let prev = ctx.exec_from();
        ctx.exec_from_set(from);
        let res = action.execute(ctx);
        ctx.exec_from_set(prev);
        let (gas, ret) = res?;

        if from != ExecFrom::Call {
            if let Some(r) = resolve_transfer_routing(action.as_ref(), ctx)? {
                if r.authorize {
                    ctx.vm_call(VmEntry::TransferAuthorize {
                        owner: r.from,
                        to: r.to,
                        action_kind: r.action_kind,
                        payload: r.payload.clone(),
                    })?;
                }
                if r.receive {
                    ctx.vm_call(VmEntry::TransferReceive {
                        from: r.from,
                        to: r.to,
                        action_kind: r.action_kind,
                        payload: r.payload,
                    })?;
                }
            }
        }
        Ok((gas, ret))
    }

    fn charge_top_extra9(ctx: &mut dyn Context, action: &ActionRef, gas: u32) -> sys::Rerr {
        if action.extra9() {
            ctx.gas_charge(gas.saturating_mul(9) as i64)?;
        }
        Ok(())
    }

    fn returned_call_extra9(action: &ActionRef, gas: u32) -> u32 {
        if action.extra9() {
            gas.saturating_mul(9)
        } else {
            0
        }
    }
}

pub struct ExecFromGuard<'a> {
    ctx: &'a mut dyn Context,
    prev: ExecFrom,
}

impl<'a> ExecFromGuard<'a> {
    pub fn enter(ctx: &'a mut dyn Context, from: ExecFrom) -> Self {
        let prev = ctx.exec_from();
        ctx.exec_from_set(from);
        Self { ctx, prev }
    }

    pub fn ctx(&mut self) -> &mut dyn Context {
        self.ctx
    }
}

impl Drop for ExecFromGuard<'_> {
    fn drop(&mut self) {
        self.ctx.exec_from_set(self.prev);
    }
}

pub fn with_exec_from<R>(
    ctx: &mut dyn Context,
    from: ExecFrom,
    f: impl FnOnce(&mut dyn Context) -> R,
) -> R {
    let mut guard = ExecFromGuard::enter(ctx, from);
    f(guard.ctx())
}
