/* parse bytecode params */
#[inline(always)]
fn finish_ntcall(
    cap: &SpaceCap,
    gst: &GasExtra,
    step_gas_use: &mut GasBuckets,
    ops: &mut Stack,
    r: Value,
    g: i64,
) -> VmrtRes<()> {
    let r = r.valid(cap)?;
    step_gas_use.resource += gst.nt_bytes(r.val_size());
    step_gas_use.resource += g;
    ops.push(r)?;
    Ok(())
}

macro_rules! itrparam {
    ($codes: expr, $pc: expr, $l: expr, $t: ty) => {{
        let r = $pc + $l;
        //#[cfg(debug_assertions)]
        if r < $pc || r > $codes.len() {
            return itr_err_code!(CodeOverflow);
        }
        let v = <$t>::from_be_bytes($codes[$pc..r].try_into().unwrap());
        $pc = r;
        v
    }};
}

macro_rules! itrparamu8 {
    ($codes: expr, $pc: expr) => {
        itrparam! {$codes, $pc, 1, u8}
    };
}

macro_rules! itrparamu16 {
    ($codes: expr, $pc: expr) => {
        itrparam! {$codes, $pc, 2, u16}
    };
}

macro_rules! peekparam {
    ($codes: expr, $pc: expr, $l: expr, $t: ty) => {{
        let _r = $pc + $l;
        // #[cfg(debug_assertions)]
        if _r < $pc || _r > $codes.len() {
            return itr_err_code!(CodeOverflow);
        }
        <$t>::from_be_bytes($codes[$pc.._r].try_into().unwrap())
    }};
}

macro_rules! peekparamu8 {
    ($codes: expr, $pc: expr) => {
        peekparam! {$codes, $pc, 1, u8}
    };
}

macro_rules! peekparamu16 {
    ($codes: expr, $pc: expr) => {
        peekparam! {$codes, $pc, 2, u16}
    };
}

macro_rules! itrparambufex {
    ($codes: expr, $pc: expr, $l: expr, $t: ty) => {{
        let s = itrparam! {$codes, $pc, $l, $t} as usize;
        let l = $pc;
        let r = l + s;
        if r < l || r > $codes.len() {
            return itr_err_code!(CodeOverflow);
        }
        $pc = r;
        Value::Bytes($codes[l..r].to_vec())
    }};
}

macro_rules! itrparambuf {
    ($codes: expr, $pc: expr) => {
        itrparambufex!($codes, $pc, 1, u8)
    };
}

macro_rules! itrparambufl {
    ($codes: expr, $pc: expr) => {
        itrparambufex!($codes, $pc, 2, u16)
    };
}

macro_rules! jump {
    ($codes: expr, $pc: expr, $l: expr) => {{
        let tpc = match $l {
            1 => peekparamu8!($codes, $pc) as usize,
            2 => peekparamu16!($codes, $pc) as usize,
            _ => return itr_err_code!(CodeOverflow),
        };
        $pc = tpc; // jump to
    }};
}

macro_rules! ostjump {
    ($codes: expr, $pc: expr, $l: expr) => {{
        let tpc = match $l {
            1 => itrparam! {$codes, $pc, 1, i8} as isize,
            2 => itrparam! {$codes, $pc, 2, i16} as isize,
            _ => return itr_err_code!(CodeOverflow),
        };
        let tpc = ($pc as isize + tpc);
        if tpc < 0 {
            return itr_err_code!(CodeOverflow);
        }
        $pc = tpc as usize; // jump to
    }};
}

macro_rules! branch {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr) => {
        if $ops.pop()?.extract_bool()? {
            jump!($codes, $pc, $l);
        } else {
            $pc += $l;
        }
    };
}

macro_rules! ostbranchex {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr, $expect: expr) => {
        if $ops.pop()?.extract_bool()? == $expect {
            ostjump!($codes, $pc, $l);
        } else {
            $pc += $l;
        }
    };
}
// is_not_zero
macro_rules! ostbranch {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr) => {
        ostbranchex!($ops, $codes, $pc, $l, true)
    };
}

// Hot path trusts verified bytecode: only execute rt/verify-accepted code
// with SpaceCap-valid stacks (gas is finalized per instruction).

