
/**
* parse bytecode params
*/



macro_rules! itrbuf {
    ($codes: expr, $pc: expr, $l: expr) => {
        { 
            let r = $pc + $l;
            let v: [u8; $l] = $codes[$pc..r].try_into().unwrap();
            $pc = r;
            v
        }
    }
}

macro_rules! itrparam {
    ($codes: expr, $pc: expr, $l: expr, $t: ty) => {
        { 
            let r = $pc + $l;
            let v = <$t>::from_be_bytes($codes[$pc..r].try_into().unwrap());
            $pc = r;
            v
        }
    }
}

macro_rules! itrparamu8 {
    ($codes: expr, $pc: expr) => {
        itrparam!{$codes, $pc, 1, u8}
    }
}

macro_rules! itrparamu16 {
    ($codes: expr, $pc: expr) => {
        itrparam!{$codes, $pc, 2, u16}
    }
}

macro_rules! itrparambufex {
    ($codes: expr, $pc: expr, $l: expr, $t: ty) => {
        {
            let s = itrparam!{$codes, $pc, $l, $t} as usize;
            let l = $pc;
            let r = l + s;
            $pc = r;
            Value::Bytes( $codes[l..r].into() )
        }
    }
}

macro_rules! itrparambuf {
    ($codes: expr, $pc: expr) => {
        itrparambufex!($codes, $pc, 1, u8)
    }
}

macro_rules! itrparambufl {
    ($codes: expr, $pc: expr) => {
        itrparambufex!($codes, $pc, 2, u16)
    }
}

macro_rules! jump {
    ($codes: expr, $pc: expr, $l: expr) => {
        {
            let tpc = match $l {
                1 =>  itrparamu8!($codes, $pc) as usize,
                2 => itrparamu16!($codes, $pc) as usize,
                _ => return itr_err_code!(CodeOverflow),
            };
            $pc = tpc; // jump to
        }
    }
}

macro_rules! ostjump {
    ($codes: expr, $pc: expr, $l: expr) => {
        {
            let tpc = match $l {
                1 => itrparam!{$codes, $pc, 1, i8} as isize,
                2 => itrparam!{$codes, $pc, 2, i16} as isize,
                _ => return itr_err_code!(CodeOverflow),
            };
            let tpc = ($pc as isize + tpc);
            if tpc < 0 {
                return itr_err_code!(CodeOverflow)
            }
            $pc = tpc as usize; // jump to
        }
    }
}

macro_rules! branch {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr) => {
        if $ops.pop()?.check_true() {
            jump!($codes, $pc, $l);
        }else{
            $pc += $l;
        }
    }
}

macro_rules! ostbranchex {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr, $cond: ident) => {
        if $ops.pop()?.$cond() {
            ostjump!($codes, $pc, $l);
        }else{
            $pc += $l;
        }
    }
}
// is_not_zero
macro_rules! ostbranch {
    ( $ops: expr, $codes: expr, $pc: expr, $l: expr) => {
        ostbranchex!($ops, $codes, $pc, $l, check_true)
    }
}

macro_rules! funcptr {
    ($codes: expr, $pc: expr, $mode: expr) => {
        {
            let idx = itrparamu8!($codes, $pc);
            let sig = itrbuf!($codes, $pc, FN_SIGN_WIDTH);
            Call(Funcptr{
                mode: $mode,
                target: CallTarget::Libidx(idx),
                fnsign: sig,
            })
        }
    }
}


