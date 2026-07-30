//! AstSelect / AstIf compositional actions.

use std::any::Any;
use std::sync::Arc;

use base::{
    ActOut, ActScope, Action, ActionDispatcher, ActionRef, AddrOrPtr, BinaryCodecs, Context,
    TopRule,
};
use field::{Decode, Encode, Reader, Uint1, Uint2};
use sys::{Rerr, Ret, errf};

#[derive(Debug, Clone)]
pub struct ActionListW1 {
    actions: Vec<ActionRef>,
}

#[derive(Debug, Clone)]
pub struct AstSelect {
    pub kind: Uint2,
    pub exe_min: Uint1,
    pub exe_max: Uint1,
    pub actions: ActionListW1,
}

#[derive(Debug, Clone)]
pub struct AstIf {
    pub kind: Uint2,
    pub cond: AstSelect,
    pub br_if: AstSelect,
    pub br_else: AstSelect,
}

impl ActionListW1 {
    pub fn from_vec(actions: Vec<ActionRef>) -> Ret<Self> {
        Uint1::from_usize(actions.len())?;
        Ok(Self { actions })
    }

    pub fn as_list(&self) -> &[ActionRef] {
        &self.actions
    }

    pub fn length(&self) -> usize {
        self.actions.len()
    }

    fn decode(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(Self, usize)> {
        let (count, mut used) = Uint1::decode(buf)?;
        let mut actions = Vec::with_capacity(count.uint() as usize);
        for _ in 0..count.uint() {
            let rest = &buf[used..];
            let (act, n) = reg.decode_action(rest)?;
            actions.push(act);
            used += n;
        }
        Ok((Self { actions }, used))
    }
}

impl AstSelect {
    pub const KIND: u16 = 25;

    pub fn create_by(min: u8, max: u8, actions: Vec<ActionRef>) -> Ret<Self> {
        Ok(Self {
            kind: Uint2::from(Self::KIND),
            exe_min: Uint1::from(min),
            exe_max: Uint1::from(max),
            actions: ActionListW1::from_vec(actions)?,
        })
    }

    fn collect_req_sign(&self) -> Vec<AddrOrPtr> {
        let mut req = vec![];
        for act in self.actions.as_list() {
            collect_ast_req_sign(&mut req, act.as_ref());
        }
        req
    }

    pub(crate) fn child_actions(&self) -> Vec<&dyn Action> {
        self.actions
            .as_list()
            .iter()
            .map(|a| a.as_ref() as &dyn Action)
            .collect()
    }
}

impl AstIf {
    pub const KIND: u16 = 26;

    pub fn create_by(cond: AstSelect, br_if: AstSelect, br_else: AstSelect) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            cond,
            br_if,
            br_else,
        }
    }

    fn collect_req_sign(&self) -> Vec<AddrOrPtr> {
        let mut req = self.cond.collect_req_sign();
        req.extend(self.br_if.collect_req_sign());
        req.extend(self.br_else.collect_req_sign());
        req
    }

    pub(crate) fn child_actions(&self) -> Vec<&dyn Action> {
        let mut out = Vec::new();
        out.extend(self.cond.child_actions());
        out.extend(self.br_if.child_actions());
        out.extend(self.br_else.child_actions());
        out
    }
}

fn collect_ast_req_sign(req: &mut Vec<AddrOrPtr>, act: &dyn Action) {
    if let Some(ast) = act.as_any().downcast_ref::<AstSelect>() {
        for child in ast.actions.as_list() {
            collect_ast_req_sign(req, child.as_ref());
        }
        return;
    }
    if let Some(ast) = act.as_any().downcast_ref::<AstIf>() {
        for child in ast.cond.actions.as_list() {
            collect_ast_req_sign(req, child.as_ref());
        }
        for child in ast.br_if.actions.as_list() {
            collect_ast_req_sign(req, child.as_ref());
        }
        for child in ast.br_else.actions.as_list() {
            collect_ast_req_sign(req, child.as_ref());
        }
        return;
    }
    req.extend(act.req_sign());
}

