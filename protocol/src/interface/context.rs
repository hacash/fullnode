
pub trait Context {
    fn env(&self) -> &Env { unimplemented!() }
    fn addr(&self, _:&AddrOrPtr) -> Ret<Address> { unimplemented!() }
    fn state(&mut self) -> &mut dyn State { unimplemented!() }
    fn check_sign(&mut self, _: &Address) -> Rerr { unimplemented!() }
    fn depth(&self) -> u8 { unimplemented!() }
    fn depth_set(&mut self, _: u8) { unimplemented!() }
    fn depth_add(&mut self) { unimplemented!() }
    fn depth_sub(&mut self) { unimplemented!() }
    fn tx(&self) -> &dyn TransactionRead { unimplemented!() }
    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