pub fn execute_code_in_frame<M: VmMachine + ?Sized, H: VmHost + base::Context + ?Sized>(
    // frame local
    pc: &mut usize,
    codes: &[u8],
    exec: ExecCtx,
    operands: &mut Stack,
    locals: &mut Stack,
    heap: &mut Heap,
    bindings: &mut FrameBindings,
    intent_state: &mut crate::frame::IntentScopeState,
    context_addr: &field::Address,
    current_addr: &field::Address,
    // shared VM machine; accessed only for the duration of individual
    // instructions so ACTION can synchronously recurse into NativeVm.
    machine: &mut M,
    host: &mut H,
) -> VmrtRes<CallExit> {
    use Bytecode::*;
    use CallExit::*;
    use ItrErrCode::*;
    use Value::*;

    let gas_table = machine.gas_table();
    let gas_extra = machine.gas_extra();
    let space_cap = machine.space_cap();
    let gst = &gas_extra;
    let cap = &space_cap;
    let kv_limits = crate::space::VolatileKvLimits::from_space_cap(cap);
    let ops = operands;
    let hei: u64 = host.height();

    macro_rules! nsr {
        () => {
            if exec.effect == EffectMode::Pure {
                return itr_err_code!(InstDisabled);
            }
        };
    } // not read in pure mode
    macro_rules! nsw {
        () => {
            if matches!(exec.effect, EffectMode::Pure | EffectMode::View) {
                return itr_err_code!(InstDisabled);
            }
        };
    } // not write in view/pure mode
    macro_rules! pu8 {
        () => {
            itrparamu8!(codes, *pc)
        };
    }
    macro_rules! fin_spec {
        ($inst:expr) => {{
            let id = pu8!();
            fin_spec_lookup($inst, id)?
        }};
    }
    macro_rules! pty {
        () => {
            ops.peek()?.ty()
        };
    }
    macro_rules! ptyn {
        () => {
            ops.peek()?.ty_num()
        };
    }
    macro_rules! pu8_as_u16 {
        () => {
            pu8!() as u16
        };
    }
    macro_rules! pu16 {
        () => {
            itrparamu16!(codes, *pc)
        };
    }
    macro_rules! pbuf {
        () => {
            itrparambuf!(codes, *pc)
        };
    }
    macro_rules! pbufl {
        () => {
            itrparambufl!(codes, *pc)
        };
    }
    macro_rules! ops_pop_to_u16 {
        () => {
            ops.pop()?.extract_u16()?
        };
    }
    macro_rules! ops_pop_to_u32 {
        () => {
            ops.pop()?.extract_u32()?
        };
    }
    macro_rules! ops_peek_to_u16 {
        () => {
            ops.peek()?.extract_u16()?
        };
    }
    macro_rules! check_compo_type {
        ($m: ident) => {
            match ops.compo() {
                Ok(c) => c.$m(),
                _ => false,
            }
        };
    }

    enum Step {
        Continue,
        Exit(CallExit),
    }

    // start run
    loop {
        // read inst
        // #[cfg(debug_assertions)]  // uncomment for debug-only check
        if *pc >= codes.len() {
            return itr_err_code!(CodeOverflow)
        }
        let instbyte = codes[*pc];
        let instruction = Bytecode::try_from_u8(instbyte)?;
        *pc += 1; // next

        // debug_print_stack(ops, locals, pc, instruction);

        // gas
        let base_gas = gas_table.gas(instbyte);
        let mut step_gas_use = GasBuckets {
            compute: base_gas,
            ..GasBuckets::default()
        };
        let mut step_gas_rebate = 0i64;

        macro_rules! gas_add {
            ($kind:ident, raw, $n:expr) => {{
                step_gas_use.$kind += ($n) as i64;
            }};
            ($kind:ident, $f:ident $(, $arg:expr)* $(,)?) => {{
                step_gas_use.$kind += gst.$f($($arg),*);
            }};
        }
        macro_rules! gas_resource {
            ($f:ident $(, $arg:expr)* $(,)?) => {{
                gas_add!(resource, $f $(, $arg)*);
            }};
        }
        macro_rules! gas_resource_raw {
            ($n:expr) => {{
                gas_add!(resource, raw, $n);
            }};
        }
        macro_rules! rebate_add {
            ($n:expr) => {{
                let add = $n;
                step_gas_rebate = step_gas_rebate
                    .checked_add(add)
                    .ok_or_else(|| ItrErr::new(ItrErrCode::GasError, "gas refund overflow"))?;
            }};
        }

        macro_rules! actcall {
            ($act_kind: expr) => {{
                let act_kind = $act_kind;
                let idx = pu8!();
                let opcode_abi = host_opcode_abi(act_kind);
                ensure_act_allowed(host, exec, act_kind, idx)?;
                let kid = u16::from_be_bytes([instbyte, idx]);
                let mut actbody = vec![];
                if opcode_abi.consumes_body {
                    let mut bdv = ops.peek()?.extract_call_data()?;
                    actbody.append(&mut bdv);
                    gas_resource!(act_bytes, actbody.len());
                }
                // ACTION transfers drive Permit/Payable hooks synchronously after the ledger move
                // (mirroring dev `action.execute` then `do_action_hook`); the VM recurses here.
                let routing = if matches!(act_kind, Bytecode::ACTION) {
                    host.action_transfer_routing(kid, &actbody).map_err(|e| {
                        ItrErr::new(crate::rt::map_native_action_code(&e), &e.to_string())
                    })?
                } else {
                    None
                };
                let (bgasu, cres) = VmHost::action_call(host, kid, actbody).map_err(|e| {
                    ItrErr::new(
                        if e.is_revert() {
                            ActCallRevert
                        } else {
                            crate::rt::map_native_action_code(&e)
                        },
                        e.as_str(),
                    )
                })?;
                if let Some(r) = routing {
                    let intent_scope = intent_state.current_scope();
                    machine.drive_transfer(host.as_context_mut(), r, intent_scope)?;
                }
                gas_resource_raw!(bgasu);
                if opcode_abi.produces_value {
                    let resv =
                        Value::type_from(act_retv_type(host, act_kind, idx)?, cres)?.valid(cap)?;
                    gas_resource!(act_bytes, resv.val_size());
                    if opcode_abi.consumes_body {
                        *ops.peek()? = resv;
                    } else {
                        ops.push(resv)?;
                    }
                } else {
                    ops.pop()?;
                }
            }};
        }

        macro_rules! local_get {
            ($idx: expr) => {{
                let v = locals.load($idx as usize)?.valid(cap)?;
                gas_resource!(stack_copy, v.dup_size());
                ops.push(v)?;
            }};
        }

        macro_rules! wlog {
            ($itn: expr) => {{
                nsw!();
                let items = ops.popn($itn)?;
                let log_bytes: usize = items.iter().map(|v| v.val_size()).sum();
                if cap.log_size == 0 {
                    return itr_err_fmt!(OutOfLogSize, "logging disabled");
                }
                let current_total = machine.log_bytes_total();
                let next_total = current_total
                    .checked_add(log_bytes)
                    .ok_or_else(|| ItrErr::new(
                        OutOfLogSize,
                        "log bytes overflow",
                    ))?;
                if next_total > cap.log_size {
                    return itr_err_fmt!(
                        OutOfLogSize,
                        "log bytes limit exceeded: {} > {}",
                        next_total,
                        cap.log_size
                    );
                }
                machine.commit_log_bytes(next_total);
                gas_add!(storage, log_bytes, log_bytes);
                host.log_push(context_addr, items)?;
            }};
        }

        macro_rules! peek_op_gas { ($method:ident($($arg:expr),*)) => {{
            let (v, outlen) = ops.peek_with_size()?;
            gas_resource!(stack_op, outlen);
            v.$method($($arg),*)?;
        }}}

        macro_rules! compare_gas {
            () => {{
                if let (Some(a), Some(b)) = (ops.tail(1), ops.tail(0)) {
                    gas_resource!(stack_cmp, lgc_compare_fee(a, b, gst));
                }
            }};
        }

        macro_rules! push_buf_gas {
            ($v:expr) => {{
                let v = $v.valid(cap)?;
                if let Some(b) = v.match_bytes() {
                    gas_resource!(stack_copy, b.len());
                }
                ops.push(v)?;
            }};
        }

        macro_rules! hwrite {
            ($idx:expr) => {{
                let v = ops.pop()?;
                gas_resource!(heap_write, v.val_size());
                heap.write($idx, v)?;
            }};
        }

        macro_rules! hread_push {
            ($v:expr) => {{
                let v = $v;
                gas_resource!(heap_read, v.val_size());
                ops.push(v)?;
            }};
        }

        macro_rules! kvput_inner {
            ($store:expr, $key_cost:expr, $ec:expr) => {{
                let v = ops.pop()?;
                let k = ops.pop()?;
                crate::space::validate_volatile_kv_put(&k, &v, &kv_limits, true, $ec)?;
                let klen = $store.key_len(&k)?;
                gas_resource!(stack_write, klen);
                if matches!(v, Value::Nil) {
                    $store.remove(&k)?;
                } else {
                    let vlen = v.val_size();
                    let is_new = !$store.contains_key(&k)?;
                    gas_resource!(stack_write, vlen);
                    if is_new {
                        gas_resource_raw!($key_cost);
                    }
                    $store.put(k, v)?;
                }
            }};
        }

        macro_rules! kvput {
            ($store:expr, $key_cost:expr, $ec:expr) => {{
                nsw!();
                kvput_inner!($store, $key_cost, $ec);
            }};
        }

        macro_rules! kvget {
            ($k:ident => $lookup:expr) => {{
                nsr!();
                let v = {
                    let $k = ops.peek()?;
                    $lookup
                }
                .valid(cap)?;
                gas_resource!(stack_copy, v.dup_size());
                *ops.peek()? = v;
            }};
        }

        macro_rules! compo_edit_gas {
            () => {{
                let len = ops.compo()?.len();
                gas_resource!(compo_items_edit, len);
            }};
        }

        macro_rules! compo_read_gas {
            () => {{
                let len = ops.peek()?.container_len()?;
                gas_resource!(compo_items_read, len);
            }};
        }

        macro_rules! compo_pop_one {
            ($method:ident) => {{
                let mut compo_val = ops.pop()?;
                let len = compo_val.compo()?.len();
                let v = compo_val.compo()?.$method()?.valid(cap)?;
                gas_resource!(compo_items_edit, len);
                gas_resource!(compo_bytes, v.val_size());
                ops.push(v)?;
            }};
        }

        let step: VmrtRes<Step> = (|| {
            match instruction {
                // action
                ACTION | ACTENV | ACTVIEW => actcall!(instruction),
                // native runtime control (VM-local side effects)
                NTCTL => {
                    let nt_idx = pu8!();
                    let argv = ops.pop()?.valid(cap)?;
                    let (r, g) = machine.call_ntctl(
                        exec,
                        cap,
                        bindings,
                        intent_state,
                        context_addr,
                        nt_idx,
                        argv,
                    )?;
                    finish_ntcall(cap, gst, &mut step_gas_use, ops, r, g)?;
                }
                // native func (pure computation, always allowed)
                NTFUNC => {
                    let nt_idx = pu8!();
                    let argv = ops.pop()?.extract_call_data()?;
                    gas_resource!(nt_bytes, argv.len());
                    let (r, g) = call_ntfunc(hei, nt_idx, &argv)?;
                    finish_ntcall(cap, gst, &mut step_gas_use, ops, r, g)?;
                }
                // native env (VM context read, forbidden in Pure mode)
                NTENV => {
                    let nt_idx = pu8!();
                    let (r, g) = call_ntenv(exec, bindings, context_addr, nt_idx)?;
                    finish_ntcall(cap, gst, &mut step_gas_use, ops, r, g)?;
                }
                // constant
                PU8 => ops.push(U8(pu8!()))?,
                PU16 => ops.push(U16(pu16!()))?,
                PBUF => push_buf_gas!(pbuf!()),
                PBUFL => push_buf_gas!(pbufl!()),
                P0 | P1 | P2 | P3 => ops.push(U8(instbyte - P0 as u8))?,
                PNBUF => ops.push(Value::empty_bytes())?,
                PNIL => ops.push(Value::Nil)?,
                PTRUE => ops.push(Bool(true))?,
                PFALSE => ops.push(Bool(false))?,
                // cast & type
                CU8 => ops.peek()?.cast_u8()?,
                CU16 => ops.peek()?.cast_u16()?,
                CU32 => ops.peek()?.cast_u32()?,
                CU64 => ops.peek()?.cast_u64()?,
                CU128 => ops.peek()?.cast_u128()?,
                CBYTES => ops.peek()?.cast_bytes()?,
                CTO => {
                    let ty = parse_cto_target_ty_param(pu8!())?;
                    ops.peek()?.cast_to(ty as u8)?;
                }
                TNIL => *ops.peek()? = Bool(pty!() == ValueTy::Nil),
                TLIST => *ops.peek()? = Bool(check_compo_type!(is_list)),
                TMAP => *ops.peek()? = Bool(check_compo_type!(is_map)),
                TIS => {
                    let ty = parse_value_ty_param(pu8!())?;
                    *ops.peek()? = Bool(pty!() == ty);
                }
                TID => *ops.peek()? = U8(ptyn!()),
                // stack & buffer
                DUP => {
                    let bsz = ops.datas.last().map(Value::dup_size).unwrap_or(0);
                    gas_resource!(stack_copy, bsz);
                    ops.push(ops.last()?)?;
                }
                DUPN => {
                    let n = pu8!();
                    let m = ops.datas.len();
                    let nsz = n as usize;
                    let mut bsz = 0usize;
                    if nsz <= m {
                        for v in &ops.datas[m - nsz..m] {
                            bsz += v.dup_size();
                        }
                    }
                    gas_resource!(stack_copy, bsz);
                    ops.dupn(n)?;
                }
                POP => {
                    ops.pop()?;
                } // drop
                POPN => {
                    let n = pu8!();
                    gas_resource_raw!(gst.stack_move_items(n as usize));
                    ops.popn(n)?;
                }
                ROLL0 => {
                    gas_resource_raw!(gst.stack_move_items(1));
                    ops.roll(0)?;
                }
                ROLL => {
                    let n = pu8!();
                    gas_resource_raw!(gst.stack_move_items(n as usize + 1));
                    ops.roll(n)?;
                }
                SWAP => {
                    gas_resource_raw!(gst.stack_move_items(2));
                    ops.swap()?;
                }
                REV => {
                    let n = pu8!();
                    gas_resource_raw!(gst.stack_move_items(n as usize));
                    ops.reverse(n)?;
                } // reverse
                // CHOOSE expects stack order [..., cond, yes, no] and keeps
                // exactly one chosen branch value on stack top.
                CHOOSE => {
                    let l = ops.datas.len();
                    if l < 3 {
                        return itr_err_code!(OutOfStack);
                    }
                    let c = l - 3;
                    let y = c + 1;
                    let n = c + 2;
                    let pick = maybe!(ops.datas[c].extract_bool()?, y, n);
                    gas_resource_raw!(gst.stack_move_items(2));
                    ops.datas.swap(c, pick);
                    ops.datas.truncate(l - 2);
                } /* choose(cond, yes, no) */
                CAT => {
                    gas_resource!(stack_op, ops.tail_val_size(1)? + ops.tail_val_size(0)?);
                    ops.cat(cap)?;
                }
                JOIN => {
                    let n = pu8!();
                    gas_resource!(stack_op, ops.tail_val_size_sum(n as usize)?);
                    ops.join(n, cap)?;
                }
                // BYTE/CUT: buf at stack bottom (subx); args toward top. Matches buf_cut(buf, start, len).
                BYTE => {
                    gas_resource!(stack_op, ops.tail_val_size(1)?);
                    let i = ops_pop_to_u16!();
                    ops.peek()?.cutbyte(i)?;
                }
                CUT => {
                    let (l, o) = (ops.pop()?, ops.pop()?); // pop len, ost (top first)
                    gas_resource!(stack_op, ops.tail_val_size(0)?);
                    ops.peek()?.cutout(l, o)?; // peek buf (bottom)
                }
                LEFT => peek_op_gas!(cutleft(pu8_as_u16!())),
                RIGHT => peek_op_gas!(cutright(pu8_as_u16!())),
                LDROP => peek_op_gas!(dropleft(pu8_as_u16!())),
                RDROP => peek_op_gas!(dropright(pu8_as_u16!())),
                SIZE => *ops.peek()? = U16(ops.peek()?.can_get_size()?),
                // compo
                NEWLIST => ops.push(Compo(CompoItem::new_list()))?,
                NEWMAP => ops.push(Compo(CompoItem::new_map()))?,
                PACKTUPLE => {
                    let (a, len) = TupleItem::pack(cap, ops)?;
                    gas_resource!(compo_items_edit, len);
                    ops.push(a)?;
                }
                TUPLE2LIST => {
                    let (list, len, bsz) = match ops.peek()? {
                        Tuple(args) => match args.to_list_with_stats(cap) {
                            Ok(v) => v,
                            Err(ItrErr(CastBeValueFail, msg)) => {
                                return Err(ItrErr::new(CastFail, &msg));
                            }
                            Err(e) => return Err(e),
                        },
                        _ => return itr_err_code!(CastFail),
                    };
                    gas_resource!(compo_items_copy, len);
                    gas_resource!(compo_bytes, bsz);
                    *ops.peek()? = list;
                }
                PACKLIST => {
                    let (l, len) = CompoItem::pack_list(cap, ops)?;
                    gas_resource!(compo_items_edit, len);
                    ops.push(l)?;
                }
                PACKMAP => {
                    let (m, len) = CompoItem::pack_map(cap, ops)?;
                    gas_resource!(compo_items_edit, len);
                    ops.push(m)?;
                }
                INSERT => {
                    let v = ops.pop()?;
                    let k = ops.pop()?;
                    let ksz = {
                        let c = ops.compo()?;
                        maybe!(c.is_map(), k.extract_key_bytes()?.len(), 0)
                    };
                    compo_edit_gas!();
                    gas_resource!(compo_bytes, ksz);
                    gas_resource!(compo_bytes, v.val_size());
                    ops.compo()?.insert(cap, k, v)?;
                }
                REMOVE => {
                    let k = ops.pop()?;
                    compo_edit_gas!();
                    ops.compo()?.remove(k)?;
                }
                CLEAR => {
                    let (len, bsz) = {
                        let c = ops.compo()?;
                        (c.len(), c.val_size())
                    };
                    gas_resource!(compo_items_edit, len);
                    gas_resource!(compo_bytes, bsz);
                    ops.compo()?.clear();
                }
                MERGE => {
                    let a = ops.pop()?;
                    let (src_len, src_bsz) = match a.match_compo() {
                        Some(c) => {
                            let len = c.len();
                            let bsz = match c.list_ref() {
                                Ok(l) => l
                                    .iter()
                                    .fold(0usize, |acc, v| add_size_saturating(acc, v.val_size())),
                                Err(_) => c.map_ref()?.iter().fold(0usize, |acc, (k, v)| {
                                    let acc = add_size_saturating(acc, k.len());
                                    add_size_saturating(acc, v.val_size())
                                }),
                            };
                            (len, bsz)
                        }
                        None => (0, 0),
                    };
                    gas_resource!(compo_items_copy, src_len);
                    gas_resource!(compo_bytes, src_bsz);
                    ops.compo()?.merge(cap, a.take_compo()?)?;
                }
                LENGTH => {
                    let l = ops.peek()?.length(cap)?;
                    *ops.peek()? = l;
                }
                HASKEY => {
                    let k = ops.pop()?;
                    let h = ops.peek()?.haskey(k)?;
                    compo_read_gas!();
                    *ops.peek()? = h;
                }
                ITEMGET => {
                    let k = ops.pop()?;
                    let v = ops.peek()?.itemget(k)?.valid(cap)?;
                    compo_read_gas!();
                    gas_resource!(compo_bytes, v.val_size());
                    *ops.peek()? = v;
                }
                KEYS => {
                    let (v, len, bsz) = ops.compo()?.keys_with_stats()?;
                    gas_resource!(compo_items_read, len);
                    gas_resource!(compo_bytes, bsz);
                    *ops.peek()? = v;
                }
                VALUES => {
                    let (v, len, bsz) = ops.compo()?.values_with_stats()?;
                    gas_resource!(compo_items_read, len);
                    gas_resource!(compo_bytes, bsz);
                    *ops.peek()? = v;
                }
                TAKEFIRST => compo_pop_one!(take_first),
                TAKELAST => compo_pop_one!(take_last),
                APPEND => {
                    let v = ops.pop()?;
                    compo_edit_gas!();
                    gas_resource!(compo_bytes, v.val_size());
                    ops.compo()?.append(cap, v)?;
                }
                CLONE => {
                    let (len, bsz, c) = {
                        let compo = ops.compo()?;
                        let len = compo.len();
                        let bsz = match compo.list_ref() {
                            Ok(l) => l
                                .iter()
                                .fold(0usize, |acc, v| add_size_saturating(acc, v.val_size())),
                            Err(_) => compo.map_ref()?.iter().fold(0usize, |acc, (k, v)| {
                                let acc = add_size_saturating(acc, k.len());
                                add_size_saturating(acc, v.val_size())
                            }),
                        };
                        (len, bsz, compo.copy())
                    };
                    gas_resource!(compo_items_copy, len);
                    gas_resource!(compo_bytes, bsz);
                    *ops.peek()? = Compo(c);
                }
                UNPACK => {
                    let i = ops.pop()?.extract_u8()?;
                    let len = ops.peek()?.container_len()?;
                    gas_resource!(compo_items_read, len);
                    let items = ops.peek()?.clone_unpack_items()?;
                    gas_resource_raw!(unpack_seq(i, locals, items, gst, cap)?);
                    ops.pop()?; // pop argv wrapper after unpack
                }
                // locals, logs, heap, global_map & memory_map
                // XLG: one stack operand (rhs); lhs is locals[idx]. Semantics: local op rhs.
                XLG => {
                    let mark = pu8!();
                    let (opt, idx) = decode_local_logic_mark(mark);
                    if matches!(opt, LxLg::Eq | LxLg::Ne) {
                        let base = locals.load(idx as usize)?;
                        let top = ops.peek()?;
                        gas_resource!(stack_cmp, lgc_compare_fee(&base, top, gst));
                    }
                    local_logic(mark, locals, ops.peek()?)?;
                }
                XOP => {
                    let mark = pu8!();
                    let rhs = ops.pop()?;
                    local_operand(mark, locals, rhs)?;
                    let (_, idx) = decode_local_operand_mark(mark);
                    let vlen = locals.load(idx as usize)?.val_size();
                    gas_resource!(stack_write, vlen);
                }
                GET => local_get!(pu8!()),
                PUT => {
                    let v = ops.pop()?.valid(cap)?;
                    let vlen = v.val_size();
                    gas_resource!(stack_write, vlen);
                    locals.save(pu8_as_u16!(), v)?;
                }
                GETX => {
                    let v = locals.load(ops_peek_to_u16!() as usize)?.valid(cap)?;
                    gas_resource!(stack_copy, v.dup_size());
                    *ops.peek()? = v;
                }
                PUTX => {
                    let v = ops.pop()?.valid(cap)?;
                    let vlen = v.val_size();
                    gas_resource!(stack_write, vlen);
                    locals.save(ops_pop_to_u16!(), v)?;
                }
                ALLOC => gas_resource_raw!(gst.one_local_alloc * locals.alloc(pu8!())? as i64),
                GET0 | GET1 | GET2 | GET3 => local_get!(instbyte - GET0 as u8),
                LOG1 | LOG2 | LOG3 | LOG4 => wlog!(instbyte - LOG1 as u8 + 2),
                HREADUL => hread_push!(heap.read_ul(pu16!())?),
                HREADU => hread_push!(heap.read_u(pu8!())?),
                HWRITEXL => hwrite!(pu16!() as usize),
                HWRITEX => hwrite!(pu8!() as usize),
                HREAD => {
                    let n = ops.pop()?;
                    let len = n.extract_u16()? as usize;
                    gas_resource!(heap_read, len);
                    let peek = ops.peek()?;
                    *peek = heap.read(peek, n)?.valid(cap)?;
                }
                HWRITE => hwrite!(ops_pop_to_u32!() as usize),
                HGROW => gas_resource_raw!(heap.grow(pu8!(), gst)?),
                // storage
                SSTAT => {
                    nsr!();
                    // SSTAT returns only a fixed-size status tuple, so it does not add SLOAD's value-return storage_read(vlen) surcharge here.
                    let v = {
                        let k = ops.peek()?;
                        host.sstat(gst, cap, context_addr, k)?
                    }
                    .valid(cap)?;
                    *ops.peek()? = v;
                }
                SLOAD => {
                    nsr!();
                    let v = {
                        let k = ops.peek()?;
                        host.sload(gst, cap, context_addr, k)?
                    }
                    .valid(cap)?;
                    if !matches!(v, Value::Nil) {
                        let vlen = v.val_size();
                        gas_add!(storage, storage_read, vlen);
                    }
                    *ops.peek()? = v;
                }
                SGET => {
                    nsr!();
                    let v = {
                        let k = ops.peek()?;
                        host.sget(gst, cap, context_addr, k)?
                    }
                    .valid(cap)?;
                    gas_add!(storage, raw, host.sget_gas(gst, &v));
                    *ops.peek()? = v;
                }
                SEDIT => {
                    nsw!();
                    let v = ops.pop()?.valid(cap)?;
                    let k = ops.pop()?;
                    let (fee, rebate) = host.sedit(gst, cap, context_addr, k, v)?;
                    gas_add!(storage, raw, fee);
                    rebate_add!(rebate);
                }
                SDEL => {
                    nsw!();
                    let k = ops.pop()?;
                    let refund = host.sdel(gst, cap, context_addr, k)?;
                    rebate_add!(refund);
                }
                SNEW => {
                    nsw!();
                    let t = ops.pop()?;
                    let v = ops.pop()?.valid(cap)?;
                    let k = ops.pop()?;
                    gas_add!(storage, raw, host.snew(gst, cap, context_addr, k, v, t)?);
                }
                SRECV => {
                    nsw!();
                    let t = ops.pop()?;
                    let k = ops.pop()?;
                    gas_add!(storage, raw, host.srecv(gst, cap, context_addr, k, t)?);
                }
                SRENT => {
                    nsw!();
                    let t = ops.pop()?;
                    let k = ops.pop()?;
                    gas_add!(storage, raw, host.srent(gst, cap, context_addr, k, t)?);
                }
                SPUT => {
                    nsw!();
                    let v = ops.pop()?;
                    let k = ops.pop()?;
                    // Gas uses the same key/value checks as sput (see `status_put_prepare`); must run before charging.
                    gas_add!(storage, raw, host.sput_gas(gst, cap, &k, &v)?);
                    host.sput(gst, cap, context_addr, k, v)?;
                }
                // global_map & memory_map
                GPUT => {
                    let map = machine.global_map_mut();
                    kvput!(map, gst.global_key_cost, GlobalError)
                }
                GGET => {
                    let map = machine.global_map_mut();
                    kvget!(k => map.get(k)?)
                }
                MPUT => {
                    nsw!();
                    let v = ops.pop()?;
                    let k = ops.pop()?;
                    crate::space::validate_volatile_kv_put(&k, &v, &kv_limits, true, MemoryError)?;
                    let klen = k.extract_key_bytes_with_error_code(MemoryError)?.len();
                    gas_resource!(stack_write, klen);
                    let memory_map = machine.memory_map_mut();
                    if matches!(v, Value::Nil) {
                        memory_map.remove(context_addr, &k)?;
                    } else {
                        let vlen = v.val_size();
                        let is_new = match memory_map.entry(context_addr)? {
                            Some(mem) => !mem.contains_key(&k)?,
                            None => true,
                        };
                        gas_resource!(stack_write, vlen);
                        if is_new {
                            gas_resource_raw!(gst.memory_key_cost);
                        }
                        memory_map.entry_mut(context_addr)?.put(k, v)?;
                    }
                }
                MGET => {
                    let memory_map = machine.memory_map_mut();
                    kvget!(k => memory_map.get(context_addr, k)?)
                }
                MTAKE => {
                    nsw!();
                    let k = ops.peek()?.clone();
                    let memory_map = machine.memory_map_mut();
                    let v = memory_map.get(context_addr, &k)?.valid(cap)?;
                    if matches!(v, Value::Nil) {
                        return itr_err_fmt!(MemoryError, "memory take value not found");
                    }
                    gas_resource!(stack_copy, v.dup_size());
                    memory_map.remove(context_addr, &k)?;
                    *ops.peek()? = v;
                }
                // logic
                AND => binop_btw(ops, lgc_and)?,
                OR => binop_btw(ops, lgc_or)?,
                EQ => {
                    compare_gas!();
                    binop_btw(ops, lgc_equal)?
                }
                NEQ => {
                    compare_gas!();
                    binop_btw(ops, lgc_not_equal)?
                }
                LT => binop_btw(ops, lgc_less)?,
                GT => binop_btw(ops, lgc_greater)?,
                LE => binop_btw(ops, lgc_less_equal)?,
                GE => binop_btw(ops, lgc_greater_equal)?,
                NOT => ops.peek()?.cast_bool_not()?,
                // bitop
                BSHR => binop_arithmetic(ops, bit_shr)?,
                BSHL => binop_arithmetic(ops, bit_shl)?,
                BXOR => binop_arithmetic(ops, bit_xor)?,
                BOR => binop_arithmetic(ops, bit_or)?,
                BAND => binop_arithmetic(ops, bit_and)?,
                // arithmetic
                ADD => binop_arithmetic(ops, add_checked)?,
                SUB => binop_arithmetic(ops, sub_checked)?,
                MUL => binop_arithmetic(ops, mul_checked)?,
                DIV => binop_arithmetic(ops, |x, y| div_checked(x, y, crate::rt::IR_NAME_DIV))?,
                DIVUP => binop_arithmetic(ops, div_up_checked)?,
                DIVEXACT => binop_arithmetic(ops, div_exact_op_checked)?,
                MOD => binop_arithmetic(ops, mod_checked)?,
                POW => binop_arithmetic(ops, pow_checked)?,
                MAX => binop_arithmetic(ops, max_checked)?,
                MIN => binop_arithmetic(ops, min_checked)?,
                FIN2 => {
                    let spec = fin_spec!(Bytecode::FIN2);
                    binop_arithmetic(ops, |x, y| fin2_checked(spec, x, y))?
                }
                FIN3 => {
                    let spec = fin_spec!(Bytecode::FIN3);
                    triop_arithmetic(ops, |x, y, z| fin3_checked(spec, x, y, z))?
                }
                ABSDIFF => binop_arithmetic(ops, absdiff_checked)?,
                ADDMOD => triop_arithmetic(ops, addmod_checked)?,
                MULMOD => triop_arithmetic(ops, mulmod_checked)?,
                MULDIV => triop_arithmetic(ops, |x, y, z| {
                    muldiv_checked(x, y, z, crate::rt::IR_NAME_MUL_DIV)
                })?,
                MULDIVUP => triop_arithmetic(ops, muldiv_up_checked)?,
                MULADD => triop_arithmetic(ops, muladd_checked)?,
                MULSUB => triop_arithmetic(ops, mulsub_checked)?,
                FINPOW3 => {
                    let spec = fin_spec!(Bytecode::FINPOW3);
                    let exp_bits = if ops.len() >= 3 {
                        let idx = ops.len() - 2;
                        match ops
                            .datas
                            .get(idx)
                            .filter(|v| v.is_uint())
                            .and_then(|v| v.extract_u128().ok())
                        {
                            Some(exp) if exp > 0 => (u128::BITS - exp.leading_zeros()) as i64,
                            _ => 0,
                        }
                    } else {
                        0
                    };
                    gas_add!(compute, raw, gst.rpow_extra(exp_bits));
                    triop_arithmetic(ops, |x, y, z| finpow3_checked(spec, x, y, z))?
                }
                CLAMP => triop_arithmetic(ops, clamp_checked)?,
                FIN4 => {
                    let spec = fin_spec!(Bytecode::FIN4);
                    quadop_arithmetic(ops, |x, y, z, w| fin4_checked(spec, x, y, z, w))?
                }
                FINP3 => {
                    let spec = fin_spec!(Bytecode::FINP3);
                    triop_arithmetic(ops, |x, y, z| finp3_checked(spec, x, y, z))?
                }
                FINP4 => {
                    let spec = fin_spec!(Bytecode::FINP4);
                    quadop_arithmetic(ops, |x, y, z, w| finp4_checked(spec, x, y, z, w))?
                }
                INC => unary_inc(ops.peek()?, pu8!())?,
                DEC => unary_dec(ops.peek()?, pu8!())?,
                SQRT => {
                    let v = {
                        let x = ops.peek()?;
                        sqrt_floor_checked(x)?
                    };
                    *ops.peek()? = v;
                }
                SQRTUP => {
                    let v = {
                        let x = ops.peek()?;
                        sqrt_up_checked(x)?
                    };
                    *ops.peek()? = v;
                }
                // workflow control
                JMPL => jump!(codes, *pc, 2),
                JMPS => ostjump!(codes, *pc, 1),
                JMPSL => ostjump!(codes, *pc, 2),
                BRL => branch!(ops, codes, *pc, 2),
                BRS => ostbranch!(ops, codes, *pc, 1),
                BRSL => ostbranch!(ops, codes, *pc, 2),
                BRSLN => ostbranchex!(ops, codes, *pc, 2, false),
                // other
                NT => return itr_err_code!(InstNeverTouch), // never touch
                NOP => {}                                   // do nothing
                BURN => gas_resource_raw!(gst.burn_extra(pu16!() as i64)),
                // exit
                RET => return Ok(Step::Exit(Return)), // func return <DATA>
                END => return Ok(Step::Exit(Finish)), // func end
                ERR => return Ok(Step::Exit(Throw)),  // throw <ERROR>
                ABT => return Ok(Step::Exit(Abort)),  // panic
                AST => {
                    if !ops.pop()?.extract_bool()? {
                        return Ok(Step::Exit(Abort));
                    }
                } // assert(..)
                PRT => debug_print_value(context_addr, current_addr, exec, ops.pop()?),
                // call
                CODECALL | CALL | CALLEXT | CALLEXTVIEW | CALLUSEVIEW | CALLUSEPURE | CALLTHIS
                | CALLSELF | CALLSUPER | CALLSELFVIEW | CALLSELFPURE => {
                    let plen = instruction.metadata().param as usize;
                    let end = *pc + plen;
                    if end > codes.len() {
                        return itr_err_code!(CodeOverflow);
                    }
                    let call = decode_user_call_site(instruction, &codes[*pc..end])?;
                    *pc = end;
                    check_call_mode(exec, &call)?;
                    return Ok(Step::Exit(Call(call)));
                }
                // inst invalid
                _ => return itr_err_fmt!(InstInvalid, "{}", instbyte),
            }
            Ok(Step::Continue)
        })();

        // reduce gas for use: charge protocol first, then commit VM bucket
        let current_gas_use = machine.gas_use();
        let (step_total, next_gas_use) =
            check_add_gas_use(&current_gas_use, &step_gas_use, gst)?;
        VmHost::gas_charge(host, step_total)?;
        VmHost::gas_rebate(host, step_gas_rebate)?;
        machine.commit_gas_use(next_gas_use);
        match step {
            Ok(Step::Exit(exit)) => return Ok(exit),
            Ok(Step::Continue) => {}
            Err(e) => return Err(e),
        }
        // next
    }
}

