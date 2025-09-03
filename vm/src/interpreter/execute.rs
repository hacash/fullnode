
/**
* parse bytecode params
*/




macro_rules! checkcodetail {
    ($pc: expr, $tail: expr) => {
        if $pc >= $tail {
            return itr_err_code!(CodeOverflow)
        }
    }
}

macro_rules! itrbuf {
    ($codes: expr, $pc: expr, $tail: expr, $l: expr) => {
        { 
            let r = $pc + $l;
            if r > $tail {
                return itr_err_code!(CodeOverflow)
            }
            let v: [u8; $l] = $codes[$pc..r].try_into().unwrap();
            $pc = r;
            v
        }
    }
}

macro_rules! itrparam {
    ($codes: expr, $pc: expr, $tail: expr, $l: expr, $t: ty) => {
        { 
            let r = $pc + $l; 
            if r > $tail {
                return itr_err_code!(CodeOverflow)
            }
            let v = <$t>::from_be_bytes($codes[$pc..r].try_into().unwrap());
            $pc = r;
            v
        }
    }
}

macro_rules! itrparamu8 {
    ($codes: expr, $pc: expr, $tail: expr) => {
        itrparam!{$codes, $pc, $tail, 1, u8}
    }
}

macro_rules! itrparamu16 {
    ($codes: expr, $pc: expr, $tail: expr) => {
        itrparam!{$codes, $pc, $tail, 2, u16}
    }
}

macro_rules! itrparambufex {
    ($codes: expr, $pc: expr, $tail: expr, $l: expr, $t: ty) => {
        {
            let s = itrparam!{$codes, $pc, $tail, $l, $t} as usize + 1;
            let l = $pc;
            let r = l + s;
            if r > $tail {
                return itr_err_code!(CodeOverflow)
            }
            $pc = r;
            Value::Bytes( $codes[l..r].into() )
        }
    }
}

macro_rules! itrparambuf {
    ($codes: expr, $pc: expr, $tail: expr) => {
        itrparambufex!($codes, $pc, $tail, 1, u8)
    }
}

macro_rules! itrparambufl {
    ($codes: expr, $pc: expr, $tail: expr) => {
        itrparambufex!($codes, $pc, $tail, 2, u16)
    }
}

macro_rules! jump {
    ($codes: expr, $pc: expr, $tail: expr, $l: expr) => {
        {
            let tpc = match $l {
                1 =>  itrparamu8!($codes, $pc, $tail) as usize,
                2 => itrparamu16!($codes, $pc, $tail) as usize,
                _ => return itr_err_code!(CodeOverflow),
            };
            checkcodetail!(tpc, $tail);
            $pc = tpc; // jump to
        }
    }
}

macro_rules! ostjump {
    ($codes: expr, $pc: expr, $tail: expr, $l: expr) => {
        {
            let tpc = match $l {
                1 => itrparam!{$codes, $pc, $tail, 1, i8} as isize,
                2 => itrparam!{$codes, $pc, $tail, 2, i16} as isize,
                _ => return itr_err_code!(CodeOverflow),
            };
            let tpc = ($pc as isize + tpc);
            if tpc < 0 {
                return itr_err_code!(CodeOverflow)
            }
            checkcodetail!(tpc as usize, $tail);
            $pc = tpc as usize; // jump to
        }
    }
}

macro_rules! branch {
    ( $ops: expr, $codes: expr, $pc: expr, $tail: expr, $l: expr) => {
        if $ops.pop()?.checked_bool()? {
            jump!($codes, $pc, $tail, $l);
        }else{
            $pc += $l;
        }
    }
}

macro_rules! ostbranchex {
    ( $ops: expr, $codes: expr, $pc: expr, $tail: expr, $l: expr, $cond: ident) => {
        if $ops.pop()?.$cond()? {
            ostjump!($codes, $pc, $tail, $l);
        }else{
            $pc += $l;
        }
    }
}
// is_not_zero
macro_rules! ostbranch {
    ( $ops: expr, $codes: expr, $pc: expr, $tail: expr, $l: expr) => {
        ostbranchex!($ops, $codes, $pc, $tail, $l, checked_bool)
    }
}

