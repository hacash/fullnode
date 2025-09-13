use native::NativeCall;



macro_rules! print_sub {
    ($suo:expr, $subx:expr, $tab:expr, $desc:ident) => {
        match $subx.subs() {
            0 => $subx.print($suo, 0, $desc),
            _ => { let mut buf = s!("\n") + &$subx.print($suo, $tab+1, $desc);
                if $desc {
                    buf += &(s!("\n") + &$suo.repeat($tab));
                }
                buf
            }
        }
    };
}

macro_rules! print_sub_newline {
    ($suo:expr, $subx:expr, $tab:expr, $desc:ident) => {{
        let sub = $subx.print($suo, $tab+1, $desc);
        let emp = sub.replace(" ", "").replace("\n", "");
        match emp.len() > 0 {
            true => {
                let mut buf = s!("\n") + &sub;
                if $desc {
                    buf += &(s!("\n") + &$suo.repeat($tab));
                }
                buf
            },
            false => emp,
        }
        
    }}
}

macro_rules! print_sub_inline {
    ($suo:expr, $subx:expr, $desc:ident) => {{
        let substr = print_sub!($suo, $subx, 0, $desc);
        match substr.contains('\n') {
            true => substr[($suo.len()+1)..substr.len()-1].to_owned(),
            false => substr,
        }
    }};
}


macro_rules! print_subx_suby_op {
    ($suo:expr, $buf:ident, $self:ident, $desc:ident, $op:expr) => {
        {
            let mut subx = print_sub_inline!($suo, $self.subx, $desc);
            let mut suby = print_sub_inline!($suo, $self.suby, $desc);
            // check level
            let clv = match OpTy::from_bytecode($self.inst) {
                Ok(t) => t.level(),
                _ => 0,
            };
            let llv = $self.subx.level();
            let rlv = $self.suby.level();
            if clv>0&&llv>0 && clv>llv {
                subx = format!("({})", &subx);
            }
            if clv>0&&rlv>0 && clv>rlv {
                suby = format!("({})", &suby);
            }
            let res = format!("{} {} {}", &subx, $op, &suby);
            $buf.push_str(&res);
        }
    };
}



/*************************************/



#[derive(Debug, Clone)]
pub struct IRNodeEmpty {}

impl IRNode for IRNodeEmpty {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { false }
    fn bytecode(&self) -> u8 { 0 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> { Ok(vec![]) }
}


#[derive(Debug, Clone)]
pub struct IRNodeText {
    pub text: String
}

impl IRNode for IRNodeText {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { false }
    fn bytecode(&self) -> u8 { 0 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> { Ok(vec![]) }
}

impl IRNodeText {
    pub fn into_text(self) -> String {
        self.text
    }
}



/*************************************/


#[derive(Debug, Clone)]
pub struct IRNodeLeaf {
    pub hrtv: bool, 
    pub inst: Bytecode,
}

impl IRNode for IRNodeLeaf {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn serialize(&self) -> Vec<u8> { vec![self.inst as u8] }
    fn print(&self, sou: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(sou.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                NOP => {}, 
                P0 => buf.push('0'),
                P1 => buf.push('1'),
                PNBUF => buf.push_str("\"\""),
                _ => {
                    buf.push_str(meta.intro);
                    buf.push_str("()");
                }
            };
        } else {
            buf.push_str(&format!("{:?}", self.inst));
        }
        buf
    }

}

impl IRNodeLeaf {
    pub fn nop() -> Self {
        Self { hrtv: false, inst: Bytecode::NOP }
    }
    pub fn nop_box() -> Box<dyn IRNode> {
        Box::new(Self::nop())
    }
}


/*************************************/


#[derive(Debug, Clone)]
pub struct IRNodeParam1 {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: u8,
    pub text: String,
}

impl IRNode for IRNodeParam1 {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::once::<u8>(self.bytecode())
        .chain([self.para])
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        self.codegen().unwrap()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                PU8 =>  buf.push_str(&format!("{}", self.para)),
                GETX => buf.push_str(&format!("${}", self.para)),
                EXTENV => {
                    let ary = CALL_EXTEND_ENV_DEFS;
                    let mut idx = self.para as usize;
                    if idx >= ary.len() { idx = 0; }
                    buf.push_str(&format!("{}()", ary[idx].0));
                },
                _ => {
                    buf.push_str(&format!("{}({})", meta.intro, self.para));
                }
            };
        } else {
            buf.push_str(&format!("{:?} {}", self.inst, self.para));
        }
        buf
    }
}

