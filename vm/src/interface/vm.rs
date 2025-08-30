

/*
    VM manage
*/
pub trait VMMng {
    fn prepare(&self) -> Box<dyn VMInst> { unimplemented!() }
    fn reclaim(&self, _: Box<dyn VMInst>) { unimplemented!() }
}




/*
    VM 
*/
pub trait VMInst: Send + Sync {
    fn main_call(&mut self, _: &mut dyn Context, _irnds: &[u8]) -> Rerr { unimplemented!() }
    fn abst_call(&mut self, _: &mut dyn Context, _contract_addr: Address, _syscty: u8) -> Rerr { unimplemented!() }
}






