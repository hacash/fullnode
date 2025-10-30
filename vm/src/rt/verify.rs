pub fn convert_and_check(cap: &SpaceCap, ctype: CodeType, codes: &[u8]) -> VmrtErr {
    use CodeType::*;
    let bytecodes = match ctype {
        IRNode =>  &runtime_irs_to_bytecodes(codes)?,
        Bytecode => codes
    };
    // check size
    if bytecodes.len() > cap.one_function_size {
        return itr_err_code!(CodeTooLong)
    }
    // verify inst
    verify_bytecodes(bytecodes)
}


pub fn verify_bytecodes(codes: &[u8]) -> VmrtErr {
    // check empty
    let cl = codes.len();
    if cl <= 0 {
        return itr_err_code!(CodeEmpty)
    }
    if cl > u16::MAX as usize {
        return itr_err_code!(CodeTooLong)
    }
    // check end
    check_tail_end(codes[cl - 1])?;
    // check inst valid
    let (instable, jumpdests) = verify_valid_instruction(codes)?;
    // check jump dests
    verify_jump_dests(&instable, &jumpdests)?;
    // ok finish
    Ok(())
}


fn check_tail_end(c: u8) -> VmrtErr {
    let tail: Bytecode = std_mem_transmute!(c);
    if let RET | END | ERR | ABT |
        CALLCODE | CALLSTATIC | CALLLIB | CALLINR | CALL // CALLDYN
    = tail {
        return Ok(())
    };
    // error
    itr_err_code!(CodeNotWithEnd)
}


/*

*/   
fn verify_valid_instruction(codes: &[u8]) -> VmrtRes<(Vec<u8>, Vec<isize>)> {
    // use Bytecode::*;
    let cdlen = codes.len(); // end/ret/err/abt in tail
    let mut instable = vec![0u8; cdlen];
    let mut jumpdest = vec![];
    let mut i = 0;
    let mut cur = 0u8;
    while i < cdlen {
        cur = codes[i];
        let inst: Bytecode = std_mem_transmute!(cur);
        let meta = inst.metadata();
        if ! meta.valid {
            return itr_err_fmt!(InstInvalid, "{}", inst as u8)
        }
        instable[i] = 1; // yes is valid instruction
        macro_rules! pu8 { () => {{
            if i >= cdlen { return itr_err_code!(InstParamsErr) }
            codes[i as usize]
        }}}
        macro_rules! pu16 { () => {{
            let r = i + 2;
            if r > cdlen { return itr_err_code!(InstParamsErr) }
            u16::from_be_bytes(codes[i as usize..r as usize].try_into().unwrap())
        }}}
        macro_rules! pi8 { () => {
            pu8!() as i8
        }}
        macro_rules! pi16 { () => {
            pu16!() as i16
        }}
        macro_rules! adddest { ($jt:expr) => {{
            jumpdest.push($jt)
        }}}
        i += meta.param as usize;
        match inst {
            // push buf
            PBUF  => i += ( pu8!()) as usize,
            PBUFL => i += (pu16!()) as usize,
            // jump record
            JMPL  | BRL  => adddest!(pu16!() as isize),
            JMPS  | BRS  => adddest!(i as isize + pi8!() as isize + 1),
            JMPSL | BRSL | BRSLN => adddest!(i as isize + pi16!() as isize + 2),
            _ => {}
        };
        if i >= cdlen {
            return itr_err_code!(InstParamsErr)
        }
        // next
        i += 1;
    }
    check_tail_end(cur)?; // check end
    // finish orr
    Ok((instable, jumpdest))
}


// 
fn verify_jump_dests(instable: &[u8], jumpdests: &[isize]) -> VmrtErr {
    let itlen = instable.len();
    let right = itlen as isize - 1;
    for jp in jumpdests {
        let j = *jp;
        if j < 0 || j > right {
            return itr_err_code!(JumpOverflow)   
        }
        if 0 == instable[j as usize] {
            return itr_err_code!(JumpInDataSeg) 
        }
    }
    // finish
    Ok(())
}