impl IRNodeParam1 {
    pub fn into_text(self) -> String {
        self.text
    }
    pub fn to_text(&self) -> String {
        self.text.clone()
    }
    pub fn as_text(&self) -> &String {
        &self.text
    }
}


#[derive(Debug, Clone)]
pub struct IRNodeParam2 {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: [u8; 2],
}

impl IRNode for IRNodeParam2 {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::once::<u8>(self.bytecode())
        .chain(self.para)
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        self.codegen().unwrap()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                PU16 => buf.push_str(&format!("{}", u16::from_be_bytes(self.para))) ,
                _ => {
                    buf.push_str(meta.intro);
                    buf.push_str(&format!("(0x{})", hex::encode(self.para)));
                }
            };
        } else {
            buf.push_str(&format!("{:?} {} {}", self.inst, self.para[0], self.para[1]));
        }
        buf
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParams {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: Vec<u8>,
}

impl IRNode for IRNodeParams {
    fn as_any(&self) -> &dyn Any { self }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::once::<u8>(self.bytecode())
        .chain(self.para.clone())
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        self.codegen().unwrap()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        let parastr = hex::encode(&self.para);
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                PBUF | PBUFL => print_data_bytes(self, &mut buf),
                CALLCODE => {
                    let i = self.para[0];
                    let f = hex::encode(&self.para[1..]);
                    buf.push_str(&format!("{} <{}>::<{}>", meta.intro, i, f));
                }
                _ => {
                    buf.push_str(&format!("{}(0x{})", meta.intro, parastr));
                }
            }
        } else {
            buf.push_str(&format!("{:?} 0x{}", self.inst, parastr));
        }
        buf
    }
}

fn print_data_bytes(this: &IRNodeParams, buf: &mut String) {
    let l = maybe!(PBUF==this.inst, 1, 2) as usize;
    let data = this.para[l..].to_vec();
    if let Some(s) = ascii_show_string(&data) {
        buf.push_str(&format!("\"{}\"", literals(s)));
        return
    }
    // check address
    if data.len() == Address::SIZE {
        let addr = Address::from_vec(data.clone());
        if let Ok(..) = addr.check_version() {
            buf.push_str(&format!("{}", addr.readable()));
            return 
        }
    }
    // normal data
    buf.push_str(&format!("0x{}", hex::encode(&data)));
}







#[derive(Debug, Clone)]
pub struct IRNodeParamsSingle {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: Vec<u8>,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParamsSingle {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 1 }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>>{
        iter::empty::<u8>()
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .chain(self.para.clone())
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain(self.para.clone())
        .chain(self.subx.serialize())
        .collect()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        let parastr = hex::encode(&self.para);
        if desc {
            let meta = self.inst.metadata();
            let substr = print_sub_inline!(suo, self.subx, desc);
            match self.inst {
                CALL => {
                    let lx = Address::SIZE;
                    let adr = Address::from_vec(self.para[0..lx].to_vec());
                    let fun = hex::encode(&self.para[lx..]);
                    buf.push_str(&format!("{}.<{}>({})", adr.readable(), fun, substr));
                }
                CALLINR => {
                    let f = hex::encode(&self.para);
                    buf.push_str(&format!("self.<{}>({})", f, substr));
                }
                CALLLIB | CALLSTATIC => {
                    let clt = maybe!(CALLLIB==self.inst, ":", "::");
                    let i = self.para[0];
                    let f = hex::encode(&self.para[1..]);
                    buf.push_str(&format!("<{}>{}<{}>({})", i, clt, f, substr));
                }
                _ => {
                    buf.push_str(&format!("{}(0x{}, {})", meta.intro, parastr, substr));
                }
            }
        } else {
            let substr = print_sub!(suo, self.subx, tab, desc);
            buf.push_str(&format!("{:?} 0x{} {}", self.inst, parastr, substr));
        }
        buf
    }
}


/*************************************/


