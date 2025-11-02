


macro_rules! define_func_codes {
    () => {
        
        pub fn fitsh(self, irs: &str) -> Ret<Self> {
            let tks = Tokenizer::new(irs.as_bytes());
            let sytax = Syntax::new(tks.parse()?);
            let irnodes = sytax.parse()?;
            self.irnode(irnodes)
        }

        pub fn irnode(self, irnodes: IRNodeBlock) -> Ret<Self> {
            let ircodes = irnodes.serialize();
            self.ircode(ircodes)
        }

        pub fn ircode(mut self, ircodes: Vec<u8>) -> Ret<Self> {
            // debug_println!("{}", ircodes.ircode_print(true).unwrap());
            let cds = convert_ir_to_bytecode(&ircodes)?;
            verify_bytecodes(&cds)?;
            self.func.cdty[0] |= CodeType::IRNode as u8;
            self.func.code = BytesW2::from(ircodes)?;
            Ok(self)
        }
        
        pub fn bytecode(mut self, cds: Vec<u8>) -> Ret<Self> {
            verify_bytecodes(&cds)?;
            self.func.cdty[0] |= CodeType::Bytecode as u8;
            self.func.code = BytesW2::from(cds).unwrap();
            Ok(self)
        }

    };
} 



#[allow(dead_code)]
pub struct Abst {
    func: ContractAbstCall
}


#[allow(dead_code)]
impl Abst {
    
    pub fn new(fnsg: AbstCall) -> Self {
        let mut func = ContractAbstCall::new();
        func.sign = Fixed1::from([fnsg.uint()]);
        Self { func }
    }

    define_func_codes!{}


}



#[allow(dead_code)]
pub struct Func {
    func: ContractUserFunc
}


#[allow(dead_code)]
impl Func {
    
    pub fn new(fname: &str) -> Self {
        let mut func = ContractUserFunc::new();
        func.sign = Fixed4::from(calc_func_sign(fname));
        Self { func }
    }

    define_func_codes!{}

    pub fn public(mut self) -> Self {
        self.func.cdty[0] |= FnConf::Public as u8;
        self
    }

    pub fn types(mut self, ret: Option<ValueTy>, params: Vec<ValueTy>) -> Self {
        self.func.pmdf = FuncArgvTypes::from_types(ret, params).unwrap();
        self
    }




}