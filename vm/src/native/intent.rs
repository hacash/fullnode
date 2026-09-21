use std::collections::VecDeque;

use crate::frame::IntentScopeState;
use crate::machine::{DeferredRegistry, IntentRuntime};
use crate::rt::{
    BoundIntentId, EffectMode, ExecCtx, FrameBindings, ItrErr, ItrErrCode, SpaceCap, VmrtErr,
    VmrtRes,
};
use crate::value::{CompoItem, ContractAddress, IntentId, TupleItem, Value};

use super::NativeCtl;

fn ctl_expect_no_arg(argv: Value, name: &str) -> VmrtErr {
    if argv.is_nil() {
        return Ok(());
    }
    itr_err_fmt!(
        ItrErrCode::IntentError,
        "native ctl '{}' expects no arguments",
        name
    )
}

fn ctl_require_edit(exec: ExecCtx, name: &str) -> VmrtErr {
    if exec.effect != EffectMode::Edit {
        return itr_err_fmt!(
            ItrErrCode::IntentError,
            "native ctl '{}' not allowed in non-edit mode",
            name
        );
    }
    Ok(())
}

fn ctl_require_non_pure(exec: ExecCtx, name: &str) -> VmrtErr {
    if exec.effect == EffectMode::Pure {
        return itr_err_fmt!(
            ItrErrCode::IntentError,
            "native ctl '{}' not allowed in pure mode",
            name
        );
    }
    Ok(())
}

fn ctl_contract_owner(bindings: &FrameBindings, name: &str) -> VmrtRes<ContractAddress> {
    bindings.this_contract.ok_or_else(|| {
        ItrErr::new(
            ItrErrCode::IntentError,
            &format!("intent '{}' only allowed in contract context", name),
        )
    })
}

fn bound_intent_id(intent_state: &IntentScopeState, name: &str) -> VmrtRes<usize> {
    intent_state.current_bound_intent_id().ok_or_else(|| {
        ItrErr::new(
            ItrErrCode::IntentError,
            &format!("intent '{}' requires bound intent", name),
        )
    })
}

fn extract_intent_handle_id(argv: &Value, name: &str) -> VmrtRes<usize> {
    let err_msg = format!("native ctl '{}' requires intent handle", name);
    extract_intent_handle_id_with_error(argv, ItrErrCode::IntentError, &err_msg)
}

pub(super) fn extract_intent_handle_id_with_error(
    argv: &Value,
    err_code: ItrErrCode,
    err_msg: &str,
) -> VmrtRes<usize> {
    let Some(handle) = argv.match_handle() else {
        return itr_err_fmt!(err_code, "{}", err_msg);
    };
    let Some(intent_id) = handle.downcast_ref::<IntentId>() else {
        return itr_err_fmt!(err_code, "{}", err_msg);
    };
    Ok(intent_id.0)
}

fn optional_intent_binding(
    argv: &Value,
    intents: &IntentRuntime,
    name: &str,
) -> VmrtRes<BoundIntentId> {
    if argv.is_nil() {
        return Ok(None);
    }
    let id = extract_intent_handle_id(argv, name)?;
    if !intents.exists(id) {
        return itr_err_fmt!(ItrErrCode::IntentError, "intent {} not found", id);
    }
    Ok(Some(id))
}

fn owned_bound_intent(
    bindings: &FrameBindings,
    intent_state: &IntentScopeState,
    intents: &IntentRuntime,
    name: &str,
) -> VmrtRes<(ContractAddress, usize)> {
    let owner = ctl_contract_owner(bindings, name)?;
    let id = bound_intent_id(intent_state, name)?;
    intents.ensure_owner(&owner, id)?;
    Ok((owner, id))
}

fn tuple_argv_error(name: &str, expect: usize) -> ItrErr {
    ItrErr::new(
        ItrErrCode::IntentError,
        &format!("native ctl '{}' requires {} arguments", name, expect),
    )
}

fn ctl_expect_tuple(argv: Value, name: &str, expect: usize) -> VmrtRes<Vec<Value>> {
    let Value::Tuple(args) = argv else {
        return Err(tuple_argv_error(name, expect));
    };
    if args.len() != expect {
        return Err(tuple_argv_error(name, expect));
    }
    Ok(args.to_vec())
}

fn list_argv_error(name: &str) -> ItrErr {
    ItrErr::new(
        ItrErrCode::IntentError,
        &format!("native ctl '{}' requires list argument", name),
    )
}

fn ctl_expect_list(argv: Value, name: &str) -> VmrtRes<Vec<Value>> {
    let compo = argv.compo_ref().map_err(|_| list_argv_error(name))?;
    let list = compo.list_ref().map_err(|_| list_argv_error(name))?;
    Ok(list.iter().cloned().collect())
}

fn ctl_expect_pair(argv: Value, name: &str) -> VmrtRes<(Value, Value)> {
    let mut items = ctl_expect_tuple(argv, name, 2)?.into_iter();
    Ok((items.next().unwrap(), items.next().unwrap()))
}

fn ctl_expect_kv_list(argv: Value, name: &str) -> VmrtRes<Vec<(Value, Value)>> {
    let items = ctl_expect_list(argv, name)?;
    if items.len() % 2 != 0 {
        return itr_err_fmt!(
            ItrErrCode::IntentError,
            "native ctl '{}' requires list(key,value,...)",
            name
        );
    }
    let mut pairs = Vec::with_capacity(items.len() / 2);
    let mut iter = items.into_iter();
    while let Some(key) = iter.next() {
        let val = iter.next().unwrap();
        pairs.push((key, val));
    }
    Ok(pairs)
}

