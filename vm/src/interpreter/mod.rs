use std::ops::{BitAnd, BitOr, BitXor};

use crate::machine::{VmHost, VmMachine};
use crate::native::*;
use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::space::*;
use crate::value::Value::*;
use crate::value::*;
use base::{GasBuckets, VmHostAllowedPolicy, VmHostCallKind};

fn action_kind_name(act_kind: Bytecode) -> &'static str {
    match act_kind {
        Bytecode::ACTION => "ACTION",
        Bytecode::ACTENV => "ACTENV",
        Bytecode::ACTVIEW => "ACTVIEW",
        _ => "ACTION?",
    }
}

fn act_host_kind(act_kind: Bytecode) -> Option<VmHostCallKind> {
    match act_kind {
        Bytecode::ACTION => Some(VmHostCallKind::Action),
        Bytecode::ACTENV => Some(VmHostCallKind::Env),
        Bytecode::ACTVIEW => Some(VmHostCallKind::View),
        _ => None,
    }
}

/// Enforce Registry `VmHostAllowedPolicy` at ACTION / ACTENV / ACTVIEW sites.
fn ensure_act_allowed<H: VmHost + ?Sized>(
    host: &H,
    exec: ExecCtx,
    act_kind: Bytecode,
    id: u8,
) -> VmrtErr {
    if exec.effect == EffectMode::Pure {
        return Err(ItrErr::new(
            ItrErrCode::ActDisabled,
            &format!("{} not supported in pure call", action_kind_name(act_kind)),
        ));
    }

    let Some(kind) = act_host_kind(act_kind) else {
        return Err(ItrErr::new(
            ItrErrCode::InstParamsErr,
            &format!("unknown host action kind {}", action_kind_name(act_kind)),
        ));
    };
    let Some(def) = host.vm_host_def(kind, id) else {
        return Err(ItrErr::new(
            ItrErrCode::InstParamsErr,
            &format!("{} id {} not found", action_kind_name(act_kind), id),
        ));
    };

    match def.allowed_policy {
        VmHostAllowedPolicy::Any => Ok(()),
        VmHostAllowedPolicy::TopOnly => {
            if exec.entry != EntryKind::Main
                || exec.effect != EffectMode::Edit
                || !exec.is_outer_entry()
            {
                return Err(ItrErr::new(
                    ItrErrCode::ActDisabled,
                    "action not supported in current call context",
                ));
            }
            Ok(())
        }
        VmHostAllowedPolicy::CallOnly => {
            // Nested / non-top Edit calls only (not Main depth-0).
            if exec.effect != EffectMode::Edit || exec.is_outer_entry() {
                return Err(ItrErr::new(
                    ItrErrCode::ActDisabled,
                    "action only supported in nested call context",
                ));
            }
            Ok(())
        }
        VmHostAllowedPolicy::ViewOnly => {
            // Readable from Edit or View; Pure already rejected above.
            Ok(())
        }
    }
}

#[derive(Clone, Copy)]
struct HostOpcodeAbi {
    consumes_body: bool,
    produces_value: bool,
}

/// Stack and body behavior belongs to the opcode, not to a registered host.
/// Host definitions select the capability, return value type and call policy.
const fn host_opcode_abi(act_kind: Bytecode) -> HostOpcodeAbi {
    match act_kind {
        Bytecode::ACTION => HostOpcodeAbi {
            consumes_body: true,
            produces_value: false,
        },
        Bytecode::ACTVIEW => HostOpcodeAbi {
            consumes_body: true,
            produces_value: true,
        },
        Bytecode::ACTENV => HostOpcodeAbi {
            consumes_body: false,
            produces_value: true,
        },
        _ => HostOpcodeAbi {
            consumes_body: false,
            produces_value: false,
        },
    }
}

fn act_retv_type<H: VmHost + ?Sized>(host: &H, act_kind: Bytecode, idx: u8) -> VmrtRes<ValueTy> {
    use base::VmValueType;
    let Some(kind) = act_host_kind(act_kind) else {
        return Ok(ValueTy::Nil);
    };
    let Some(def) = host.vm_host_def(kind, idx) else {
        return Err(ItrErr::new(
            ItrErrCode::InstParamsErr,
            &format!("{} id {} not found", action_kind_name(act_kind), idx),
        ));
    };
    Ok(match def.ret {
        VmValueType::Nil => ValueTy::Nil,
        VmValueType::Bool => ValueTy::Bool,
        VmValueType::U8 => ValueTy::U8,
        VmValueType::U64 => ValueTy::U64,
        VmValueType::Address => ValueTy::Address,
        VmValueType::Bytes => ValueTy::Bytes,
    })
}

include!("operand.rs");
include!("instruction.rs");
include!("execute.rs");
