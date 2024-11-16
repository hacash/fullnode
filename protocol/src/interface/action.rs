/*
*
*/
pub trait ActExec {
    // return: (more gas use, exec value)
    fn execute(&self, _: &mut dyn Context) -> Ret<(u32, Vec<u8>)> { unimplemented!() }
}


/*
*
*/
pub trait Action : ActExec + Field + Send + Sync + DynClone {
    fn kind(&self) -> u16 { unimplemented!() }
    fn level(&self) -> i8 { ActLv::MAINCALL }
    fn burn_90(&self) -> bool { false } // is_burning_90_persent_fee
    // fn req_sign(&self, _: &Vec<Address>) -> Vec<Address> { vec![] } // request_need_sign_addresses
}

clone_trait_object!(Action);





