


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
            self.func.cdty = Fixed1::from([CodeType::IRNode as u8]);
            self.func.code = BytesW2::from(ircodes)?;
            Ok(self)
        }
    };
} 



#[allow(dead_code)]
pub struct AbstFunc {
    func: ContractAbstCall
}


#[allow(dead_code)]
impl AbstFunc {
    
    pub fn new(fnsg: AbstCall) -> Self {
        let mut func = ContractAbstCall::new();
        func.sign = Fixed1::from([fnsg.uint()]);
        Self { func }
    }

    define_func_codes!{}


}



#[allow(dead_code)]
pub struct UserFunc {
    func: ContractUserFunc
}


#[allow(dead_code)]
impl UserFunc {
    
    pub fn new(fname: &str) -> Self {
        let mut func = ContractUserFunc::new();
        func.sign = Fixed4::from(calc_func_sign(fname));
        Self { func }
    }

    define_func_codes!{}


}