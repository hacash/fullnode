

/*
    return gasuse, retval 
*/
pub fn sandbox_call(ctx: &mut dyn Context, contract: ContractAddress, funcname: String, params: &str) -> Ret<(i64, String)> {
    use rt::Bytecode::*;

    let hei = ctx.env().block.height;

    let mainaddr = ctx.env().tx.main.clone();
    let txinfo = &ctx.env().tx as *const TxInfo;
    let txinfo = txinfo as *mut TxInfo;
    unsafe {
        (*txinfo).swap_addrs(&mut vec![mainaddr, contract.into_addr()]);
    }

    // let mut pc: usize = 0;
    // let mut gas_limit: i64 = 65535; // 2000

    // let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    // let cadr = contract;
    

    let cap = SpaceCap::new(hei);
    // let param_len = param.len();
    // if param_len > cap.max_value_size {
    //     return errf!("call param size overflow")
    // }

    let gas_limit = cap.max_gas_of_tx as i64;
    let gas = &mut gas_limit.clone();

    let mut codes: Vec<u8> = vec![];
    parse_push_params(&mut codes, params)?;

    /* 
    // push param to operand stack
    if param_len > 0 {
        codes.push(PBUFL as u8);
        codes.append(&mut (param_len as u16).to_be_bytes().to_vec());
        codes.append(&mut param);
    }else {
        codes.push(PNBUF as u8);
    }
    // test 
    
    let mut param = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap().to_vec();
    codes.push(PBUF as u8);
    codes.push(param.len() as u8);
    codes.append(&mut param);
    codes.push(CTO as u8);
    codes.push(ValueTy::Addr as u8);

    */

    // call contract
    let fnsg = calc_func_sign(&funcname);
    codes.push(CALL as u8);
    codes.push(1); // lib idx
    codes.append(&mut fnsg.to_vec());
    codes.push(RET as u8); // return the value

    // debug_println!("sandbox call codes: {}", codes.bytecode_print(true).unwrap_or_default());

    // do callparam_len
    unsafe {
        let ctxptr = ctx as *mut dyn Context;
        let staptr = ctx.state() as *mut dyn State;
        let ctx: &mut dyn Context = &mut *ctxptr;
        let sta: &mut dyn State = &mut *staptr;  
        let sta = &mut VMState::wrap(sta);
        let mut exenv = ExecEnv{ ctx, sta, gas };
        // do execute
        let mut vmb = global_machine_manager().assign(hei);
        vmb.machine.as_mut().unwrap().main_call(&mut exenv, CodeType::Bytecode, codes)
    }.map(|v|(
        gas_limit-*gas, v.to_json()
    ))

}



fn parse_push_params(codes: &mut Vec<u8>, pms: &str) -> Rerr {
    macro_rules! push { ( $( $a: expr ),+) => { $( codes.push($a as u8) );+ } }
    use Bytecode::*;
    let pms: Vec<_> = pms.split(",").collect();
    let pms: usize = pms.iter().map(|a|{
        let s: Vec<_> = a.split(":").collect();
        let n = s.len();
        let v = maybe!(n>=1, s[0], "");
        let t = maybe!(n>=2, s[1], "nil");
        parse_one_param(codes, t, v)
    }).sum();
    match pms {
        0      => { push!(PNIL); } // none argv
        1      => { /* already push in parse_one_param */ }
        2..255 => { push!(PU8, pms, PACKLIST); } 
        255..  => return errf!("param number is too much"),
    }
    Ok(())
}


fn parse_one_param(codes: &mut Vec<u8>, t: &str, v: &str) -> usize {
    // debug_println!("parse_one_param {}, {}", t, v);
    use Bytecode::*;
    use ValueTy::*;
    macro_rules! push { ( $( $a: expr ),+) => { $( codes.push($a as u8) );+ } }
    let ty = ValueTy::from_name(t);
    let Ok(ty) = ty else {
        return 0
    };
    match ty {
        Nil  => push!(PNIL),
        Bool => push!(maybe!(v=="true", P1, P0)),
        U8   => if let Ok(n) = v.parse::<u8>() {
            push!(PU8, n);
        },
        U16   => if let Ok(n) = v.parse::<u16>() {
            push!(PU16);
            codes.append(&mut Vec::from(n.to_be_bytes()));
        },
        U32   => if let Ok(n) = v.parse::<u32>() {
            push!(PBUF, 4);
            codes.append(&mut Vec::from(n.to_be_bytes()));
            push!(CU32);
        },
        U64   => if let Ok(n) = v.parse::<u64>() {
            push!(PBUF, 8);
            codes.append(&mut Vec::from(n.to_be_bytes()));
            push!(CU64);
        },
        U128   => if let Ok(n) = v.parse::<u128>() {
            push!(PBUF, 16);
            codes.append(&mut Vec::from(n.to_be_bytes()));
            push!(CU128);
        },
        Address => if let Ok(adr) = field::Address::from_readable(v) {
            push!(PBUF, field::Address::SIZE);
            codes.append(&mut adr.into_vec());
            push!(CTO, ty);
        },
        Bytes => if let Ok(mut bts) = hex::decode(v) {
            push!(PBUF, bts.len());
            codes.append(&mut bts);
        },
        _ => return 0
    };
    // yes
    1
}