#[inline(always)]
fn check_add_gas_use(
    gas_use: &GasBuckets,
    step_gas_use: &GasBuckets,
    gst: &GasExtra,
) -> VmrtRes<(i64, GasBuckets)> {
    if step_gas_use.compute < 0 || step_gas_use.resource < 0 || step_gas_use.storage < 0 {
        return itr_err_fmt!(
            ItrErrCode::GasError,
            "gas cost invalid: compute={}, resource={}, storage={}",
            step_gas_use.compute,
            step_gas_use.resource,
            step_gas_use.storage
        );
    }
    let step_total = step_gas_use
        .compute
        .checked_add(step_gas_use.resource)
        .and_then(|v| v.checked_add(step_gas_use.storage))
        .ok_or_else(|| ItrErr::new(ItrErrCode::OutOfGas, "gas overflow"))?;
    let next_gas_use = gas_use
        .checked_add(step_gas_use)
        .ok_or_else(|| ItrErr::new(ItrErrCode::OutOfGas, "gas use overflow"))?;
    let check_limit = |name: &str, used: i64, limit: i64| -> VmrtErr {
        if limit > 0 && used > limit {
            return itr_err_fmt!(
                ItrErrCode::OutOfGas,
                "{} gas limit exceeded: used {} > limit {}",
                name,
                used,
                limit
            );
        }
        Ok(())
    };
    check_limit("compute", next_gas_use.compute, gst.compute_limit)?;
    check_limit("resource", next_gas_use.resource, gst.resource_limit)?;
    check_limit("storage", next_gas_use.storage, gst.storage_limit)?;
    Ok((step_total, next_gas_use))
}

#[allow(unused)]
fn debug_print_stack(ops: &Stack, lcs: &Stack, pc: &usize, inst: Bytecode) {
    println!(
        "operds({})={}\nlocals({})={}\n-------- pc = {}, nbt = {:?}",
        ops.len(),
        &ops.print_stack(),
        lcs.len(),
        &lcs.print_stack(),
        *pc,
        inst
    );
}

#[allow(unused)]
fn debug_print_value(_ctx: &field::Address, _cur: &field::Address, _exec: ExecCtx, _val: Value) {
    debug_println!(
        "{}-{} {} {:?} => {:?}",
        _ctx.to_readable().chars().take(7).collect::<String>(),
        _cur.to_readable().chars().take(7).collect::<String>(),
        _exec.call_depth,
        _exec,
        _val
    );
}
