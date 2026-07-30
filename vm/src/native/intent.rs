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
