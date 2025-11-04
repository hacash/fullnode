
pub trait ExtActCal {
    fn height(&self) -> u64; // ctx blk hei
    fn action_call(&mut self, _: u16, _: Vec<u8>) -> Ret<(u32, Vec<u8>)>;
}

pub trait StateOperat {
    fn state(&mut self) -> &mut dyn State;
    fn state_fork(&mut self) -> Arc<Box<dyn State>>;
    fn state_merge(&mut self, _: Arc<Box<dyn State>>);
    fn state_recover(&mut self, _: Arc<Box<dyn State>>);
    fn state_replace(&mut self, _: Box<dyn State>) -> Box<dyn State>;
}

pub trait Context : StateOperat + ExtActCal {
    fn clone_mut(&self) -> &mut dyn Context;
    fn as_ext_caller(&mut self) -> &mut dyn ExtActCal;
    fn env(&self) -> &Env;
    fn addr(&self, _:&AddrOrPtr) -> Ret<Address>;
    fn check_sign(&mut self, _: &Address) -> Rerr;
    fn depth(&mut self) -> &mut CallDepth;
    fn depth_set(&mut self, _: CallDepth);
    // fn depth_add(&mut self) { never!() }
    // fn depth_sub(&mut self) { never!() }
    fn tx(&self) -> &dyn TransactionRead;
    fn vm(&mut self) -> &mut dyn VM;
    fn vm_replace(&mut self, _: Box<dyn VM>) -> Box<dyn VM>;
    // tex
    fn tex_state(&mut self) -> &mut TexState;
    
}

