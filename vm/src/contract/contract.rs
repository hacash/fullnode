

pub struct Contract {
    ctrt: ContractSto
}


impl Contract {
    
    pub fn new() -> Self {
        Self {
            ctrt: ContractSto::new()
        }
    }

    pub fn call(&mut self, a: Abst) -> &mut Self {
        self.ctrt.abstcalls.push(a.func).unwrap();
        self
    }

    pub fn func(&mut self, a: Func) -> &mut Self {
        self.ctrt.userfuncs.push(a.func).unwrap();
        self
    }

    pub fn testnet_deploy_print(&self, fe: &str) {
        let mut act = ContractDeploy::new();
        act.contract = self.ctrt.clone();
        // print
        curl_trs_fee(vec![Box::new(act)], Amount::from(fe).unwrap());
    } 


}
