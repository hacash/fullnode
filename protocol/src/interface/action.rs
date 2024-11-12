
pub trait Action : Field + Send + Sync + DynClone {
    fn kind(&self) -> u16 { unimplemented!() }
    fn level(&self) -> i8 { ActionLevel::MAIN }
    fn gas(&self) -> i64 { 0 } // fixed gas use
    fn burn_90(&self) -> bool { false } // is_burning_90_persent_fee
    fn req_sign(&self, _: &Vec<Address>) -> Vec<Address> { vec![] } // request_need_sign_addresses
    
}

clone_trait_object!(Action);