#[derive(Debug, Clone)]
pub struct IRNodeWrapOne {
    pub node: Box<dyn IRNode>,
}

impl std::ops::Deref for IRNodeWrapOne {
    type Target = Box<dyn IRNode>;
    fn deref(&self) -> &Box<dyn IRNode> {
        &self.node
    }
}

impl std::ops::DerefMut for IRNodeWrapOne {
    fn deref_mut(&mut self) -> &mut Box<dyn IRNode> {
        &mut self.node
    }
}

impl IRNode for IRNodeWrapOne {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 0 }
    fn hasretval(&self) -> bool { self.node.hasretval() }
    fn bytecode(&self) -> u8 { self.node.bytecode() }
    fn codegen(&self) -> VmrtRes<Vec<u8>> { self.node.codegen() }
    fn serialize(&self) -> Vec<u8> { self.node.serialize() }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let res = self.node.print(suo, tab, desc);
        maybe!(desc, format!("({})", res), res)
    }
}

/*************************************/



#[derive(Debug, Clone)]
pub struct IRNodeSingle {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeSingle {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 1 }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::empty::<u8>()
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain(self.subx.serialize())
        .collect()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                CU8 | CU16 | CU32 | CU64 | CU128 | NOT | RET | ERR | AST => {
                    let substr = print_sub_inline!(suo, self.subx, desc);
                    match self.inst {
                        CU8   => buf.push_str(&format!("{} as u8", substr)),
                        CU16  => buf.push_str(&format!("{} as u16", substr)),
                        CU32  => buf.push_str(&format!("{} as u32", substr)),
                        CU64  => buf.push_str(&format!("{} as u64", substr)),
                        CU128 => buf.push_str(&format!("{} as u128", substr)),
                        NOT   => buf.push_str(&format!("! {}", substr)),
                        RET | ERR | AST => buf.push_str(&format!("{} {}", meta.intro, substr)),
                        _ => {}
                    };
                },
                _ => {
                    let substr = print_sub_inline!(suo, self.subx, desc);
                    buf.push_str(&format!("{}({})", meta.intro, substr));
                }
            };
        } else {
            let substr = print_sub!(suo, self.subx, tab, desc);
            buf.push_str(&format!("{:?} {}", self.inst, substr));
        }
        buf
    }

}


#[derive(Debug, Clone)]
pub struct IRNodeDouble {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
}

impl IRNode for IRNodeDouble {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn subs(&self) -> usize { 2 }
    fn level(&self) -> u8 {
        match OpTy::from_bytecode(self.inst) {
            Ok(t) => t.level(),
            _ => 0,
        }
    }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        if let Some(c) = compile_double(self.inst, &self.subx, &self.suby)? {
            return Ok(c)
        }
        iter::empty::<u8>()
        .chain(self.suby.codegen()?)
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain(self.subx.serialize())
        .chain(self.suby.serialize())
        .collect::<Vec<u8>>()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                ADD 
                | SUB 
                | MUL 
                | DIV 
                | POW 
                | MOD 
                | GT  
                | LT  
                | GE  
                | LE  
                | AND 
                | OR  
                | EQ  
                | NEQ 
                | BAND
                | BOR 
                | BXOR
                | BSHL
                | BSHR
                | CAT 
                => {
                    let sg = OpTy::from_bytecode(self.inst).unwrap().symbol();
                    print_subx_suby_op!(suo, buf, self, desc, sg)
                }
                IRWHILE => {
                    let subxstr = print_sub_inline!(suo, self.subx, desc);
                    let subystr = print_sub!(suo, self.suby, tab, desc);
                    buf.push_str(&format!("while {} {{{}}}", subxstr, subystr))
                }
                _ => {
                    let subxstr = print_sub!(suo, self.subx, tab, desc);
                    let subystr = print_sub!(suo, self.suby, tab, desc);
                    buf.push_str(&format!("{}({}, {})", meta.intro, subxstr, subystr))
                }
            }
        } else {
            let subxstr = print_sub!(suo, self.subx, tab, desc);
            let subystr = print_sub!(suo, self.suby, tab, desc);
            buf.push_str(&format!("{:?} {} {}", self.inst, subxstr, subystr));
        }
        buf
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeTriple {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
    pub subz: Box<dyn IRNode>,
}

