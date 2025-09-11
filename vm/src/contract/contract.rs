

pub struct Contract {
    cargv: BytesW1,
    ctrt: ContractSto
}


impl Contract {
    
    pub fn new() -> Self {
        Self {
            cargv: BytesW1::new(),
            ctrt: ContractSto::new()
        }
    }

    pub fn call(mut self, a: Abst) -> Self {
        self.ctrt.abstcalls.push(a.func).unwrap();
        self
    }

    pub fn func(mut self, a: Func) -> Self {
        self.ctrt.userfuncs.push(a.func).unwrap();
        self
    }

    pub fn cargv(mut self, a: Vec<u8>) -> Self {
        self.cargv = BytesW1::from(a).unwrap();
        self
    }

    pub fn testnet_deploy_print(&self, fe: &str) {
        let txfee = Amount::from(fe).unwrap();
        let mut act = ContractDeploy::new();
        act.contract = self.ctrt.clone();
        act.construct_argv = self.cargv.clone();
        act.protocol_cost = txfee.dist_mul(CONTRACT_STORE_FEE_MUL as u128).unwrap();
        // print
        curl_trs_fee(vec![Box::new(act)], txfee);
    } 


}
