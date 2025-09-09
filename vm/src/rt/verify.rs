
pub fn verify_bytecodes(codes: &[u8]) -> VmrtErr {
    use Bytecode::*;
    // check empty
    let cl = codes.len();
    if cl <= 0 {
        return itr_err_code!(CodeEmpty)
    }
    if cl > u16::MAX as usize {
        return itr_err_code!(CodeTooLong)
    }
    // check end
    let tail: Bytecode = std_mem_transmute!(codes[cl - 1]);
    if let RET | END | ERR | ABT = tail {} else {
        return itr_err_code!(CodeNotWithEnd)
    };
    // check valid
    let (instable, jumpdests) = verify_valid_instruction(codes)?;
    // check jump dests
    verify_jump_dests(&instable, &jumpdests)?;
    // ok finish
    Ok(())
}



/*

*/   
fn verify_valid_instruction(codes: &[u8]) -> VmrtRes<(Vec<u8>, Vec<isize>)> {
    // use Bytecode::*;
    let cdlen = codes.len();
    let mut instable = vec![0u8; cdlen];
    let mut jumpdest = vec![];
    let mut i = 0;
    while i < cdlen {
        let inst: Bytecode = std_mem_transmute!(codes[i]);
        let meta = inst.metadata();
        if ! meta.valid {
            return itr_err_code!(InstInvalid)
        }
        instable[i] = 1; // yes is valid instruction
        i += 1;
        macro_rules! pu8 { () => {{
            if i >= cdlen { return itr_err_code!(CodeOverflow) }
            codes[i as usize]
        }}}
        macro_rules! pu16 { () => {{
            let r = i + 2;
            if r > cdlen { return itr_err_code!(CodeOverflow) }
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
        i += meta.param as usize;
        // next
    }
    // finish orr
    Ok((instable, jumpdest))
}


// 
fn verify_jump_dests(instable: &[u8], jumpdests: &[isize]) -> VmrtErr {
    let itlen = instable.len();
    for jp in jumpdests {
        let j = *jp;
        if j < 0 || j > itlen as isize {
            return itr_err_code!(JumpOverflow)   
        }
        if 0 == instable[j as usize] {
            return itr_err_code!(JumpInDataSeg) 
        }
    }
    // finish
    Ok(())
}