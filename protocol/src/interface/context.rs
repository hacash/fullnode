
pub trait Context {
    fn env(&self) -> &Env { unimplemented!() }
    fn addr(&self, _:&AddrOrPtr) -> Ret<Address> { unimplemented!() }
    fn state(&mut self) -> &mut dyn State { unimplemented!() }
    fn state_replace(&mut self, _: Box<dyn State>) -> Box<dyn State> { unimplemented!() }
    fn check_sign(&mut self, _: &Address) -> Rerr { unimplemented!() }
    fn depth(&self) -> i8 { unimplemented!() }
    fn depth_set(&mut self, _: i8) { unimplemented!() }
    fn depth_add(&mut self) { unimplemented!() }
    fn depth_sub(&mut self) { unimplemented!() }
    fn tx(&self) -> &dyn TransactionRead { unimplemented!() }
    fn vm(&mut self) -> &mut dyn VMI { unimplemented!() }
    fn vm_replace(&mut self, _: Box<dyn VMI>) -> Box<dyn VMI> { unimplemented!() }
    
    fn action_call(&mut self, _: u16, _: Vec<u8>) -> Ret<(i64, Vec<u8>)> { unimplemented!() }

    fn fork_sub(&mut self) -> Box<dyn State> { unimplemented!() }
    fn swap_sub(&mut self, _: Box<dyn State>) -> Box<dyn State> { unimplemented!() }
    fn merge_sub(&mut self, _: Box<dyn State>) { unimplemented!() }
}

