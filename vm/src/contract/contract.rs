

pub struct Contract {
    argv: BytesW1,
    ctrt: ContractSto
}


impl Contract {

    pub fn serialize(&self) -> Vec<u8> {
        self.ctrt.serialize()
    }
    
    pub fn new() -> Self {
        Self {
            argv: BytesW1::new(),
            ctrt: ContractSto::new()
        }
    }

    pub fn lib(mut self, a: Address) -> Self {
        let adr = ContractAddress::from_addr(a).unwrap();
        self.ctrt.librarys.push(adr).unwrap();
        self
    }

    pub fn inh(mut self, a: Address) -> Self {
        let adr = ContractAddress::from_addr(a).unwrap();
        self.ctrt.inherits.push(adr).unwrap();
        self
    }

    pub fn syst(mut self, a: Abst) -> Self {
        self.ctrt.abstcalls.push(a.func).unwrap();
        self
    }

    pub fn func(mut self, a: Func) -> Self {
        self.ctrt.userfuncs.push(a.func).unwrap();
        self
    }

    pub fn argv(mut self, a: Vec<u8>) -> Self {
        self.argv = BytesW1::from(a).unwrap();
        self
    }

    pub fn testnet_deploy_print_by_nonce(&self, fee: &str, nonce: u32) {
        let txfee = Amount::from(fee).unwrap();
        let mut act = ContractDeploy::new();
        act.nonce = Uint4::from(nonce);
        act.contract = self.ctrt.clone();
        act.construct_argv = self.argv.clone();
        act.protocol_cost = txfee.dist_mul(CONTRACT_STORE_FEE_MUL as u128).unwrap();
        // print
        curl_trs_2(vec![Box::new(act)], fee);
    } 

    pub fn testnet_deploy_print(&self, fee: &str) {
        self.testnet_deploy_print_by_nonce(fee, 0)
    } 

    pub fn testnet_update_print(&self, cadr: Address, fee: &str) {
        let txfee = Amount::from(fee).unwrap();
        let mut act = ContractUpdate::new();
        act.contract = self.ctrt.clone();
        act.address = cadr;
        act.protocol_cost = txfee.dist_mul(CONTRACT_STORE_FEE_MUL as u128).unwrap();
        // print
        curl_trs_2(vec![Box::new(act)], fee);
    } 


}
