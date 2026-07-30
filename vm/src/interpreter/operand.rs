fn check_call_mode(exec: ExecCtx, call: &CallSpec) -> VmrtErr {
    if let CallSpec::Invoke { target, effect, .. } = *call {
        match (exec.entry, effect, exec.is_outer_entry()) {
            (EntryKind::Abst, _, _) if matches!(target, CallTarget::Ext(_)) => {
                return itr_err_code!(CallInAbst);
            }
            // P2SH allows Edit-mode calls only for Use(lib) without context switch; extending this rule changes P2SH write capabilities and must be reviewed carefully.
            (EntryKind::P2sh, EffectMode::Edit, _) if !matches!(target, CallTarget::Use(_)) => {
                return itr_err_code!(CallOtherInP2sh);
            }
            (EntryKind::Main, EffectMode::Edit, true) if !call.switches_context() => {
                return itr_err_code!(CallOtherInMain);
            }
            _ => {}
        }
    }
    let next = call.callee_effect(exec.effect);
    match (exec.effect, next) {
        (EffectMode::Pure, EffectMode::Pure) => Ok(()),
        (EffectMode::Pure, _) => itr_err_code!(CallInPure),
        (EffectMode::View, EffectMode::Edit) => itr_err_code!(CallLocInView),
        (EffectMode::Edit, _) | (EffectMode::View, EffectMode::View | EffectMode::Pure) => Ok(()),
    }
}

fn local_operand(mark: u8, locals: &mut Stack, mut value: Value) -> VmrtErr {
    // XOP: stack top is rhs; local[idx] from mark is lhs. Result stays in local[idx].
    // Caller charges stack_write on the stored result val_size (see execute.rs XOP).
    let (opt, idx) = decode_local_operand_mark(mark);
    let basev = locals.slot_mut(idx)?;
    match opt {
        LxOp::Add => locop_arithmetic(basev, &mut value, add_checked),
        LxOp::Sub => locop_arithmetic(basev, &mut value, sub_checked),
        LxOp::Mul => locop_arithmetic(basev, &mut value, mul_checked),
        LxOp::Div => locop_arithmetic(basev, &mut value, |x, y| {
            div_checked(x, y, crate::rt::IR_NAME_DIV)
        }),
    }?;
    Ok(())
}

fn local_logic(mark: u8, locals: &mut Stack, value: &mut Value) -> VmrtErr {
    // XLG: stack top is rhs; local[idx] from mark is lhs. Result overwrites rhs on stack.
    let (opt, idx) = decode_local_logic_mark(mark);
    let basev = locals.slot_mut(idx)?;
    let out = match opt {
        LxLg::And => lgc_and(basev, value),
        LxLg::Or => lgc_or(basev, value),
        LxLg::Eq => lgc_equal(basev, value),
        LxLg::Ne => lgc_not_equal(basev, value),
        LxLg::Gt => lgc_greater(basev, value),
        LxLg::Ge => lgc_greater_equal(basev, value),
        LxLg::Lt => lgc_less(basev, value),
        LxLg::Le => lgc_less_equal(basev, value),
    }?;
    *value = out;
    Ok(())
}

fn unpack_seq(
    i: u8,
    locals: &mut Stack,
    items: Vec<Value>,
    gst: &GasExtra,
    cap: &SpaceCap,
) -> VmrtRes<i64> {
    let start = i as usize;
    let end = start.checked_add(items.len()).ok_or_else(|| ItrErr::code(OutOfStack))?;
    if locals.len() < end {
        return itr_err_code!(OutOfStack);
    }
    let mut gas = 0i64;
    for (off, v) in items.into_iter().enumerate() {
        let v = v.valid(cap)?;
        v.check_container_cap(cap)?;
        gas += gst.stack_write(v.val_size());
        let idx = u8::try_from(start + off).map_err(|_| ItrErr::code(OutOfStack))?;
        *locals.slot_mut(idx)? = v;
    }
    Ok(gas)
}
