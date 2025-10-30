
/* 
pub fn _codes_verify(codes: &[u8]) -> VmrtErr {
    let mut pc: isize = 0;
    let max = codes.len() as isize;
    
    macro_rules! cof { ($i:expr) => {
        if $i as isize >= max { return itr_err_code!(CodeOverflow) }
    }}
    macro_rules! vrf { ($m:expr) => {{
        if ! $m.valid { return itr_err_fmt!(CallInvalid, "invalid bytecode")  }
    }}}
    macro_rules! pu8 { () => {{
        let r = pc + 1;
        cof!(r);
        codes[r as usize]
    }}}
    macro_rules! pi8 { () => {
        pu8!() as i8
    }}
    macro_rules! pu16 { () => {{
        let r = pc + 2;
        cof!(r);
        u16::from_be_bytes(codes[pc as usize..r as usize].try_into().unwrap())
    }}}
    macro_rules! pi16 { () => {
        pu16!() as i16
    }}
    macro_rules! jmpvrf { ($i:expr) => {{
        let pc = $i as usize;
        cof!(pc);
        let inst: Bytecode = std_mem_transmute!(codes[pc]);
        vrf!(inst.metadata());
    }}}

    loop {
        if pc >= max { break }
        let byte = codes[pc as usize];
        let inst: Bytecode = std_mem_transmute!(byte);
        let meta = inst.metadata();
        vrf!(meta);
        pc += 1;
        match inst {
            PBUF  => pc += (pu8!() ) as isize,
            PBUFL => pc += (pu16!()) as isize,
            JMPL  | BRL  => jmpvrf!(pu16!() + 2),
            JMPS  | BRS  => jmpvrf!(pc + pi8!() as isize + 1),
            JMPSL | BRSL | BRSLN => jmpvrf!(pc + pi16!() as isize + 2),
            _ => {},
        }
        pc += meta.param as isize;
    }
    Ok(())
}

*/