impl Encode for ActionListW1 {
    fn size(&self) -> usize {
        Uint1::SIZE + self.actions.iter().map(|a| a.size()).sum::<usize>()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        Uint1::from(self.actions.len() as u8).encode_to(out);
        for action in &self.actions {
            action.encode_to(out);
        }
    }
}

impl Encode for AstSelect {
    fn size(&self) -> usize {
        self.kind.size() + self.exe_min.size() + self.exe_max.size() + self.actions.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.exe_min.encode_to(out);
        self.exe_max.encode_to(out);
        self.actions.encode_to(out);
    }
}

impl Encode for AstIf {
    fn size(&self) -> usize {
        self.kind.size() + self.cond.size() + self.br_if.size() + self.br_else.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.cond.encode_to(out);
        self.br_if.encode_to(out);
        self.br_else.encode_to(out);
    }
}

fn validate_ast_select(min: usize, max: usize, num: usize) -> Rerr {
    if min > max {
        return sys::errf!("action ast select max cannot be less than min");
    }
    if max > num {
        return sys::errf!("action ast select max cannot exceed list num");
    }
    if num > base::TX_ACTIONS_MAX {
        return sys::errf!(
            "action ast select num cannot exceed {}",
            base::TX_ACTIONS_MAX
        );
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

impl Action for AstSelect {
    fn kind(&self) -> u16 {
        Self::KIND
    }

    fn scope(&self) -> ActScope {
        ActScope {
            top: Some(TopRule::None),
            allow_ast: true,
            allow_call: false,
        }
    }

    fn min_tx_type(&self) -> u8 {
        3
    }

    fn req_sign(&self) -> Vec<AddrOrPtr> {
        self.collect_req_sign()
    }

    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        // AST control nodes are structural. Their children and snapshot
        // boundaries are metered; charging the serialized wrapper here would
        // double-charge relative to dev.
        let gas = 0;
        let min = self.exe_min.uint() as usize;
        let max = self.exe_max.uint() as usize;
        validate_ast_select(min, max, self.actions.length())?;
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

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for AstIf {
    fn kind(&self) -> u16 {
        Self::KIND
    }

    fn scope(&self) -> ActScope {
        ActScope {
            top: Some(TopRule::None),
            allow_ast: true,
            allow_call: false,
        }
    }

    fn min_tx_type(&self) -> u8 {
        3
    }

    fn req_sign(&self) -> Vec<AddrOrPtr> {
        self.collect_req_sign()
    }

    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
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

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_ast_select(
    reg: &dyn BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != AstSelect::KIND {
        return sys::decodef!("AstSelect codec got kind {}", kind.uint());
    }
    let exe_min: Uint1 = r.read()?;
    let exe_max: Uint1 = r.read()?;
    let (actions, used) = ActionListW1::decode(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    Ok((
        Arc::new(AstSelect {
            kind,
            exe_min,
            exe_max,
            actions,
        }),
        r.used(),
    ))
}

fn decode_ast_select_inline(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(AstSelect, usize)> {
    let (act, used) = create_ast_select(reg, AstSelect::KIND, buf)?;
    let Some(ast) = act.as_any().downcast_ref::<AstSelect>() else {
        return sys::decodef!("AstSelect decode type mismatch");
    };
    Ok((ast.clone(), used))
}

pub fn create_ast_if(reg: &dyn BinaryCodecs, _kind: u16, buf: &[u8]) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != AstIf::KIND {
        return sys::decodef!("AstIf codec got kind {}", kind.uint());
    }
    let (cond, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    let (br_if, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    let (br_else, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    Ok((
        Arc::new(AstIf {
            kind,
            cond,
            br_if,
            br_else,
        }),
        r.used(),
    ))
}