/**
* execute code
*/
pub fn execute_code(

    pc: &mut usize, // pc
    codes: &[u8], // max len = 65536
    mode: CallMode,
    depth: isize,

    gas_usable: &mut i64, // max gas can be use

    gas_table: &GasTable, // len = 256
    gas_extra: &GasExtra,
    space_cap: &SpaceCap,

    operands: &mut Stack,
    locals: &mut Stack,
    heap: &mut Heap,

    globals: &mut GKVMap,
    memorys: &mut CtcKVMap,

    ctx: &mut dyn ExtActCal,
    state: &mut VMState,

    context_addr: &ContractAddress, 
    current_addr: &ContractAddress, 

    // _is_sys_call: bool,
    // _call_depth: usize,

) -> VmrtRes<CallExit> {

    use Value::*;
    use CallMode::*;
    use CallExit::*;
    use ItrErrCode::*;
    use Bytecode::*;

    let cap = space_cap;
    let ops = operands;
    let gst = gas_extra;
    let hei: u64 = ctx.height();

    // check code length
    // let codelen = codes.len();
    // let tail = codelen;

    macro_rules! check_gas { () => { if *gas_usable < 0 { return itr_err_code!(OutOfGas) } } }
    macro_rules! pu8 { () => { itrparamu8!(codes, *pc) } }
    macro_rules! pty { () => { ops.peek()?.ty() } }
    macro_rules! ptyn { () => { ops.peek()?.ty_num() } }
    macro_rules! pu8_as_u16 { () => { pu8!() as u16 } }
    macro_rules! pu16 { () => { itrparamu16!(codes, *pc) } }
    macro_rules! pbuf { () => { itrparambuf!(codes, *pc) } }
    macro_rules! pbufl { () => { itrparambufl!(codes, *pc) } }
    macro_rules! pcutbuf { ($w: expr) => { itrbuf!(codes, *pc, $w) } }
    macro_rules! _pctrtaddr { () => { ContractAddress::parse(&pcutbuf!(CONTRACT_ADDRESS_WIDTH)).map_err(|e|ItrErr(ContractAddrErr, e))? }}
    macro_rules! ops_pop_to_u16 { () => { ops.pop()?.checked_u16()? } }
    macro_rules! ops_peek_to_u16 { () => { ops.peek()?.checked_u16()? } }
    macro_rules! check_compo_type { ($m: ident) => { match ops.compo() { Ok(c) => c.$m(), _ => false, } } }

    // start run
    let exit;
    loop {
        // read inst
        let instbyte = codes[*pc as usize]; // u8
        let instruction: Bytecode = std_mem_transmute!(instbyte.clone());
        *pc += 1; // next

        // debug_print_stack(ops, locals, pc, instruction);

        // do execute
        let mut gas: i64 = 0;
        *gas_usable -= gas_table.gas(instbyte); // 
        // println!("gas usable {} cp: {}, inst: {:?}", *gas_usable, gas_table.gas(instbyte), instruction);

        macro_rules! extcall { ($is_action: expr, $pass_body: expr, $have_retv: expr) => {
            if $is_action && (mode != Main || depth > 0)  {
                return itr_err_fmt!(ExtActDisabled, "extend action just can use in main call")
            }
            if mode == Static {
                return itr_err_fmt!(ExtActDisabled, "extend call not support in static call")
            }
            let idx = pu8!();
            let kid = u16::from_be_bytes(vec![instbyte, idx].try_into().unwrap());
            let mut actbody = vec![];
            if $pass_body {
                let mut bdv = ops.peek()?.canbe_ext_call_data(heap)?;
                actbody.append(&mut bdv);
            }
            let (bgasu, cres) = ctx.action_call(kid, actbody).map_err(|e|
                ItrErr::new(ExtActCallError, e.as_str()))?;
            gas += bgasu as i64;
            let resv;
            if $have_retv {
                let vty = match instruction {
                    EXTENV  => search_ext_by_id(idx, &CALL_EXTEND_ENV_DEFS),
                    EXTFUNC => search_ext_by_id(idx, &CALL_EXTEND_FUNC_DEFS),
                    _ => never!(),
                }.unwrap().2;
                resv = Value::type_from(vty, cres)?; //  from ty
            } else {
                resv = Value::Bytes(cres); // only bytes
            }
            if $pass_body && $have_retv {
                *ops.peek()? = resv;
            } else if !$pass_body &&  $have_retv {
                ops.push(resv)?;
            } else if  $pass_body && !$have_retv {
                ops.pop()?;
            } else {
                never!()
            }
        }}

        let mut ntcall = |idx: u8| -> VmrtErr {
            let argv = match idx {
                NativeCall::idx_context_address => context_addr.serialize(), // context_address
                _ => ops.peek()?.canbe_ext_call_data(heap)?
            };
            let (r, g) = NativeCall::call(hei, idx, &argv)?;
            *ops.peek()? = r; gas += g; 
            Ok(())
        };

        match instruction {
            // ext action
            EXTACTION => { extcall!(true,  true,  false); },
            EXTENV    => { extcall!(false, false, true);  },
            EXTFUNC   => { extcall!(false, true,  true);  },
            // native call
            NTCALL => ntcall(pu8!())?,
            // constant
            PU8   => ops.push(U8(pu8!()))?,
            PU16  => ops.push(U16(pu16!()))?,
            PBUF  => ops.push(pbuf!())?,
            PBUFL => ops.push(pbufl!().valid(cap)?)?, // buf long
            P0    => ops.push(U8(0))?,
            P1    => ops.push(U8(1))?,
            P2    => ops.push(U8(2))?,
            P3    => ops.push(U8(3))?,
            PNBUF => ops.push(Value::empty_bytes())?,
            PNIL  => ops.push(Value::Nil)?,
            // cast & type
            CU8   => ops.peek()?.cast_u8()?,
            CU16  => ops.peek()?.cast_u16()?,
            CU32  => ops.peek()?.cast_u32()?,
            CU64  => ops.peek()?.cast_u64()?,
            CU128 => ops.peek()?.cast_u128()?, /* CU256 => ops.peek()?.cast_u256()?, */
            CBUF  => ops.peek()?.cast_buf()?,
            CTO   => ops.peek()?.cast_to(pu8!())?,
            TNIL  => *ops.peek()? = Bool(pty!() == ValueTy::Nil),
            TLIST => *ops.peek()? = Bool(check_compo_type!(is_list)),
            TMAP  => *ops.peek()? = Bool(check_compo_type!(is_map)),
            TIS   => *ops.peek()? = Bool(ptyn!() == pu8!()),
            TID   => *ops.peek()? =   U8(ptyn!()),
            // stack & buffer
            DUP    => ops.push(ops.last()?)?,
            DUPN   => ops.dupn(pu8!())?,
            POP    => { ops.pop()?; } // drop
            POPN   => { ops.popn(pu8!())?; },
            PICK   => ops.pick(pu8!())?,
            SWAP   => ops.swap()?,
            REV    => ops.reverse(pu8!())?, // reverse
            CHOISE => { if ops.pop()?.check_false() { ops.swap()? } ops.pop()?; } /* x ? a : b */
            CAT    => ops.cat(cap)?,
            JOIN   => ops.join(pu8!(), cap)?,
            BYTE   => { let i = ops_pop_to_u16!(); ops.peek()?.cutbyte(i)?; }  
            CUT    => { let (l, o) = (ops.pop()?, ops.pop()?); ops.peek()?.cutout(l, o)?; }
            LEFT   => ops.peek()?.cutleft(  pu8_as_u16!())?,
            RIGHT  => ops.peek()?.cutright( pu8_as_u16!())?,
            LDROP  => ops.peek()?.dropleft( pu8_as_u16!())?,
            RDROP  => ops.peek()?.dropright(pu8_as_u16!())?,
            SIZE   => { *ops.peek()? = U16(ops.peek()?.can_get_size()?) }
            // compo
            NEWLIST  => ops.push(Compo(CompoItem::new_list()))?,
            NEWMAP   => ops.push(Compo(CompoItem::new_map()))?,
            PACKLIST => { let l = CompoItem::pack_list(cap, ops)?; ops.push(l)? }
            PACKMAP  => { let m = CompoItem::pack_map( cap, ops)?; ops.push(m)? }
            INSERT   => { let v = ops.pop()?; let k = ops.pop()?; ops.compo()?.insert(cap, k, v)? }
            REMOVE   => { let k = ops.pop()?; ops.compo()?.remove(k)?; }
            CLEAR    => { ops.compo()?.clear() }
            MERGE    => { let p = ops.pop()?; ops.compo()?.merge(p.compo_get()?)?; }
            LENGTH   => { let l = ops.compo()?.length(cap)?; *ops.peek()? = l; }
            HASKEY   => { let k = ops.pop()?; let h = ops.compo()?.haskey(k)?; *ops.peek()? = h; }
            ITEMGET  => { let k = ops.pop()?; *ops.peek()? = ops.compo()?.itemget(k)?; }
            KEYS     => { ops.compo()?.keys()?; }
            VALUES   => { ops.compo()?.values()?; }
            HEAD     => { let v = ops.pop()?.compo()?.head()?; ops.push(v)?; }
            TAIL     => { let v = ops.pop()?.compo()?.tail()?; ops.push(v)?; }
            APPEND   => { let v = ops.pop()?; ops.compo()?.append(cap, v)? }
            CLONE    => { let c = ops.compo()?.copy(); *ops.peek()? = Compo(c); }
            UPLIST   => { let i = ops.pop()?.checked_u8()?; unpack_list(i, locals, ops.pop()?.compo()?.list_mut()?)?; }
            // heap
            HGROW    => gas += heap.grow(pu8!())?,
            HWRITE   => heap.write(ops_pop_to_u16!(), ops.pop()?)?,
            HREAD    => { let n = ops.pop()?; *ops.peek()? = heap.read(ops.peek()?, n)? }
            HWRITEX  => heap.write(pu8_as_u16!(),  ops.pop()?)?,
            HWRITEXL => heap.write(pu16!(), ops.pop()?)?,
            HREADU   => ops.push(heap.read_u(  pu8!())?)?,
            HREADUL  => ops.push(heap.read_ul(pu16!())?)?,
            HSLICE   => *ops.peek()? = heap.slice(ops.pop()?, ops.peek()?)?,
            // locals & heap & global & memory
            XLG   => local_logic(pu8!(), locals, ops.peek()?)?,
            XOP   => local_operand(pu8!(), locals, ops.pop()?)?,
            ALLOC => { gas += gst.local_one_alloc * locals.alloc(pu8!())? as i64 } 
            PUTX   => locals.save(ops_pop_to_u16!(), ops.pop()?)?,
            GETX   => *ops.peek()? = locals.load(ops_peek_to_u16!() as usize)?,
            PUT   => locals.save(pu8_as_u16!(), ops.pop()?)?,
            GET   => ops.push(locals.load(pu8!() as usize)?)?,
            GET0  => ops.push(locals.load(0)?)?,
            GET1  => ops.push(locals.load(1)?)?,
            GET2  => ops.push(locals.load(2)?)?,
            GET3  => ops.push(locals.load(3)?)?,
            // storage
            SREST => *ops.peek()? = state.srest(hei, context_addr, ops.peek()?)?,
            SLOAD => *ops.peek()? = state.sload(hei, context_addr, ops.peek()?)?,
            SDEL  => state.sdel(context_addr, ops.pop()?)?,
            SSAVE => { let v = ops.pop()?; state.ssave(hei, context_addr, ops.pop()?, v)?; },
            SRENT => { let t = ops.pop()?; gas += state.srent(gst, hei, context_addr, ops.pop()?, t)?; },
            // global & memory
            GPUT => { let v = ops.pop()?; globals.put(ops.pop()?, v)?; },
            GGET => *ops.peek()? = globals.get(ops.peek()?)?,
            MPUT => { let v = ops.pop()?; memorys.entry(context_addr)?.put(ops.pop()?, v)?; },
            MGET => *ops.peek()? = memorys.entry(context_addr)?.get(ops.peek()?)?,
            // logic
            AND  => binop_btw(ops, lgc_and)?,
            OR   => binop_btw(ops, lgc_or)?,
            EQ   => binop_btw(ops, lgc_equal)?,
            NEQ  => binop_btw(ops, lgc_not_equal)?,
            LT   => binop_btw(ops, lgc_less)?,
            GT   => binop_btw(ops, lgc_greater)?,
            LE   => binop_btw(ops, lgc_less_equal)?,
            GE   => binop_btw(ops, lgc_greater_equal)?,
            NOT  => ops.peek()?.cast_bool_not()?,
            // bitop
            BSHR => binop_arithmetic(ops, bit_shr)?,
            BSHL => binop_arithmetic(ops, bit_shl)?,
            BXOR => binop_arithmetic(ops, bit_xor)?,
            BOR  => binop_arithmetic(ops, bit_or)?,
            BAND => binop_arithmetic(ops, bit_and)?,
            // arithmetic
            ADD  => binop_arithmetic(ops, add_checked)?,
            SUB  => binop_arithmetic(ops, sub_checked)?,
            MUL  => binop_arithmetic(ops, mul_checked)?,
            DIV  => binop_arithmetic(ops, div_checked)?,
            MOD  => binop_arithmetic(ops, mod_checked)?,
            POW  => binop_arithmetic(ops, pow_checked)?,
            MAX  => binop_arithmetic(ops, max_checked)?,
            MIN  => binop_arithmetic(ops, min_checked)?,
            INC  => ops.peek()?.inc(pu8!())?,
            DEC  => ops.peek()?.dec(pu8!())?,
            // workflow control
            JMPL  =>        jump!(codes, *pc, 2),
            JMPS  =>     ostjump!(codes, *pc, 1),
            JMPSL =>     ostjump!(codes, *pc, 2),
            BRL   =>      branch!(ops, codes, *pc, 2),
            BRS   =>   ostbranch!(ops, codes, *pc, 1),
            BRSL  =>   ostbranch!(ops, codes, *pc, 2),   
            BRSLN => ostbranchex!(ops, codes, *pc, 2, check_false),   
            // other
            NT   => return itr_err_code!(InstNeverTouch), // never touch
            NOP  => {}, // do nothing
            BURN => gas += pu16!() as i64,         
            // exit
            RET => { exit = Return; break }, // func return <DATA>
            END => { exit = Finish; break }, // func end
            ERR => { exit = Throw;  break },  // throw <ERROR>
            ABT => { exit = Abort;  break },  // panic
            AST => { if ops.pop()?.check_false() { exit = Abort;  break } }, // assert(..)
            PRT => { debug_print_value(context_addr, current_addr, mode, depth, ops.pop()?) }
            // call CALLDYN
            CALLCODE | CALLSTATIC | CALLLIB | CALLINR | CALL => {
                let ist = instruction;
                check_call_mode(mode, ist)?;
                // ok return
                match ist {
                    CALLCODE =>   exit = funcptr!(codes, *pc, CodeCopy),
                    CALLSTATIC => exit = funcptr!(codes, *pc, Static),
                    CALLLIB =>    exit = funcptr!(codes, *pc, Library),
                    CALL =>       exit = funcptr!(codes, *pc, Outer),
                    CALLINR =>    exit = Call(Funcptr{
                        mode: Inner,
                        target: CallTarget::Inner,
                        fnsign: pcutbuf!(FN_SIGN_WIDTH),
                    }),
                    /* CALLDYN =>    exit = Call(Funcptr{ // Outer
                        mode: Outer,
                        target: CallTarget::Addr(ops.pop()?.checked_contract_address()?),
                        fnsign: ops.pop()?.checked_fnsign()?,
                    }), */
                    _ => unreachable!()
                };
                break
                // call exit
            }
            // inst invalid
            _ => return itr_err_fmt!(InstInvalid, "{}", instbyte),
        }

        // reduce gas for use
        *gas_usable -= gas; // more gas use
        check_gas!();
        // next
    }

    // exit
    check_gas!();
    Ok(exit)

}


