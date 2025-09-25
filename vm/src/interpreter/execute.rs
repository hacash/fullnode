
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
    _current_addr: &ContractAddress, 

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
    macro_rules! _ops_peek_to_u16 { () => { ops.peek()?.checked_u16()? } }
    macro_rules! check_compo_type { ($m: ident) => { match ops.compo() { Ok(c) => c.$m(), _ => false, } } }

    // start run
    let exit;
    loop {
        // read inst
        let instbyte = codes[*pc as usize]; // u8
        let instruction: Bytecode = std_mem_transmute!(instbyte.clone());
        *pc += 1; // next

        // debug_println!("operds = {}\nlocals = {}\n-------- pc = {}, nbt = {:?}", &ops.print_stack(), &locals.print_stack(), pc, instruction);

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
            let vid = idx as usize;
            if $have_retv {
                let vty = match instruction {
                    EXTENV  => CALL_EXTEND_ENV_DEFS[vid],
                    EXTFUNC => CALL_EXTEND_FUNC_DEFS[vid],
                    _ => never!(),
                }.2;
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

        match instruction {
            // ext action
            EXTACTION => { extcall!(true,  true,  false); },
            EXTENV    => { extcall!(false, false, true);  },
            EXTFUNC   => { extcall!(false, true,  true);  },
            // native call
            NTCALL => { let (r, g) = NativeCall::call(pu8!(), &ops.peek()?.canbe_ext_call_data(heap)?)?;
                *ops.peek()? = r; gas += g; },
            // type
            TLIST => *ops.peek()? = Bool(check_compo_type!(is_list)),
            TMAP  => *ops.peek()? = Bool(check_compo_type!(is_map)),
            TNIL  => *ops.peek()? = Bool(pty!() == ValueTy::Nil),
            TIS   => *ops.peek()? = Bool(ptyn!() == pu8!()),
            TID   => *ops.peek()? =   U8(ptyn!()),
            // cast
            CU8   => ops.peek()?.cast_u8()?,
            CU16  => ops.peek()?.cast_u16()?,
            CU32  => ops.peek()?.cast_u32()?,
            CU64  => ops.peek()?.cast_u64()?,
            CU128 => ops.peek()?.cast_u128()?,
            /* CU256 => ops.peek()?.cast_u256()?, */
            CBUF  => ops.peek()?.cast_buf()?,
            CTO   => ops.peek()?.cast_to(pu8!())?,
            // constant
            P0    => ops.push(U8(0))?,
            P1    => ops.push(U8(1))?,
            P2    => ops.push(U8(2))?,
            P3    => ops.push(U8(3))?,
            PU8   => ops.push(U8(pu8!()))?,
            PU16  => ops.push(U16(pu16!()))?,
            PBUF  => ops.push(pbuf!())?,
            PBUFL => ops.push(pbufl!().valid(cap)?)?, // buf long
            PNBUF => ops.push(Value::empty_bytes())?,
            PNIL  => ops.push(Value::Nil)?,
            // stack & buffer operand
            DUP    => ops.push(ops.last()?)?,
            DUPX   => ops.dupx(pu8!())?,
            POP    => { ops.pop()?; } // drop
            POPX   => ops.popx(pu8!())?,
            SWAP   => ops.swap()?,
            REV    => ops.reverse()?, // reverse
            CHOISE => { if ops.pop()?.check_false() { ops.swap()? } ops.pop()?; } /* x ? a : b */
            SIZE   => { *ops.peek()? = U16(ops.peek()?.can_get_size()?) }
            CAT    => ops.cat(cap)?,
            JOIN   => ops.join(cap)?,
            BYTE   => { let i = ops_pop_to_u16!(); ops.peek()?.cutbyte(i)?; }
            CUT    => { let (l, o) = (ops.pop()?, ops.pop()?); ops.peek()?.cutout(l, o)?; }
            LEFT   => ops.peek()?.cutleft(  pu8_as_u16!())?,
            RIGHT  => ops.peek()?.cutright( pu8_as_u16!())?,
            LDROP  => ops.peek()?.dropleft( pu8_as_u16!())?,
            RDROP  => ops.peek()?.dropright(pu8_as_u16!())?,
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
            ITEM     => { let k = ops.pop()?; *ops.peek()? = ops.compo()?.itemget(k)?; }
            KEYS     => { ops.compo()?.keys()?; }
            VALUES   => { ops.compo()?.values()?; }
            HEAD     => { let v = ops.pop()?.compo()?.head()?; ops.push(v)?; }
            TAIL     => { let v = ops.pop()?.compo()?.tail()?; ops.push(v)?; }
            APPEND   => { let v = ops.pop()?; ops.compo()?.append(cap, v)? }
            CLONE    => { let c = ops.compo()?.copy(); *ops.peek()? = Compo(c); }
            // heap
            HGROW    => gas += heap.grow(pu8!())?,
            HWRITE   => heap.write(ops.pop()?, ops.pop()?)?,
            HREAD    => *ops.peek()? = heap.read(ops.pop()?, ops.peek()?)?,
            HWRITEX  => heap.write_x(  pu8!(), ops.pop()?)?,
            HWRITEXL => heap.write_xl(pu16!(), ops.pop()?)?,
            HREADU   => ops.push(heap.read_u(  pu8!())?)?,
            HREADUL  => ops.push(heap.read_ul(pu16!())?)?,
            HSLICE   => *ops.peek()? = heap.slice(ops.pop()?, ops.peek()?)?,
            // locals & heap & global & memory
            // GETX   => *ops.peek()? = locals.load(ops_peek_to_u16!() as usize)?,
            // PUTX   => locals.save(ops_pop_to_u16!(), ops.pop()?)?,
            GET3  => ops.push(locals.load(3)?)?,
            GET2  => ops.push(locals.load(2)?)?,
            GET1  => ops.push(locals.load(1)?)?,
            GET0  => ops.push(locals.load(0)?)?,
            GET   => ops.push(locals.load(pu8!() as usize)?)?,
            PUT   => locals.save(pu8_as_u16!(), ops.pop()?)?,
            MOVE  => locals.save(pu8_as_u16!(), ops.pop()?)?,
            ALLOC => { gas += gst.local_one_alloc * locals.alloc(pu8!())? as i64 } 
            XOP   => local_operand(pu8!(), locals, ops.pop()?)?,
            XLG   => local_logic(pu8!(), locals, ops.peek()?)?,
            // storage
            SRENT => gas += state.srent(gst, hei, context_addr, ops.pop()?, ops.pop()?)?,
            SDEL  => state.sdel(context_addr, ops.pop()?)?,
            SSAVE => state.ssave(hei, context_addr,ops.pop()?, ops.pop()?)?,
            SLOAD => *ops.peek()? = state.sload(hei, context_addr, ops.peek()?)?,
            STIME => *ops.peek()? = state.stime(hei, context_addr, ops.peek()?)?,
            // memory & global
            MGET => *ops.peek()? = memorys.entry(context_addr)?.get(ops.peek()?)?,
            MPUT => memorys.entry(context_addr)?.put(ops.pop()?, ops.pop()?)?,
            GGET => *ops.peek()? = globals.get(ops.peek()?)?,
            GPUT => globals.put(ops.pop()?, ops.pop()?)?,
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
        Main    if not_ist!(CALL, CALLSTATIC, CALLCODE) // CALLDYN
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