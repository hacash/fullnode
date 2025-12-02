#[derive(Debug, Default)]
pub struct Maincall {
    // ctrt: ContractSto
    codes: Vec<u8>,
}




impl Maincall {

    pub fn new() -> Self {
        Self::default()
    }

    pub fn fitsh(mut self, s: &str) -> Ret<Self> {
        let codes = lang_to_bytecode(s)?;
        self.codes = codes;
        Ok(self)
    }

    pub fn testnet_call_print(self, fee: &str) {
        let act = ContractMainCall::from_bytecode(self.codes).unwrap();
        curl_trs_3(vec![Box::new(act)], fee);
    }

    

    
}






