
pub trait Context {
    fn env(&self) -> &Env { unimplemented!() }
    fn addr(&self, _:&AddrOrPtr) -> Ret<Address> { unimplemented!() }
    fn state(&mut self) -> &mut dyn State { unimplemented!() }
    // fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }
    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

