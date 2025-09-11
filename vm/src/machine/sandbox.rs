

/*
    return gasuse, retval 
*/
pub fn sandbox_call(ctx: &mut dyn Context, contract: ContractAddress, funcname: String, mut param: Vec<u8>) -> Ret<(i64, Vec<u8>)> {
    use rt::Bytecode::*;

    let hei = ctx.env().block.height;

    // let mut pc: usize = 0;
    // let mut gas_limit: i64 = 65535; // 2000

    // let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    // let cadr = contract;
    

    let cap = SpaceCap::new(hei);
    let param_len = param.len();
    if param_len > cap.max_value_size {
        return errf!("call param size overflow")
    }

    let gas_limit = cap.max_gas_of_tx as i64;
    let gas = &mut gas_limit.clone();

    let mut codes: Vec<u8> = vec![];
    // push param to operand stack
    if param_len > 0 {
        codes.push(PBUFL as u8);
        codes.append(&mut (param_len as u16 - 1).to_be_bytes().to_vec());
        codes.append(&mut param);
    }else {
        codes.push(PNBUF as u8);
    }
    // call contract
    let fnsg = calc_func_sign(&funcname);
    codes.push(CALL as u8);
    codes.append(&mut contract.into_vec());
    codes.append(&mut fnsg.to_vec());
    codes.push(RET as u8); // return the value
    // println!("call codes: {}", codes.hex());

    // do callparam_len
    unsafe {
        let ctxptr = ctx as *mut dyn Context;
        let staptr = ctx.state() as *mut dyn State;
        let ctx: &mut dyn Context = &mut *ctxptr;
        let sta: &mut dyn State = &mut *staptr;  
        let sta = &mut VMState::wrap(sta);
        let mut exenv = ExecEnv{ ctx, sta, gas };
        // do execute
        let mut vmb = MACHINE_MANAGER.assign(hei);
        vmb.machine.as_mut().unwrap().main_call(&mut exenv, CodeType::Bytecode, codes)
    }.map(|v|(
        gas_limit-*gas, v.raw()
    ))

}



