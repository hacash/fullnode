

pub struct Contract {
    pfee: Amount,
    ctrt: ContractSto
}


impl Contract {
    
    pub fn new() -> Self {
        Self {
            pfee: Amount::new(),
            ctrt: ContractSto::new()
        }
    }

    pub fn call(&mut self, a: AbstFunc) -> &mut Self {
        self.ctrt.abstcalls.push(a.func).unwrap();
        self
    }

    pub fn func(&mut self, a: UserFunc) -> &mut Self {
        self.ctrt.userfuncs.push(a.func).unwrap();
        self
    }

    pub fn pfee(&mut self, fe: &str) -> &mut Self {
        self.pfee = Amount::from(fe).unwrap();
        self
    }

    pub fn testnet_deploy_print(&self, fe: &str) {
        let mut act = ContractDeploy::new();
        act.contract = self.ctrt.clone();
        act.protocol_fee = self.pfee.clone();
        // print
        curl_trs_fee(vec![Box::new(act)], Amount::from(fe).unwrap());
    } 


}
