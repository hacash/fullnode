use crate::frame::IntentScopeState;
use crate::machine::{DeferredRegistry, IntentRuntime};
use crate::rt::{EffectMode, ExecCtx, FrameBindings, ItrErr, ItrErrCode, SpaceCap, VmrtRes};
use crate::value::Value;

use super::intent::*;
use super::{NativeCtl, NativeEnv, NativeFunc};

pub fn call_ntfunc(hei: u64, idx: u8, argv: &[u8]) -> VmrtRes<(Value, i64)> {
    NativeFunc::call(hei, idx, argv)
}

pub fn call_ntctl(
    exec: ExecCtx,
    cap: &SpaceCap,
    bindings: &mut FrameBindings,
    intent_state: &mut IntentScopeState,
    _context_addr: &field::Address,
    intents: &mut IntentRuntime,
    deferred_registry: &mut DeferredRegistry,
    idx: u8,
    argv: Value,
) -> VmrtRes<(Value, i64)> {
    match NativeCtl::try_from_u8(idx)? {
        NativeCtl::defer => call_defer(exec, bindings, intents, deferred_registry, argv),
        NativeCtl::intent_new => call_intent_new(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_use => call_intent_use(exec, cap, bindings, intent_state, intents, argv),
        NativeCtl::intent_pop => call_intent_pop(exec, bindings, intent_state, argv),
        NativeCtl::intent_is_own_handle => {
            call_intent_is_own_handle(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_kind => call_intent_kind(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_kind_is => {
            call_intent_kind_is(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_destroy => {
            call_intent_destroy(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_destroy_if_empty => {
            call_intent_destroy_if_empty(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_clear => call_intent_clear(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_len => call_intent_len(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_has => call_intent_has(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_keys => call_intent_keys(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_keys_page => {
            call_intent_keys_page(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_keys_after => {
            call_intent_keys_after(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_get => call_intent_get(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_get_or => call_intent_get_or(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_require => {
            call_intent_require(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_require_eq => {
            call_intent_require_eq(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_require_absent => {
            call_intent_require_absent(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_require_many => {
            call_intent_require_many(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_require_map => {
            call_intent_require_map(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_has_all => {
            call_intent_has_all(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_has_any => {
            call_intent_has_any(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_put => call_intent_put(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_put_if_absent => {
            call_intent_put_if_absent(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_put_if_absent_or_match => {
            call_intent_put_if_absent_or_match(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_put_flat_kv => {
            call_intent_put_flat_kv(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_replace => {
            call_intent_replace(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_replace_if => {
            call_intent_replace_if(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_rename => call_intent_rename(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_take => call_intent_take(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_take_or => {
            call_intent_take_or(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_take_if => {
            call_intent_take_if(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_take_many => {
            call_intent_take_many(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_take_map => {
            call_intent_take_map(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_consume => {
            call_intent_consume(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_consume_many => {
            call_intent_consume_many(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_del => call_intent_del(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_del_if => call_intent_del_if(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_del_many => {
            call_intent_del_many(exec, bindings, intent_state, intents, argv)
        }
        NativeCtl::intent_append => call_intent_append(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_inc => call_intent_inc(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_add => call_intent_add(exec, bindings, intent_state, intents, argv),
        NativeCtl::intent_sub => call_intent_sub(exec, bindings, intent_state, intents, argv),
        NativeCtl::Null => unreachable!(),
    }
}

pub fn call_ntenv(
    exec: ExecCtx,
    _bindings: &FrameBindings,
    context_addr: &field::Address,
    idx: u8,
) -> VmrtRes<(Value, i64)> {
    if exec.effect == EffectMode::Pure {
        return itr_err_code!(ItrErrCode::InstDisabled);
    }
    let env = NativeEnv::try_from_u8(idx)?;
    let r = match env {
        NativeEnv::context_address => Value::Address(*context_addr),
        NativeEnv::Null => unreachable!(),
    };
    Ok((r, env.gas_of()))
}
