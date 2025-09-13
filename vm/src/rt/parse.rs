

#[macro_export]
macro_rules! build_codes {
    ($( $b:expr )+) => {  {
        vec![
            $(
                $b as u8
            ),+
        ]
    } }
}





pub trait BytecodePrint {
    fn bytecode_print(&self, desc: bool) -> VmrtRes<String>;
}

impl BytecodePrint for Vec<u8> {

    fn bytecode_print(&self, desc: bool) -> VmrtRes<String> {
        let mut res = String::new();
        let mut jpdests = vec![];
        if desc  {
            jpdests = scan_jump_dests(&self);
            res.push_str(&format!("block: {:?}\nentry:\n", jpdests));
        }
        let max = self.len();
        let mut i = 0;
        macro_rules! pu16 {
            ($i:expr) => {
                u16::from_be_bytes(self[$i..$i+2].try_into().unwrap())
            };
        }
        while i < max {
            let byte = self[i];
            let inst: Bytecode = std_mem_transmute!(byte);
            let meta = inst.metadata();
            if desc {
                let mut line = maybe!(jpdests.contains(&i), format!("#{}:\n    ", i), s!("    "));
                match inst {
                    P0 => line += &format!("{} · ", 0),
                    P1 => line += &format!("{} · ", 1),
                    PU8 => line += &format!("{} · ", self[i+1]),
                    PU16 => line += &format!("{} · ", pu16!(i+1)),
                    RET | END | ERR | ABT => line += "--",
                    JMPL | JMPS | JMPSL => line += "# ",
                    BRL | BRS | BRSL | BRSLN => line += "?# ",
                    _ => {},
                }
                res.push_str(&format!("{}{}", line, meta.intro));
            }else{
                res.push_str(&format!("{:?} ", inst));
            }
            if ! meta.valid {
                return itr_err_fmt!(InstInvalid, "bytecode_print err of inst {}", byte)
            }
            i += 1;
            if meta.param > 0 {  
                let mut pms = vec![];  
                let mut nmpm = ||{
                    for k in 0..meta.param {
                        pms.push(format!("{}", self[i+k as usize]));
                    }
                }; 
                if desc {
                    res.push_str("[");
                    if let JMPS | BRS = inst {
                        let s = self[i];
                        pms.push(format!(" -#{}- ", s as isize + 1));
                    }else if let JMPL | BRL = inst {
                        let s = pu16!(i) as i16;
                        pms.push(format!(" -#{}- ", s as isize + 2));
                    }else if let JMPSL | BRSL | BRSLN = inst {
                        let s = pu16!(i) as i16;
                        pms.push(format!(" -#{}- ", i as isize + s as isize + 2));
                    }else if let EXTENV = inst {
                        let ary = CALL_EXTEND_ENV_DEFS;
                        let mut idx = self[i] as usize;
                        if idx >= ary.len() { idx = 0; }
                        pms.push(format!(" {}() ", ary[idx].0));
                    }else if let XOP = inst {
                        let (opt, idx) = local_operand_param_parse(self[i]);
                        pms.push(format!("{}, {}", idx, opt));
                    }else if let XLG = inst {
                        let (opt, idx) = local_logic_param_parse(self[i]);
                        pms.push(format!("{}, {}", opt, idx));
                    }else if let EXTFUNC = inst {
                        let ary = CALL_EXTEND_FUNC_DEFS;
                        let mut idx = self[i] as usize;
                        if idx >= ary.len() { idx = 0; }
                        pms.push(format!(" {}(..) ", ary[idx].0));
                    }else if let CALL = inst {
                        let lx = Address::SIZE;
                        let addr = Address::from_vec(self[i..i+lx].to_vec());
                        let func = hex::encode(&self[i+lx..i+lx+4]);
                        pms.push(format!(" {}.<{}> ", addr.readable(), func));
                    }else{
                        nmpm();
                    }
                }else{
                    nmpm();
                }
                if let PBUF = inst {
                    let n = self[i] as usize + 1;
                    i += 1;
                    let r = i + n;
                    pms.push(format!("0x{}", hex::encode(&self[i..r])));
                    i += n-1;
                } else if let PBUFL = inst {
                    let n = u16::from_be_bytes(self[i..i+2].try_into().unwrap()) as usize + 1;
                    i += 2;
                    let r = i + n;
                    pms.push(format!("0x{}", hex::encode(&self[i..r])));
                    i += n-1;
                }
                if desc {
                    res.push_str(&pms.join(","));
                    res.push_str("]");
                }else{
                    res.push_str(&pms.join(" "));
                    res.push_str(" ");
                }
            }       
            i += meta.param as usize;
            if desc {
                res.push_str("\n");
            }
        }
        Ok(res)
    }
}


/*
    return block mark
*/
fn scan_jump_dests(codes: &[u8]) -> Vec<usize> {
    let mut dests = vec![];
    let cdl = codes.len();
    let mut i = 0;
    macro_rules! adddest { ($jt:expr) => {{
        dests.push($jt as usize)
    }}}
    macro_rules! pu8 { () => {{
        codes[i as usize]
    }}}
    macro_rules! pi8 { () => {
        pu8!() as i8
    }}
    macro_rules! pu16 { () => {{
        let r = i + 2;
        u16::from_be_bytes(codes[i as usize..r as usize].try_into().unwrap())
    }}}
    macro_rules! pi16 { () => {
        pu16!() as i16
    }}
    while i < cdl {
        let inst: Bytecode = std_mem_transmute!(codes[i]);
        let meta = inst.metadata();
        i += 1;
        match inst {
            PBUF  => i += (pu8!() +1+1) as usize,
            PBUFL => i += (pu16!()+2+1) as usize,
            JMPL  | BRL  => adddest!(pu16!() + 2),
            JMPS  | BRS  => adddest!(i as isize + pi8!() as isize + 1),
            JMPSL | BRSL | BRSLN => adddest!(i as isize + pi16!() as isize + 2),
            _ => {},
        };
        i += meta.param as usize;
    }

    dests
}