fn ctl_expect_bytes(value: &Value, name: &str, label: &str) -> VmrtRes<Vec<u8>> {
    value.extract_bytes().map_err(|_| {
        ItrErr::new(
            ItrErrCode::IntentError,
            &format!("native ctl '{}' requires {} bytes argument", name, label),
        )
    })
}

fn ctl_expect_u32(value: &Value, name: &str, label: &str) -> VmrtRes<u32> {
    value.extract_u32().map_err(|_| {
        ItrErr::new(
            ItrErrCode::IntentError,
            &format!(
                "native ctl '{}' requires {} uint-family argument convertible to u32",
                name, label
            ),
        )
    })
}

fn ctl_expect_optional_bytes<'a>(
    value: &'a Value,
    name: &str,
    label: &str,
) -> VmrtRes<Option<&'a Value>> {
    if value.is_nil() {
        return Ok(None);
    }
    if matches!(value, Value::Bytes(_)) {
        return Ok(Some(value));
    }
    itr_err_fmt!(
        ItrErrCode::IntentError,
        "native ctl '{}' requires {} bytes argument",
        name,
        label
    )
}

fn ctl_put_kv_list(
    exec: ExecCtx,
    bindings: &FrameBindings,
    intent_state: &IntentScopeState,
    intents: &mut IntentRuntime,
    argv: Value,
    name: &str,
) -> VmrtErr {
    ctl_require_edit(exec, name)?;
    let pairs = ctl_expect_kv_list(argv, name)?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, name)?;
    intents.put_many(&owner, id, pairs)
}

pub fn call_defer(
    exec: ExecCtx,
    bindings: &FrameBindings,
    intents: &mut IntentRuntime,
    deferred_registry: &mut DeferredRegistry,
    argv: Value,
) -> VmrtRes<(Value, i64)> {
    if exec.effect != EffectMode::Edit {
        return itr_err_fmt!(
            ItrErrCode::DeferredError,
            "defer not allowed in non-edit mode"
        );
    }
    let Some(caddr) = bindings.this_contract else {
        return itr_err_fmt!(ItrErrCode::DeferredError, "defer requires contract context");
    };
    let intent_scope = if argv.is_nil() {
        Some(None)
    } else {
        let id = extract_intent_handle_id_with_error(
            &argv,
            ItrErrCode::DeferredError,
            "defer requires intent handle",
        )?;
        intents
            .ensure_owner(&caddr, id)
            .map_err(|e| ItrErr::new(ItrErrCode::DeferredError, &e.1))?;
        Some(Some(id))
    };
    deferred_registry.register_current(&caddr, intent_scope)?;
    Ok((Value::Nil, NativeCtl::defer.gas_of()))
}

/// Register the intent this frame is already bound to as its deferred settlement
/// hook. Produces the same registry entry as `defer(handle)` for the same intent,
/// so the two forms dedup against each other; the difference is only where the id
/// comes from — the frame's binding instead of a handle value.
pub fn call_defer_current(
    exec: ExecCtx,
    bindings: &FrameBindings,
    intent_state: &IntentScopeState,
    intents: &mut IntentRuntime,
    deferred_registry: &mut DeferredRegistry,
    argv: Value,
) -> VmrtRes<(Value, i64)> {
    ctl_require_edit(exec, "defer_current")
        .map_err(|e| ItrErr::new(ItrErrCode::DeferredError, &e.1))?;
    if !argv.is_nil() {
        return itr_err_fmt!(
            ItrErrCode::DeferredError,
            "native ctl 'defer_current' expects no arguments"
        );
    }
    let Some(caddr) = bindings.this_contract else {
        return itr_err_fmt!(
            ItrErrCode::DeferredError,
            "defer_current requires contract context"
        );
    };
    // The binding may have been inherited from a caller in another contract, so the
    // owner check is what keeps a foreign scope out of this contract's registry.
    let Some(id) = intent_state.current_bound_intent_id() else {
        return itr_err_fmt!(
            ItrErrCode::DeferredError,
            "defer_current requires bound intent"
        );
    };
    intents
        .ensure_owner(&caddr, id)
        .map_err(|e| ItrErr::new(ItrErrCode::DeferredError, &e.1))?;
    deferred_registry.register_current(&caddr, Some(Some(id)))?;
    Ok((Value::Nil, NativeCtl::defer_current.gas_of()))
}

macro_rules! intent_std_fn {
    ($name:ident, |$exec:ident, $bindings:ident, $intent_state:ident, $intents:ident, $argv:ident| $body:block) => {
        pub fn $name(
            $exec: ExecCtx,
            $bindings: &FrameBindings,
            $intent_state: &IntentScopeState,
            $intents: &mut IntentRuntime,
            $argv: Value,
        ) -> VmrtRes<(Value, i64)> $body
    };
}

macro_rules! intent_stack_fn {
    ($name:ident, |$exec:ident, $cap:ident, $bindings:ident, $intent_state:ident, $intents:ident, $argv:ident| $body:block) => {
        pub fn $name(
            $exec: ExecCtx,
            $cap: &SpaceCap,
            $bindings: &mut FrameBindings,
            $intent_state: &mut IntentScopeState,
            $intents: &mut IntentRuntime,
            $argv: Value,
        ) -> VmrtRes<(Value, i64)> $body
    };
}

macro_rules! intent_pop_fn {
    ($name:ident, |$exec:ident, $bindings:ident, $intent_state:ident, $argv:ident| $body:block) => {
        pub fn $name(
            $exec: ExecCtx,
            $bindings: &mut FrameBindings,
            $intent_state: &mut IntentScopeState,
            $argv: Value,
        ) -> VmrtRes<(Value, i64)> $body
    };
}