impl IRNode for IRNodeTriple {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 3 }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        if let Some(c) = compile_triple(self.inst, &self.subx, &self.suby, &self.subz)? {
            return Ok(c)
        }
        iter::empty::<u8>()
        .chain(self.subz.codegen()?)
        .chain(self.suby.codegen()?)
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain(self.subx.serialize())
        .chain(self.suby.serialize())
        .chain(self.subz.serialize())
        .collect()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                IRIF => {
                    let subxstr = &print_sub_inline!(suo, self.subx, desc);
                    let subystr = print_sub_newline!(suo, self.suby, tab, desc);
                    let subzstr = print_sub_newline!(suo, self.subz, tab, desc);
                    buf.push_str(&format!("if {} {{{}}}", subxstr, subystr));
                    if subzstr.len() > 0 {
                        buf.push_str(&format!(" else {{{}}}", subzstr));
                    }
                }
                _ => {
                    let subxstr = print_sub!(suo, self.subx, tab, desc);
                    let subystr = print_sub!(suo, self.suby, tab, desc);
                    let subzstr = print_sub!(suo, self.subz, tab, desc);
                    buf.push_str(&format!("{}({}, {}, {})", meta.intro, subxstr, subystr, subzstr));
                }
            }
        } else {
            let subxstr = print_sub!(suo, self.subx, tab, desc);
            let subystr = print_sub!(suo, self.suby, tab, desc);
            let subzstr = print_sub!(suo, self.subz, tab, desc);
            buf.push_str(&format!("{:?} {} {} {}", self.inst, subxstr, subystr, subzstr));
        }
        buf
    }
}


/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeParam1Single {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: u8,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam1Single {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 1 }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::empty()
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .chain([self.para])
        .collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain([self.para])
        .chain(self.subx.serialize())
        .collect()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        let substr = print_sub!(suo, self.subx, tab, desc);
        if desc {
            let meta = self.inst.metadata();
            match self.inst {
                PUTX => {
                    let substr = &print_sub_inline!(suo, self.subx, desc);
                    let line = format!("${} = {}", self.para, substr);
                    buf.push_str(&line);
                }
                XOP => {
                    let substr = &print_sub_inline!(suo, self.subx, desc);
                    let (opt, idx) = local_operand_param_parse(self.para);
                    let line = format!("${} {} {}", idx, opt, substr);
                    buf.push_str(&line);
                }
                XLG => {
                    let substr = &print_sub_inline!(suo, self.subx, desc);
                    let (opt, idx) = local_logic_param_parse(self.para);
                    let line = format!("${} {} {}", idx, opt, substr);
                    buf.push_str(&line);
                }
                EXTFUNC => {
                    let substr = &print_sub_inline!(suo, self.subx, desc);
                    let ary = CALL_EXTEND_FUNC_DEFS;
                    let mut idx = self.para as usize;
                    if idx >= ary.len() { idx = 0; }
                    buf.push_str(&format!("{}({})", ary[idx].0, substr));
                }
                NTCALL => {
                    let substr = &print_sub_inline!(suo, self.subx, desc);
                    let ntcall: NativeCall = std_mem_transmute!(self.para);
                    buf.push_str(&format!("{}({})", ntcall.name(), substr));
                }
                _ => {
                    buf.push_str(meta.intro);
                    buf.push_str(&format!("({}, {})", self.para, substr));
                }
            };
        } else {
            buf.push_str(&format!("{:?} {} {}", self.inst, self.para, substr));
        }
        buf
    }
}


#[derive(Debug, Clone)]
pub struct IRNodeParam2Single {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub para: [u8; 2],
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam2Single {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 1 }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        iter::empty::<u8>()
        .chain(self.subx.codegen()?)
        .chain([self.bytecode()])
        .chain(self.para).collect::<Vec<u8>>().into_vmrt()
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
        .chain(self.para)
        .chain(self.subx.serialize())
        .collect()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let mut buf = String::from(suo.repeat(tab));
        let parastr = hex::encode(&self.para);
        let substr = print_sub!(suo, self.subx, tab, desc);
        if desc {
            let meta = self.inst.metadata();
            buf.push_str(meta.intro);
            buf.push_str(&format!("(0x{}, {})", parastr, substr));
        } else {
            buf.push_str(&format!("{:?} 0x{} {}", self.inst, parastr, substr));
        }
        buf
    }
}



