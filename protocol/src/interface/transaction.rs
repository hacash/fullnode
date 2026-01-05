
static TX_NIL_SIGNS: OnceLock<Vec<Sign>> = OnceLock::new();
static TX_NIL_ACTIS: OnceLock<Vec<Box<dyn Action>>> = OnceLock::new();



pub trait TxExec {
    fn execute(&self, _: &mut dyn Context) -> Rerr { never!() }
}


pub trait TransactionRead : Serialize + TxExec + Send + Sync + DynClone { 
    fn ty(&self) -> u8;

    fn hash(&self) -> Hash;
    fn hash_with_fee(&self) -> Hash;

    fn main(&self) -> Address { ADDRESS_ZERO }
    fn addrs(&self) -> Vec<Address> { vec![] }

    fn timestamp(&self) -> &Timestamp { Timestamp::zero_ref() }

    fn fee(&self) -> &Amount { Amount::zero_ref() }
    fn fee_pay(&self) -> Amount { Amount::zero() }
    fn fee_got(&self) -> Amount { Amount::zero() }
    fn fee_extend(&self) -> Ret<(u16, Amount)> { err!("cannot get fee extend") }
    fn fee_purity(&self) -> u64 { 0 }
    
    fn message(&self) -> &Fixed16 { Fixed16::zero_ref() }
    fn reward(&self) -> &Amount { Amount::zero_ref() }

    fn action_count(&self) -> usize { 0 }
    fn actions(&self) -> &Vec<Box<dyn Action>> { TX_NIL_ACTIS.get_or_init(||vec![]) }
    fn signs(&self) -> &Vec<Sign> { TX_NIL_SIGNS.get_or_init(||vec![]) }
    
    fn req_sign(&self) -> Ret<HashSet<Address>> { errf!("cannot req sign") }
    fn verify_signature(&self) -> Rerr { errf!("failed") }

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

    fn set_fee(&mut self, _: Amount) {}
    fn set_nonce(&mut self, _: Hash) {}

    fn fill_sign(&mut self,_: &Account) -> Ret<Sign> { errf!("never") }
    fn push_sign(&mut self,_: Sign) -> Rerr { errf!("never") }
    fn push_action(&mut self, _: Box<dyn Action>) -> Rerr { errf!("never") }

}


clone_trait_object!(TransactionRead);
clone_trait_object!(Transaction);