intent_std_fn!(call_intent_new, |exec,
                                 bindings,
                                 _intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_new")?;
    let owner = ctl_contract_owner(bindings, "intent_new")?;
    let id = intents.create(owner, ctl_expect_bytes(&argv, "intent_new", "kind")?)?;
    Ok((Value::handle(IntentId(id)), NativeCtl::intent_new.gas_of()))
});

intent_stack_fn!(call_intent_use, |exec,
                                   cap,
                                   _bindings,
                                   intent_state,
                                   intents,
                                   argv| {
    ctl_require_edit(exec, "intent_use")?;
    if intent_state.len() >= cap.intent_bind_depth {
        return itr_err_fmt!(
            ItrErrCode::IntentError,
            "intent bind depth exceeded max {}",
            cap.intent_bind_depth
        );
    }
    let binding = optional_intent_binding(&argv, intents, "intent_use")?;
    intent_state.push(binding);
    Ok((Value::Nil, NativeCtl::intent_use.gas_of()))
});

// Bind by kind instead of by handle: resolve `kind` among the intents this contract
// owns and push the single match. This is the addressing form a frame can use
// without carrying a handle — either because the entry never had one, or because it
// is a different VM entry than the one that created the intent.
//
// Resolution is exact and fail-closed: no open intent of that kind, or more than one,
// binds nothing. Unlike `intent_use` the owner is required, because `kind` only
// identifies an intent inside one owner's bucket.
intent_stack_fn!(call_intent_use_kind, |exec,
                                        cap,
                                        bindings,
                                        intent_state,
                                        intents,
                                        argv| {
    ctl_require_edit(exec, "intent_use_kind")?;
    let owner = ctl_contract_owner(bindings, "intent_use_kind")?;
    if intent_state.len() >= cap.intent_bind_depth {
        return itr_err_fmt!(
            ItrErrCode::IntentError,
            "intent bind depth exceeded max {}",
            cap.intent_bind_depth
        );
    }
    let kind = ctl_expect_bytes(&argv, "intent_use_kind", "kind")?;
    let ids = intents.open_ids_by_kind(&owner, &kind)?;
    let id = match ids.len() {
        0 => {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent_use_kind found no open intent of this kind"
            )
        }
        1 => ids[0],
        n => {
            return itr_err_fmt!(
                ItrErrCode::IntentError,
                "intent_use_kind kind addresses {} open intents, expected exactly 1",
                n
            )
        }
    };
    intent_state.push(Some(id));
    Ok((Value::Nil, NativeCtl::intent_use_kind.gas_of()))
});

// Whether the current frame is bound to an intent. Frame-local probe: it reads the
// binding stack and touches no intent data, so it needs neither an owner nor a
// bound intent, and it is the only way a Deferred handler can tell a pre-bound
// settlement entry from the unbound global one — the entry's parameter channel is
// fixed by the VM, so the scope itself is the sole discriminator.
intent_std_fn!(call_intent_bound, |exec,
                                    _bindings,
                                    intent_state,
                                    _intents,
                                    argv| {
    ctl_require_non_pure(exec, "intent_bound")?;
    ctl_expect_no_arg(argv, "intent_bound")?;
    Ok((
        Value::Bool(intent_state.current_bound_intent_id().is_some()),
        NativeCtl::intent_bound.gas_of(),
    ))
});

intent_pop_fn!(call_intent_pop, |exec, _bindings, intent_state, argv| {
    ctl_require_edit(exec, "intent_pop")?;
    ctl_expect_no_arg(argv, "intent_pop")?;
    if intent_state.pop().is_none() {
        return itr_err_fmt!(ItrErrCode::IntentError, "intent stack is empty");
    }
    Ok((Value::Nil, NativeCtl::intent_pop.gas_of()))
});

intent_std_fn!(call_intent_put, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_put")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_put")?;
    let (key, val) = ctl_expect_pair(argv, "intent_put")?;
    intents.put(&owner, id, key, val)?;
    Ok((Value::Nil, NativeCtl::intent_put.gas_of()))
});

intent_std_fn!(call_intent_get, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_non_pure(exec, "intent_get")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_get")?;
    Ok((
        intents.get(&owner, id, &argv)?,
        NativeCtl::intent_get.gas_of(),
    ))
});

intent_std_fn!(call_intent_take, |exec,
                                  bindings,
                                  intent_state,
                                  intents,
                                  argv| {
    ctl_require_edit(exec, "intent_take")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_take")?;
    Ok((
        intents.take(&owner, id, &argv)?,
        NativeCtl::intent_take.gas_of(),
    ))
});

intent_std_fn!(call_intent_del, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_del")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_del")?;
    intents.del(&owner, id, &argv)?;
    Ok((Value::Nil, NativeCtl::intent_del.gas_of()))
});

intent_std_fn!(call_intent_has, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_non_pure(exec, "intent_has")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_has")?;
    Ok((
        Value::Bool(intents.has(&owner, id, &argv)?),
        NativeCtl::intent_has.gas_of(),
    ))
});

intent_std_fn!(call_intent_kind, |exec,
                                  bindings,
                                  intent_state,
                                  intents,
                                  argv| {
    ctl_require_non_pure(exec, "intent_kind")?;
    ctl_expect_no_arg(argv, "intent_kind")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_kind")?;
    Ok((intents.kind(&owner, id)?, NativeCtl::intent_kind.gas_of()))
});

intent_std_fn!(
    call_intent_kind_is,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_kind_is")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_kind_is")?;
        let kind = ctl_expect_bytes(&argv, "intent_kind_is", "kind")?;
        Ok((
            Value::Bool(intents.kind_is(&owner, id, &kind)?),
            NativeCtl::intent_kind_is.gas_of(),
        ))
    }
);