fn check_call_mode(mode: CallMode, inst: Bytecode) -> VmrtErr {
    use CallMode::*;
    use Bytecode::*;
    macro_rules! not_ist {
        ( $( $ist: expr ),+ ) => {
            ![$( $ist ),+].contains(&inst)
        }
    }
    match mode {
        Main    if not_ist!(CALL, CALLSTATIC, CALLCODE)
            => itr_err_code!(CallOtherInMain),
        Abst    if not_ist!(CALLINR, CALLLIB, CALLSTATIC, CALLCODE)
            => itr_err_code!(CallInAbst),
        Library if not_ist!(CALLLIB, CALLSTATIC, CALLCODE)
            => itr_err_code!(CallLocInLib),
        Static  if not_ist!(CALLSTATIC, CALLCODE)
            => itr_err_code!(CallLibInStatic),
        CodeCopy // not allowed any call
            => itr_err_code!(CallInCodeCopy),
        _ => Ok(()), // Outer | Inner support all call instructions
    }
}


fn local_operand(mark: u8, locals: &mut Stack, mut value: Value) -> VmrtErr {
    let opt = mark >> 6; // 0b00000011
    let idx = mark & 0b00111111; // max=64
    let basev = locals.edit(idx)?;
    match opt {
        0 => locop_arithmetic(basev, &mut value, add_checked), // +=
        1 => locop_arithmetic(basev, &mut value, sub_checked), // -=
        2 => locop_arithmetic(basev, &mut value, mul_checked), // *=
        3 => locop_arithmetic(basev, &mut value, div_checked), // /=
        _ => unreachable!(), // return itr_err_fmt!(InstParamsErr, "local operand {} not find", a)
    }?;
    Ok(())
}


