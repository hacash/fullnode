

pub struct ContextInst {
    pub env: Env,
    sta: Box<dyn State>,
}

impl ContextInst {
    pub fn new(env: Env, sta: Box<dyn State>) -> Self {
        Self{ env, sta }
    }

    pub fn into_state(self) -> Box<dyn State> {
        self.sta
    }
}


impl Context for ContextInst {
    fn env(&self) -> &Env {
        &self.env
    }

    fn addr(&self, ptr :&AddrOrPtr) -> Ret<Address> {
        ptr.real(&self.env.tx.addrs)
    }
    
    fn state(&mut self) -> &mut dyn State {
        self.sta.as_mut()
    }

    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

