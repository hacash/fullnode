
pub trait TxExec {
    fn execute(&self, _: &mut dyn Context) -> Rerr { unimplemented!() }
}


pub trait TransactionRead : Serialize + TxExec + Send + Sync + DynClone { 
    fn ty(&self) -> u8 { unimplemented!() }


    fn hash(&self) -> Hash { unimplemented!() }
    fn hash_with_fee(&self) -> Hash { unimplemented!() }

    fn main(&self) -> Address { unimplemented!() }
    fn addrs(&self) -> Vec<Address> { unimplemented!() }

    fn timestamp(&self) -> &Timestamp { unimplemented!() }

    fn fee(&self) -> &Amount { unimplemented!() }
    fn fee_pay(&self) -> Amount { unimplemented!() }
    fn fee_got(&self) -> Amount { unimplemented!() }
    
    fn message(&self) -> &Fixed16 { unimplemented!() }
    fn reward(&self) -> &Amount { unimplemented!() }

    fn action_count(&self) -> &Uint2 { unimplemented!() }
    fn actions(&self) -> &Vec<Box<dyn Action>> { unimplemented!() }
    fn signs(&self) -> &Vec<Sign> { unimplemented!() }
    
    fn req_sign(&self) -> Ret<HashSet<Address>> { unimplemented!() }
    fn verify_signature(&self) -> Rerr { unimplemented!() }

    fn fee_purity(&self) -> u64 { unimplemented!() }

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

    fn set_fee(&mut self, _: Amount) { unimplemented!(); }
    fn set_nonce(&mut self, _: Hash) { unimplemented!(); }

    fn fill_sign(&mut self,_: &Account) -> Ret<Sign> { unimplemented!() }
    fn push_sign(&mut self,_: Sign) -> Rerr { unimplemented!() }
    fn push_action(&mut self, _: Box<dyn Action>) -> Rerr { unimplemented!() }

}


clone_trait_object!(TransactionRead);
clone_trait_object!(Transaction);