intent_std_fn!(call_intent_clear, |exec,
                                   bindings,
                                   intent_state,
                                   intents,
                                   argv| {
    ctl_require_edit(exec, "intent_clear")?;
    ctl_expect_no_arg(argv, "intent_clear")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_clear")?;
    intents.clear_data(&owner, id)?;
    Ok((Value::Nil, NativeCtl::intent_clear.gas_of()))
});

intent_std_fn!(call_intent_len, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_non_pure(exec, "intent_len")?;
    ctl_expect_no_arg(argv, "intent_len")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_len")?;
    Ok((
        Value::U64(intents.len(&owner, id)? as u64),
        NativeCtl::intent_len.gas_of(),
    ))
});

intent_std_fn!(call_intent_keys, |exec,
                                  bindings,
                                  intent_state,
                                  intents,
                                  argv| {
    ctl_require_non_pure(exec, "intent_keys")?;
    ctl_expect_no_arg(argv, "intent_keys")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_keys")?;
    let keys = intents
        .keys_sorted(&owner, id)?
        .into_iter()
        .map(Value::Bytes)
        .collect::<VecDeque<_>>();
    Ok((
        Value::Compo(CompoItem::list(keys)?),
        NativeCtl::intent_keys.gas_of(),
    ))
});

intent_std_fn!(
    call_intent_get_or,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_get_or")?;
        let (key, def) = ctl_expect_pair(argv, "intent_get_or")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_get_or")?;
        Ok((
            intents.get_or(&owner, id, &key, def)?,
            NativeCtl::intent_get_or.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_require,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_require")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_require")?;
        Ok((
            intents.require(&owner, id, &argv)?,
            NativeCtl::intent_require.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_require_eq,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_require_eq")?;
        let (key, expected) = ctl_expect_pair(argv, "intent_require_eq")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_require_eq")?;
        Ok((
            intents.require_eq(&owner, id, &key, &expected)?,
            NativeCtl::intent_require_eq.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_take_or,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_take_or")?;
        let (key, def) = ctl_expect_pair(argv, "intent_take_or")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_take_or")?;
        let val = if intents.has(&owner, id, &key)? {
            intents.take(&owner, id, &key)?
        } else {
            def
        };
        Ok((val, NativeCtl::intent_take_or.gas_of()))
    }
);

intent_std_fn!(
    call_intent_replace,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_replace")?;
        let (key, val) = ctl_expect_pair(argv, "intent_replace")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_replace")?;
        Ok((
            intents.replace(&owner, id, key, val)?,
            NativeCtl::intent_replace.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_destroy,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_destroy")?;
        ctl_expect_no_arg(argv, "intent_destroy")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_destroy")?;
        intents.destroy(&owner, id)?;
        Ok((Value::Nil, NativeCtl::intent_destroy.gas_of()))
    }
);

intent_std_fn!(
    call_intent_put_if_absent,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_put_if_absent")?;
        let (key, val) = ctl_expect_pair(argv, "intent_put_if_absent")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_put_if_absent")?;
        Ok((
            Value::Bool(intents.put_if_absent(&owner, id, key, val)?),
            NativeCtl::intent_put_if_absent.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_replace_if,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_replace_if")?;
        let items = ctl_expect_tuple(argv, "intent_replace_if", 3)?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_replace_if")?;
        Ok((
            Value::Bool(intents.replace_if(
                &owner,
                id,
                items[0].clone(),
                items[1].clone(),
                items[2].clone(),
            )?),
            NativeCtl::intent_replace_if.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_append,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_append")?;
        let (key, val) = ctl_expect_pair(argv, "intent_append")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_append")?;
        Ok((
            Value::U64(intents.append(&owner, id, key, &val)? as u64),
            NativeCtl::intent_append.gas_of(),
        ))
    }
);

intent_std_fn!(call_intent_inc, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_inc")?;
    let (key, delta) = ctl_expect_pair(argv, "intent_inc")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_inc")?;
    Ok((
        intents.inc(&owner, id, key, delta)?,
        NativeCtl::intent_inc.gas_of(),
    ))
});

intent_std_fn!(
    call_intent_del_if,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_del_if")?;
        let (key, expected) = ctl_expect_pair(argv, "intent_del_if")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_del_if")?;
        Ok((
            Value::Bool(intents.del_if(&owner, id, key, expected)?),
            NativeCtl::intent_del_if.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_take_if,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_take_if")?;
        let (key, expected) = ctl_expect_pair(argv, "intent_take_if")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_take_if")?;
        let (hit, val) = intents.take_if(&owner, id, key, expected)?;
        Ok((
            Value::Tuple(TupleItem::new(vec![Value::Bool(hit), val])?),
            NativeCtl::intent_take_if.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_is_own_handle,
    |exec, bindings, _intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_is_own_handle")?;
        let owner = ctl_contract_owner(bindings, "intent_is_own_handle")?;
        let id = extract_intent_handle_id(&argv, "intent_is_own_handle")?;
        if !intents.exists(id) {
            return Ok((Value::Bool(false), NativeCtl::intent_is_own_handle.gas_of()));
        }
        Ok((
            Value::Bool(intents.is_owner(&owner, id)?),
            NativeCtl::intent_is_own_handle.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_destroy_if_empty,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_destroy_if_empty")?;
        ctl_expect_no_arg(argv, "intent_destroy_if_empty")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_destroy_if_empty")?;
        Ok((
            Value::Bool(intents.destroy_if_empty(&owner, id)?),
            NativeCtl::intent_destroy_if_empty.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_keys_page,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_keys_page")?;
        let (cursor_arg, limit_arg) = ctl_expect_pair(argv, "intent_keys_page")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_keys_page")?;
        let cursor = ctl_expect_u32(&cursor_arg, "intent_keys_page", "cursor")? as usize;
        let limit = ctl_expect_u32(&limit_arg, "intent_keys_page", "limit")? as usize;
        let (next, keys) = intents.keys_page(&owner, id, cursor, limit)?;
        let list = Value::Compo(CompoItem::list(
            keys.into_iter().map(Value::Bytes).collect::<VecDeque<_>>(),
        )?);
        let next_cursor = next.map(|v| Value::U32(v as u32)).unwrap_or(Value::Nil);
        Ok((
            Value::Tuple(TupleItem::new(vec![next_cursor, list])?),
            NativeCtl::intent_keys_page.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_keys_after,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_keys_after")?;
        let (after_key, limit_arg) = ctl_expect_pair(argv, "intent_keys_after")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_keys_after")?;
        let limit = ctl_expect_u32(&limit_arg, "intent_keys_after", "limit")? as usize;
        let start = ctl_expect_optional_bytes(&after_key, "intent_keys_after", "after_key")?;
        let (next, keys) = intents.keys_after(&owner, id, start, limit)?;
        let list = Value::Compo(CompoItem::list(
            keys.into_iter().map(Value::Bytes).collect::<VecDeque<_>>(),
        )?);
        let next_key = next.map(Value::Bytes).unwrap_or(Value::Nil);
        Ok((
            Value::Tuple(TupleItem::new(vec![next_key, list])?),
            NativeCtl::intent_keys_after.gas_of(),
        ))
    }
);

// Page over the intents this contract owns, by ascending id, returning their kinds.
//
// Together with `intent_use_kind` this closes the case where a frame cannot name the
// kind it needs: it can enumerate what is open and bind the kind it recognizes. The
// cursor is a position, not a reference — passing back a cursor whose intent is gone
// simply continues after it — so a page stays well defined while the bucket mutates.
// Kinds are returned as stored, duplicates included: two entries with the same kind
// tell the caller that `intent_use_kind` on that kind cannot resolve.
//
// The page is a Compo, which holds at most `compo_length` items, so the requested
// limit is clamped to that before the scan: the returned container is legal by
// construction, rather than because the open-intent cap and the container cap happen
// to agree. A caller asking for a larger page simply gets a shorter one and, when
// more remain, a cursor to continue from.
intent_stack_fn!(
    call_intent_open_page,
    |exec, cap, bindings, _intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_open_page")?;
        let owner = ctl_contract_owner(bindings, "intent_open_page")?;
        let (cursor_arg, limit_arg) = ctl_expect_pair(argv, "intent_open_page")?;
        let limit = (ctl_expect_u32(&limit_arg, "intent_open_page", "limit")? as usize)
            .min(cap.compo_length);
        let after = if cursor_arg.is_nil() {
            None
        } else {
            Some(ctl_expect_u32(&cursor_arg, "intent_open_page", "cursor")? as usize)
        };
        let (next, kinds) = intents.open_kind_page(&owner, after, limit)?;
        let list = Value::Compo(CompoItem::list(
            kinds.into_iter().map(Value::Bytes).collect::<VecDeque<_>>(),
        )?);
        let next_cursor = next.map(|v| Value::U32(v as u32)).unwrap_or(Value::Nil);
        let out = Value::Tuple(TupleItem::new(vec![next_cursor, list])?);
        // Backstop under the clamp: if either cap ever drifts, fail here instead of
        // pushing an over-limit container into the operand stack.
        out.check_container_cap(cap)?;
        Ok((out, NativeCtl::intent_open_page.gas_of()))
    }
);

intent_std_fn!(
    call_intent_require_absent,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_require_absent")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_require_absent")?;
        intents.require_absent(&owner, id, &argv)?;
        Ok((Value::Nil, NativeCtl::intent_require_absent.gas_of()))
    }
);

intent_std_fn!(
    call_intent_require_many,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_require_many")?;
        let keys = ctl_expect_list(argv, "intent_require_many")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_require_many")?;
        let vals = intents
            .require_many(&owner, id, &keys)?
            .into_iter()
            .collect::<VecDeque<_>>();
        Ok((
            Value::Compo(CompoItem::list(vals)?),
            NativeCtl::intent_require_many.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_require_map,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_require_map")?;
        let keys = ctl_expect_list(argv, "intent_require_map")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_require_map")?;
        Ok((
            Value::Compo(CompoItem::map(intents.require_map(&owner, id, &keys)?)?),
            NativeCtl::intent_require_map.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_put_flat_kv,
    |exec, bindings, intent_state, intents, argv| {
        ctl_put_kv_list(
            exec,
            bindings,
            intent_state,
            intents,
            argv,
            "intent_put_flat_kv",
        )?;
        Ok((Value::Nil, NativeCtl::intent_put_flat_kv.gas_of()))
    }
);

intent_std_fn!(
    call_intent_rename,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_rename")?;
        let (from_key, to_key) = ctl_expect_pair(argv, "intent_rename")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_rename")?;
        intents.move_key(&owner, id, from_key, to_key)?;
        Ok((Value::Nil, NativeCtl::intent_rename.gas_of()))
    }
);

intent_std_fn!(call_intent_add, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_add")?;
    let (key, delta) = ctl_expect_pair(argv, "intent_add")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_add")?;
    Ok((
        intents.add(&owner, id, key, delta)?,
        NativeCtl::intent_add.gas_of(),
    ))
});

intent_std_fn!(call_intent_sub, |exec,
                                 bindings,
                                 intent_state,
                                 intents,
                                 argv| {
    ctl_require_edit(exec, "intent_sub")?;
    let (key, delta) = ctl_expect_pair(argv, "intent_sub")?;
    let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_sub")?;
    Ok((
        intents.sub(&owner, id, key, delta)?,
        NativeCtl::intent_sub.gas_of(),
    ))
});

intent_std_fn!(
    call_intent_take_many,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_take_many")?;
        let keys = ctl_expect_list(argv, "intent_take_many")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_take_many")?;
        let vals = intents
            .take_many(&owner, id, &keys)?
            .into_iter()
            .collect::<VecDeque<_>>();
        Ok((
            Value::Compo(CompoItem::list(vals)?),
            NativeCtl::intent_take_many.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_take_map,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_take_map")?;
        let keys = ctl_expect_list(argv, "intent_take_map")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_take_map")?;
        Ok((
            Value::Compo(CompoItem::map(intents.take_map(&owner, id, &keys)?)?),
            NativeCtl::intent_take_map.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_del_many,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_del_many")?;
        let keys = ctl_expect_list(argv, "intent_del_many")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_del_many")?;
        Ok((
            Value::U64(intents.del_many(&owner, id, &keys)? as u64),
            NativeCtl::intent_del_many.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_has_all,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_has_all")?;
        let keys = ctl_expect_list(argv, "intent_has_all")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_has_all")?;
        Ok((
            Value::Bool(intents.has_all(&owner, id, &keys)?),
            NativeCtl::intent_has_all.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_has_any,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_non_pure(exec, "intent_has_any")?;
        let keys = ctl_expect_list(argv, "intent_has_any")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_has_any")?;
        Ok((
            Value::Bool(intents.has_any(&owner, id, &keys)?),
            NativeCtl::intent_has_any.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_consume,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_consume")?;
        let (owner, id) = owned_bound_intent(bindings, intent_state, intents, "intent_consume")?;
        Ok((
            intents.consume(&owner, id, &argv)?,
            NativeCtl::intent_consume.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_consume_many,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_consume_many")?;
        let keys = ctl_expect_list(argv, "intent_consume_many")?;
        let (owner, id) =
            owned_bound_intent(bindings, intent_state, intents, "intent_consume_many")?;
        let vals = intents
            .consume_many(&owner, id, &keys)?
            .into_iter()
            .collect::<VecDeque<_>>();
        Ok((
            Value::Compo(CompoItem::list(vals)?),
            NativeCtl::intent_consume_many.gas_of(),
        ))
    }
);

intent_std_fn!(
    call_intent_put_if_absent_or_match,
    |exec, bindings, intent_state, intents, argv| {
        ctl_require_edit(exec, "intent_put_if_absent_or_match")?;
        let (key, val) = ctl_expect_pair(argv, "intent_put_if_absent_or_match")?;
        let (owner, id) = owned_bound_intent(
            bindings,
            intent_state,
            intents,
            "intent_put_if_absent_or_match",
        )?;
        Ok((
            Value::Bool(intents.put_if_absent_or_match(&owner, id, key, val)?),
            NativeCtl::intent_put_if_absent_or_match.gas_of(),
        ))
    }
);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use field::Address;

    use crate::machine::{DeferredRegistry, IntentRuntime, IntentRuntimeLimits};
    use crate::value::{IntentId, ValueTy};

    use super::*;

    fn addr(seed: u8) -> ContractAddress {
        let mut raw = [0u8; Address::SIZE];
        raw[0] = Address::VERSION_CONTRACT;
        raw[1] = seed;
        ContractAddress::from_addr(Address::from(raw)).unwrap()
    }

    fn bindings(owner: ContractAddress) -> FrameBindings {
        FrameBindings::contract(owner, owner, Arc::from(Vec::<Address>::new()))
    }

    fn cap() -> SpaceCap {
        SpaceCap::new(0)
    }

    fn runtimes() -> (IntentRuntime, SpaceCap) {
        let cap = cap();
        (IntentRuntime::new(IntentRuntimeLimits::from_space_cap(&cap)), cap)
    }

    fn bytes(v: &[u8]) -> Value {
        Value::Bytes(v.to_vec())
    }

    fn handle(id: usize) -> Value {
        Value::handle(IntentId(id))
    }

    fn page_args(cursor: Value, limit: u32) -> Value {
        Value::Tuple(TupleItem::new(vec![cursor, Value::U32(limit)]).unwrap())
    }

    fn page_items(v: &Value) -> (Value, Vec<Vec<u8>>) {
        let Value::Tuple(tuple) = v else {
            panic!("page must return a tuple, got {v:?}");
        };
        let (next, list) = (tuple.as_slice()[0].clone(), &tuple.as_slice()[1]);
        let compo = list.compo_ref().unwrap();
        let kinds = compo
            .list_ref()
            .unwrap()
            .iter()
            .map(|item| item.extract_bytes().unwrap())
            .collect();
        (next, kinds)
    }

    #[test]
    fn catalog_rows_pin_new_ctl_ids() {
        assert_eq!(NativeCtl::intent_use_kind as u8, 65);
        assert_eq!(NativeCtl::intent_bound as u8, 66);
        assert_eq!(NativeCtl::defer_current as u8, 2);
        assert_eq!(NativeCtl::intent_open_page as u8, 68);
        assert_eq!(NativeCtl::argv_len(65), Some(1));
        assert_eq!(NativeCtl::argv_len(66), Some(0));
        assert_eq!(NativeCtl::argv_len(2), Some(0));
        assert_eq!(NativeCtl::argv_len(68), Some(2));
        assert_eq!(NativeCtl::intent_use_kind.rty_of(), ValueTy::Nil);
        assert_eq!(NativeCtl::intent_bound.rty_of(), ValueTy::Bool);
        assert_eq!(NativeCtl::defer_current.rty_of(), ValueTy::Nil);
        assert_eq!(NativeCtl::intent_open_page.rty_of(), ValueTy::Tuple);
        assert_eq!(NativeCtl::from_name("defer_current").map(|v| v.0), Some(2));
        assert!(NativeCtl::has_idx(65) && NativeCtl::has_idx(68));
    }

    #[test]
    fn create_names_at_most_one_open_intent_per_kind() {
        // kind is a per-owner name for one open intent, so a second creation with the
        // same kind is refused; destroying the intent frees the name again.
        let owner = addr(1);
        let (mut intents, _cap) = runtimes();
        let first = intents.create(owner, b"flash".to_vec()).unwrap();
        let err = intents.create(owner, b"flash".to_vec()).unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
        assert!(err.1.contains("already open"), "{}", err.1);
        assert_eq!(
            intents.open_ids_by_kind(&owner, b"flash").unwrap(),
            vec![first]
        );

        // a different contract has its own namespace for the same kind
        let other = addr(2);
        intents.create(other, b"flash".to_vec()).unwrap();

        intents.destroy(&owner, first).unwrap();
        let again = intents.create(owner, b"flash".to_vec()).unwrap();
        assert_ne!(again, first);
    }

    #[test]
    fn use_open_binds_the_single_matching_kind() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        let id = intents.create(owner, b"flash".to_vec()).unwrap();
        intents.create(owner, b"swap".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        let (r, _) = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap();
        assert!(r.is_nil());
        assert_eq!(state.current_bound_intent_id(), Some(id));
    }

    #[test]
    fn use_open_rejects_absent_kind() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        let mut state = IntentScopeState::default();

        let err = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
        assert!(state.current_bound_intent_id().is_none());
    }

    #[test]
    fn use_open_stays_fail_closed_on_a_duplicated_kind() {
        // Unreachable through `create` now that a kind names at most one open intent;
        // kept as the net that catches a future relaxation of that rule.
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        intents
            .create_ignoring_kind_uniqueness(owner, b"flash".to_vec())
            .unwrap();
        intents
            .create_ignoring_kind_uniqueness(owner, b"flash".to_vec())
            .unwrap();
        let mut state = IntentScopeState::default();

        let err = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
        assert!(err.1.contains("addresses 2 open intents"), "{}", err.1);
        assert!(state.current_bound_intent_id().is_none());
    }

    #[test]
    fn use_open_is_scoped_to_the_calling_contract() {
        let mine = addr(1);
        let theirs = addr(2);
        let mut b = bindings(mine);
        let (mut intents, cap) = runtimes();
        intents.create(theirs, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        let err = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
    }

    #[test]
    fn use_open_requires_contract_context_and_edit_mode() {
        let owner = addr(1);
        let (mut intents, cap) = runtimes();
        intents.create(owner, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        // no contract context: root bindings, same lookup must not happen
        let mut root = FrameBindings::root(owner.to_addr(), Arc::from(Vec::<Address>::new()));
        let err = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut root,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);

        // read-only effect: binding is a write-class operation
        let mut b = bindings(owner);
        let err = call_intent_use_kind(
            ExecCtx::view(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
    }

    #[test]
    fn use_open_respects_bind_depth() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        intents.create(owner, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();
        for _ in 0..cap.intent_bind_depth {
            state.push(None);
        }

        let err = call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
        assert!(err.1.contains("bind depth"), "{}", err.1);
    }

    #[test]
    fn intent_bound_reports_the_frame_binding() {
        let owner = addr(1);
        let b = bindings(owner);
        let (mut intents, _cap) = runtimes();
        let mut state = IntentScopeState::default();

        let (r, _) =
            call_intent_bound(ExecCtx::abst(), &b, &state, &mut intents, Value::Nil).unwrap();
        assert_eq!(r, Value::Bool(false));

        // an explicit "no intent" binding is still not a bound intent
        state.push(None);
        let (r, _) =
            call_intent_bound(ExecCtx::abst(), &b, &state, &mut intents, Value::Nil).unwrap();
        assert_eq!(r, Value::Bool(false));

        state.push(Some(7));
        let (r, _) =
            call_intent_bound(ExecCtx::abst(), &b, &state, &mut intents, Value::Nil).unwrap();
        assert_eq!(r, Value::Bool(true));

        let err = call_intent_bound(ExecCtx::pure(), &b, &state, &mut intents, Value::Nil)
            .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
    }

    #[test]
    fn defer_current_registers_the_bound_intent_like_defer_handle() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        let id = intents.create(owner, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();
        let mut registry = DeferredRegistry::new();
        // as `abst_call_raw` does for the entries allowed to register a hook
        registry.replace_defer_auth(Some(owner));

        call_intent_use_kind(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            bytes(b"flash"),
        )
        .unwrap();
        let (r, _) = call_defer_current(
            ExecCtx::abst(),
            &b,
            &state,
            &mut intents,
            &mut registry,
            Value::Nil,
        )
        .unwrap();
        assert!(r.is_nil());

        // same intent through the handle form is the same registry entry
        let err = call_defer(ExecCtx::abst(), &b, &mut intents, &mut registry, handle(id))
            .unwrap_err();
        assert_eq!(err.0, ItrErrCode::DeferredError);
        assert!(err.1.contains("duplicate"), "{}", err.1);

        let drained = registry.drain_lifo();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].addr, owner);
        assert_eq!(drained[0].intent_scope, Some(Some(id)));
    }

    #[test]
    fn defer_current_requires_a_binding_in_the_current_frame() {
        let owner = addr(1);
        let b = bindings(owner);
        let (mut intents, _cap) = runtimes();
        let mut state = IntentScopeState::default();
        let mut registry = DeferredRegistry::new();
        registry.replace_defer_auth(Some(owner));

        // explicitly unbound frame
        state.push(None);
        let err = call_defer_current(
            ExecCtx::abst(),
            &b,
            &state,
            &mut intents,
            &mut registry,
            Value::Nil,
        )
        .unwrap_err();
        assert!(err.1.contains("requires bound intent"), "{}", err.1);

        // read-only effect
        state.reset(Some(Some(3)));
        let err = call_defer_current(
            ExecCtx::view(),
            &b,
            &state,
            &mut intents,
            &mut registry,
            Value::Nil,
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::DeferredError);
    }

    #[test]
    fn defer_current_rejects_an_inherited_foreign_scope() {
        let mine = addr(1);
        let theirs = addr(2);
        let mut b = bindings(mine);
        let (mut intents, cap) = runtimes();
        let foreign = intents.create(theirs, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();
        let mut registry = DeferredRegistry::new();
        registry.replace_defer_auth(Some(mine));

        // `intent_use` only checks that the id exists, so a scope inherited from
        // another contract can be bound; the owner check in `defer_current` is what
        // keeps it out of this contract's registry.
        call_intent_use(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            handle(foreign),
        )
        .unwrap();
        assert_eq!(state.current_bound_intent_id(), Some(foreign));

        let err = call_defer_current(
            ExecCtx::abst(),
            &b,
            &state,
            &mut intents,
            &mut registry,
            Value::Nil,
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::DeferredError);
        assert!(err.1.contains("not owned by contract"), "{}", err.1);
        assert!(registry.drain_lifo().is_empty());
    }

    #[test]
    fn open_page_lists_kinds_by_ascending_id_and_pages() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, cap) = runtimes();
        intents.create(owner, b"k1".to_vec()).unwrap();
        intents.create(owner, b"k2".to_vec()).unwrap();
        let third = intents.create(owner, b"k3".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 2),
        )
        .unwrap();
        let (next, kinds) = page_items(&r);
        assert_eq!(kinds, vec![b"k1".to_vec(), b"k2".to_vec()]);
        assert_eq!(next, Value::U32(2));

        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(next, 2),
        )
        .unwrap();
        let (next, kinds) = page_items(&r);
        assert_eq!(kinds, vec![b"k3".to_vec()]);
        assert!(next.is_nil());

        // a cursor whose intent is gone is just a position
        intents.destroy(&owner, third).unwrap();
        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::U32(2), 8),
        )
        .unwrap();
        let (next, kinds) = page_items(&r);
        assert!(kinds.is_empty());
        assert!(next.is_nil());
    }

    #[test]
    fn open_page_clamps_the_limit_to_the_compo_cap() {
        let owner = addr(1);
        let mut b = bindings(owner);
        let (mut intents, _cap) = runtimes();
        for kind in [b"k1".as_slice(), b"k2", b"k3"] {
            intents.create(owner, kind.to_vec()).unwrap();
        }
        let mut state = IntentScopeState::default();

        // A page is one Compo, so the contract's container cap bounds it whatever the
        // caller asks for: the returned tuple holds exactly `compo_length` kinds and a
        // cursor, never an over-limit list that would break the operand-stack invariant.
        let mut small = cap();
        small.compo_length = 2;
        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &small,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 8),
        )
        .unwrap();
        let (next, kinds) = page_items(&r);
        assert_eq!(kinds, vec![b"k1".to_vec(), b"k2".to_vec()]);
        assert_eq!(next, Value::U32(2));

        // the clamp applies to a caller-supplied limit too, and the page still advances
        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &small,
            &mut b,
            &mut state,
            &mut intents,
            page_args(next, 3),
        )
        .unwrap();
        let (next, kinds) = page_items(&r);
        assert_eq!(kinds, vec![b"k3".to_vec()]);
        assert!(next.is_nil());
    }

    #[test]
    fn open_page_is_owner_scoped_and_rejects_zero_limit() {
        let mine = addr(1);
        let theirs = addr(2);
        let mut b = bindings(mine);
        let (mut intents, cap) = runtimes();
        intents.create(theirs, b"flash".to_vec()).unwrap();
        intents.create(mine, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 8),
        )
        .unwrap();
        let (_, kinds) = page_items(&r);
        assert_eq!(kinds.len(), 1);
        assert_eq!(kinds[0], b"flash".to_vec());

        let err = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 0),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);

        // same-kind duplicates are still reported as duplicates if the invariant is
        // ever relaxed (the state is built past `create`'s check on purpose)
        intents
            .create_ignoring_kind_uniqueness(mine, b"flash".to_vec())
            .unwrap();
        let (r, _) = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut b,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 8),
        )
        .unwrap();
        let (_, kinds) = page_items(&r);
        assert_eq!(kinds, vec![b"flash".to_vec(), b"flash".to_vec()]);
    }

    #[test]
    fn open_page_requires_contract_context() {
        let owner = addr(1);
        let mut root = FrameBindings::root(owner.to_addr(), Arc::from(Vec::<Address>::new()));
        let (mut intents, cap) = runtimes();
        intents.create(owner, b"flash".to_vec()).unwrap();
        let mut state = IntentScopeState::default();

        let err = call_intent_open_page(
            ExecCtx::abst(),
            &cap,
            &mut root,
            &mut state,
            &mut intents,
            page_args(Value::Nil, 8),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::IntentError);
    }
}
