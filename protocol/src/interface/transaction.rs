
pub trait TxExec {
    fn execute(&self, _: u64) -> Rerr { unimplemented!() }
}


pub trait TransactionRead : Serialize + TxExec + Send + Sync + DynClone { 
    fn ty(&self) -> u8 { unimplemented!() }

    fn address(&self) -> Address { unimplemented!() }
    fn addrlist(&self) -> Vec<Address> { unimplemented!() }

    fn actions(&self) -> &Vec<Box<dyn Action>> { unimplemented!() }

    // burn_90_percent_fee
    fn burn_90(&self) -> bool {
        for act in self.actions() {
            if act.burn_90() {
                return true
            }
        }
        // not burn
        false
    }
}   


pub trait Transaction : TransactionRead + Field + Send + Sync {
    fn as_read(&self) -> &dyn TransactionRead;
}


clone_trait_object!(TransactionRead);
clone_trait_object!(Transaction);


