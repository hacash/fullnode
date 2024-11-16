

pub struct ContextInst {
    pub env: Env,
    sta: Box<dyn State>,
}

impl ContextInst {
    pub fn new(env: Env, sta: Box<dyn State>) -> Self {
        Self{ env, sta }
    }
}


impl Context for ContextInst {
    fn env(&self) -> &Env {
        &self.env
    }
    
    fn state(&mut self) -> &mut dyn State {
        self.sta.as_mut()
    }

    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

