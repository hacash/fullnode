


macro_rules! define_func_codes {
    () => {
        
        pub fn bytecode(mut self, cds: Vec<u8>) -> Self {
            self.func.cdty = Fixed1::from([CodeType::Bytecode as u8]);
            self.func.code = BytesW2::from(cds).unwrap();
            self
        }

        pub fn irnode(mut self, irs: &str) -> Ret<Self> {
            let tks = Tokenizer::new(irs.as_bytes());
            let sytax = Syntax::new(tks.parse()?);
            let irnodes = sytax.parse()?;
            let ircodes = irnodes.serialize();
            // debug_println!("{}", ircodes.irnode_print(true).unwrap());
            self.func.cdty = Fixed1::from([CodeType::IRNode as u8]);
            self.func.code = BytesW2::from(ircodes)?;
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


}