macro_rules! funcptr {
    ($codes: expr, $pc: expr, $tail: expr, $mode: expr) => {
        {
            let idx = itrparamu8!($codes, $pc, $tail);
            let sig = itrbuf!($codes, $pc, $tail, FN_SIGN_WIDTH);
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
    let codelen = codes.len();
    if codelen == 0 {
        return itr_err_code!(CodeEmpty)
    }
    if codelen > u16::MAX as usize {
        return itr_err_code!(CodeTooLong)
    }
    let tail = codelen;

    macro_rules! check_gas { () => { if *gas_usable < 0 { return itr_err_code!(OutOfGas) } } }
    macro_rules! pu8 { () => { itrparamu8!(codes, *pc, tail) } }
    macro_rules! pu8_as_u16 { () => { pu8!() as u16 } }
    macro_rules! pu16 { () => { itrparamu16!(codes, *pc, tail) } }
    macro_rules! pbuf { () => { itrparambuf!(codes, *pc, tail) } }
    macro_rules! pbufl { () => { itrparambufl!(codes, *pc, tail) } }
    macro_rules! pcutbuf { ($w: expr) => { itrbuf!(codes, *pc, tail, $w) } }
    macro_rules! _pctrtaddr { () => { ContractAddress::parse(&pcutbuf!(CONTRACT_ADDRESS_WIDTH)).map_err(|e|ItrErr(ContractAddrErr, e))? }}
    macro_rules! ops_pop_to_u16 { () => { ops.pop()?.checked_u16()? } }
    macro_rules! ops_peek_to_u16 { () => { ops.peek()?.checked_u16()? } }

    // start run
    let exit;
    loop {
        // debug_println!("-------- pc = {}\noperds = {}\nlocals = {}", pc,  &ops.print_stack(), &locals.print_stack());

        // read inst
        let instbyte = codes[*pc as usize]; // u8
        let instruction: Bytecode = std_mem_transmute!(instbyte.clone());
        *pc += 1; // next

        // do execute
        let mut gas: i64 = 0;
        *gas_usable -= gas_table.gas(instbyte); // 
        // println!("gas usable {} cp: {}, inst: {:?}", *gas_usable, gas_table.gas(instbyte), instruction);


        macro_rules! extcall { ($ifv: expr) => { 
            let kid = u16::from_be_bytes(vec![instbyte, pu8!()].try_into().unwrap());
            let mut actbody = vec![];
            if $ifv {
                let mut bdv = ops.peek()?.to_bytes();
                actbody.append(&mut bdv);
            }
            let (bgasu, cres) = ctx.action_call(kid, actbody).map_err(|e|
                ItrErr::new(ExtActCallError, e.as_str()))?;
            gas += bgasu as i64;
            let resv = Value::bytes(cres);
            if $ifv {
                *ops.peek()? = resv;
            } else {
                ops.push(resv)?;
            }
        }}

        match instruction {
            // ext action
            EXTACTION => {
                if mode != Main || depth > 0 {
                    return itr_err_fmt!(ExtActDisabled, "extend action just can use in main call")
                }
                extcall!(true); },
            EXTFUNC   => { extcall!(true); },
            EXTENV    => { 
                if mode == Static {
                    return itr_err_fmt!(ExtActDisabled, "extend env not support in static call")
                }
                extcall!(false); },
            // native call
            NTCALL => { let (r, g) = NativeCall::call(pu8!(), ops.peek()?)?;
                *ops.peek()? = r; gas += g; },
            // constant
            P0    => ops.push(U8(0))?,
            P1    => ops.push(U8(1))?,
            PU8   => ops.push(U8(pu8!()))?,
            PU16  => ops.push(U16(pu16!()))?,
            PNBUF => ops.push(Value::empty_bytes())?,
            PBUF  => ops.push(pbuf!())?,
            PBUFL => ops.push(pbufl!().valid(cap)?)?, // buf long
            // stack & buffer operand
            DUP    => ops.push(ops.last()?)?,
            DUPX   => ops.dupx(pu8!())?,
            POP    => { ops.pop()?; }, // drop
            POPX   => ops.popx(pu8!())?,
            SWAP   => ops.swap()?,
            REV    => ops.reverse()?, // reverse
            CHIOSE => { if ops.pop()?.is_zero() { ops.swap()? } ops.pop()?; }, /* x ? a : b */
            SIZE   => *ops.peek()? = U16(ops.peek()?.val_size() as u16),
            CAT    => ops.cat(cap)?,
            JOIN   => ops.join(cap)?,
            BYTE   => { let i = ops_pop_to_u16!(); ops.peek()?.cutbyte(i)?; },
            CUT    => { let (l, o) = (ops.pop()?, ops.pop()?); ops.peek()?.cutout(l, o)?; },
            LEFT   => ops.peek()?.cutleft( pu8_as_u16!() + 1)?,
            RIGHT  => ops.peek()?.cutright(pu8_as_u16!() + 1)?,
            // locals & heap & global & memory
            ALLOC => { let num = pu8!(); locals.alloc(num)?; 
                gas += num as i64 * gst.local_one_alloc; },
            GET   => *ops.peek()? = locals.load(ops_peek_to_u16!() as usize)?,
            PUT   => { locals.save(ops_pop_to_u16!(), ops.pop()?)?; 
                gas += gst.local_put; },
            GETX  => ops.push(locals.load(pu8!() as usize)?)?,
            PUTX  => { locals.save(pu8_as_u16!(), ops.pop()?)?; 
                gas += gst.local_put; },
            MOVE => locals.append(ops.popn(pu8!())?)?,
            XOP   => local_operand(pu8!(), locals, ops.pop()?)?,
            XLG   => local_logic(pu8!(), locals, ops.peek()?)?,
            HGROW    => gas += heap.grow(pu8!())?,
            HWRITE   => heap.write(ops.pop()?, ops.pop()?)?,
            HREAD    => *ops.peek()? = heap.read(ops.pop()?, ops.peek()?)?,
            HWRITEX  => heap.write_x(  pu8!(), ops.pop()?)?,
            HWRITEXL => heap.write_xl(pu16!(), ops.pop()?)?,
            HREADU   => ops.push(heap.read_u(  pu8!())?)?,
            HREADUL  => ops.push(heap.read_ul(pu16!())?)?,
            GPUT => { globals.put(ops.pop()?, ops.pop()?)?;
                gas += gst.global_put; },
            GGET => { *ops.peek()? = globals.get(ops.peek()?)?;
                gas += gst.global_get; },
            MPUT => { memorys.entry(context_addr)?.put(ops.pop()?, ops.pop()?)?;
                gas += gst.memory_put; },
            MGET => { *ops.peek()? = memorys.entry(context_addr)?.get(ops.peek()?)?;
                gas += gst.memory_get; },
            // storage
            SRENT => {
                let (period, k) = (ops_pop_to_u16!(), ops.pop()?);
                let addgas = state.renew(gst, hei, period, context_addr, k)?;
                gas += addgas; },
            SRCV  => { state.restore(hei, context_addr, ops.pop()?, ops.pop()?)?;
                gas += gst.storage_recover; },
            SDEL  => state.delete(context_addr, ops.pop()?)?,
            SSAVE => { let period = pu8_as_u16!();
                let (k, v) = (ops.pop()?, ops.pop()?);
                let v_size = v.val_size() as i64;
                state.save(hei, period, context_addr, k, v)?;
                gas += (gst.storage_save_base + v_size ) * period as i64;
            },
            SLOAD => { *ops.peek()? = state.load(hei, context_addr, ops.peek()?)?;
                gas += gst.storage_read; },
            // cast
            CU8   => ops.peek()?.cast_u8()?,
            CU16  => ops.peek()?.cast_u16()?,
            CU32  => ops.peek()?.cast_u32()?,
            CU64  => ops.peek()?.cast_u64()?,
            CU128 => ops.peek()?.cast_u128()?,
            /*CASTU256 => ops.peek()?.cast_u256()?,*/
            CBUF  => ops.peek()?.cast_buf()?,
            TYPEID => *ops.peek()? = U8(ops.peek()?.ty_num()),
            // bitop
            BAND => binop_arithmetic(ops, bit_and)?,
            BOR  => binop_arithmetic(ops, bit_or)?,
            BXOR => binop_arithmetic(ops, bit_xor)?,
            BSHL => binop_arithmetic(ops, bit_shl)?,
            BSHR => binop_arithmetic(ops, bit_shr)?,
            // logic
            NOT => ops.peek()?.cast_bool_not(),
            AND => binop_btw(ops, lgc_and)?,
            OR  => binop_btw(ops, lgc_or)?,
            EQ  => binop_btw(ops, lgc_equal)?,
            NEQ => binop_btw(ops, lgc_not_equal)?,
            LT  => binop_btw(ops, lgc_less)?,
            GT  => binop_btw(ops, lgc_greater)?,
            LE  => binop_btw(ops, lgc_less_equal)?,
            GE  => binop_btw(ops, lgc_greater_equal)?,
            // arithmetic
            ADD => binop_arithmetic(ops, add_checked)?,
            SUB => binop_arithmetic(ops, sub_checked)?,
            MUL => binop_arithmetic(ops, mul_checked)?,
            DIV => binop_arithmetic(ops, div_checked)?,
            MOD => binop_arithmetic(ops, mod_checked)?,
            POW => binop_arithmetic(ops, pow_checked)?,
            MAX => binop_arithmetic(ops, max_checked)?,
            MIN => binop_arithmetic(ops, min_checked)?,
            INC => ops.peek()?.inc(pu8!())?,
            DEC => ops.peek()?.dec(pu8!())?,
            // workflow control
            JMPL  =>        jump!(codes, *pc, tail, 2),
            JMPS  =>     ostjump!(codes, *pc, tail, 1),
            JMPSL =>     ostjump!(codes, *pc, tail, 2),
            BRL   =>      branch!(ops, codes, *pc, tail, 2),
            BRS   =>   ostbranch!(ops, codes, *pc, tail, 1),
            BRSL  =>   ostbranch!(ops, codes, *pc, tail, 2),   
            BRSLN => ostbranchex!(ops, codes, *pc, tail, 2, checked_bool_not),   
            // other
            NT   => return itr_err_code!(InstNeverTouch), // never touch
            NOP  => {}, // do nothing
            BURN => gas += pu16!() as i64,         
            // exit
            RET => { exit = Return; break }, // func return <DATA>
            END => { exit = Finish; break }, // func end
            ERR => { exit = Throw;  break },  // throw <ERROR>
            ABT => { exit = Abort;  break },  // panic
            AST => { if !ops.pop()?.to_bool() { exit = Abort;  break } }, // assert(..)
            // call
            CALLCODE | CALLSTATIC | CALLLIB | CALLLOC | CALL | CALLDYN => {
                let ist = instruction;
                macro_rules! not_ist {
                    ( $( $ist: expr ),+ ) => {
                        ![$( $ist ),+].contains(&ist)
                    }
                }
                // check call in mode 
                match mode {
                    Main    if not_ist!(CALL, CALLDYN, CALLSTATIC, CALLCODE)
                        => itr_err_code!(CallOtherInMain),
                    Abst    if not_ist!(CALLLIB, CALLSTATIC, CALLCODE)
                        => itr_err_code!(CallInAbst),
                    Library if not_ist!(CALLLIB, CALLSTATIC, CALLCODE)
                        => itr_err_code!(CallLocInLib),
                    Static  if not_ist!(CALLSTATIC, CALLCODE)
                        => itr_err_code!(CallLibInStatic),
                    CodeCopy
                        => itr_err_code!(CallInCodeCopy), // cannot call again in code call mode 
                    _ => Ok(()), // Extenal | Location support all call instructions
                }?;
                // ok return
                match ist {
                    CALLCODE =>   exit = funcptr!(codes, *pc, tail, CodeCopy),
                    CALLSTATIC => exit = funcptr!(codes, *pc, tail, Static),
                    CALLLIB =>    exit = funcptr!(codes, *pc, tail, Library),
                    CALL =>       exit = funcptr!(codes, *pc, tail, External),
                    CALLLOC =>    exit = Call(Funcptr{
                        mode: Location,
                        target: CallTarget::Location,
                        fnsign: pcutbuf!(FN_SIGN_WIDTH),
                    }),
                    CALLDYN =>    exit = Call(Funcptr{ // External
                        mode: External,
                        target: CallTarget::Addr(ops.pop()?.checked_contract_address()?),
                        fnsign: ops.pop()?.checked_fnsign()?,
                    }),
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