/*************************************/


#[derive(Default, Debug, Clone)]
pub struct IRNodeBytecodes {
    pub inst: Bytecode,
    pub codes: Vec<u8>,
}

impl IRNode for IRNodeBytecodes {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { 0 }
    fn hasretval(&self) -> bool { false }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        Ok(self.codes.clone())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.inst as u8)
            .chain((self.codes.len() as u16).to_be_bytes())
            .chain(self.codes.clone())
            .collect::<Vec<_>>()
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let pre = suo.repeat(tab);
        let mut buf = String::new();
        if desc {
        }else{
            buf.push_str(&format!("{}bytecode {{ ", pre));
        }
        buf.push_str(&self.codes.bytecode_print(false).unwrap());
        buf.push_str(" }}");
        buf
    }
}



/*************************************/


#[derive(Default, Debug, Clone)]
pub struct IRNodeBlock {
    pub hrtv: bool, 
    pub inst: Bytecode,
    pub subs: Vec<Box<dyn IRNode>>,
}

impl std::ops::Deref for IRNodeBlock {
    type Target = Vec<Box<dyn IRNode>>;
    fn deref(&self) -> &Vec<Box<dyn IRNode>> {
        &self.subs
    }
}

impl DerefMut for IRNodeBlock {
    fn deref_mut(&mut self) -> &mut Vec<Box<dyn IRNode>> {
        &mut self.subs
    }
}

impl IRNode for IRNodeBlock {
    fn as_any(&self) -> &dyn Any { self }
    fn subs(&self) -> usize { self.subs.len() }
    fn hasretval(&self) -> bool { self.hrtv }
    fn bytecode(&self) -> u8 { self.inst as u8 }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        compile_block(&self.subs)
    }
    fn serialize(&self) -> Vec<u8> {
        if self.subs.len() > u16::MAX as usize {
            panic!("IRNodeBlock length overflow")
        }
        let mut bytes = iter::once(self.inst as u8)
            .chain((self.subs.len() as u16).to_be_bytes()).collect::<Vec<_>>();
        for a in &self.subs {
            bytes.append(&mut a.serialize());
        }
        bytes
    }
    fn print(&self, suo: &str, tab: usize, desc: bool) -> String {
        let pre = suo.repeat(tab);
        let num = self.subs.len();
        let mut buf = String::new();
        if desc {
        }else{
            buf.push_str(&format!("{}{:?} {} :\n", pre, self.inst, num));
        }
        if num == 0 {
            return buf
        }
        for a in &self.subs {
            buf.push_str(&a.print(suo, tab, desc));
            buf.push('\n');
        }
        buf.pop();
        if desc {
            buf.push_str(&pre);
        }else{
        }
        buf
    }
}


#[allow(dead_code)]
impl IRNodeBlock {
    pub fn new() -> IRNodeBlock {
        IRNodeBlock{
            hrtv: false, 
            inst: IRBLOCK,
            subs: vec![],
        }
    }
    pub fn with_capacity(n: usize) -> IRNodeBlock {
        IRNodeBlock{
            hrtv: false, 
            inst: IRBLOCK,
            subs: Vec::with_capacity(n),
        }
    }
    pub fn into_vec(self) -> Vec<Box<dyn IRNode>> {
        self.subs
    }
    pub fn into_one(mut self) -> Box<dyn IRNode> {
        self.subs.pop().unwrap()
    }
}


/*******************************/



pub trait IRNodePrint {
    fn irnode_print(&self, desc: bool) -> VmrtRes<String>;
}

impl IRNodePrint for Vec<u8> {

    fn irnode_print(&self, desc: bool) -> VmrtRes<String> {
        let irs = parse_ir_block(self, &mut 0)?;
        let res = irs.print("    ", 0, desc);
        Ok(res)
    }

}







/*************************/




fn literals(s: String) -> String {
    s.replace("\\", "\\\\")
    .replace("\t", "\\t")
    .replace("\n", "\\n")
    .replace("\r", "\\r")
    .replace("\"", "\\\"")
}