fn local_logic(mark: u8, locals: &mut Stack, value: &mut Value) -> VmrtErr {
    let opt = mark >> 5; // 0b00000111
    let idx = mark & 0b00011111; // max=32
    let basev = locals.edit(idx)?;
    match opt {
        0 => locop_btw(value, basev, lgc_and),
        1 => locop_btw(value, basev, lgc_or),
        2 => locop_btw(value, basev, lgc_equal),
        3 => locop_btw(value, basev, lgc_not_equal),
        4 => locop_btw(value, basev, lgc_less),
        5 => locop_btw(value, basev, lgc_less_equal),
        6 => locop_btw(value, basev, lgc_greater),
        7 => locop_btw(value, basev, lgc_greater_equal),
        _ => unreachable!(), // return itr_err_fmt!(InstParamsErr, "local operand {} not find", a)
    }?;
    Ok(())
}


fn unpack_list(mut i: u8, locals: &mut Stack, list: &mut VecDeque<Value>) -> VmrtErr {
    let start = i as usize;
    if locals.len() < start + list.len() {
        return itr_err_code!(OutOfLocal)
    }
    // replace
    while let Some(item) = list.pop_front() {
        *locals.edit(i)? = item;
        i += 1;
    }
    Ok(())
}


fn debug_print_value(ctx: &ContractAddress, cur: &ContractAddress 
, mode: CallMode, depth: isize, val: Value) {
    debug_println!("{}-{} {} {:?} => {:?}", 
        ctx.prefix(7), cur.prefix(7), depth, mode, val);
}


#[allow(unused)]
fn debug_print_stack(ops: &Stack, lcs: &Stack, pc: &usize, inst: Bytecode) {
    println!("operds({})={}\nlocals({})={}\n-------- pc = {}, nbt = {:?}", 
    ops.len(), &ops.print_stack(), lcs.len(), &lcs.print_stack(), 
    *pc, inst);

}