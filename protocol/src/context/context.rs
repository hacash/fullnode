


pub struct ContextInst<'a> {
    pub env: Env,
    pub txr: &'a dyn TransactionRead,
    sta: Box<dyn State>,

    check_sign_cache: HashMap<Address, Ret<bool>>,
}

impl ContextInst<'_> {

    pub fn new<'a>(env: Env, sta: Box<dyn State>, txr: &'a dyn TransactionRead) -> ContextInst<'a> {
        ContextInst{ env, sta, txr,
            check_sign_cache: HashMap::new(),
        }
    }

    pub fn into_state(self) -> Box<dyn State> {
        self.sta
    }
}


impl Context for ContextInst<'_> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn addr(&self, ptr :&AddrOrPtr) -> Ret<Address> {
        ptr.real(&self.env.tx.addrs)
    }
    
    fn state(&mut self) -> &mut dyn State {
        self.sta.as_mut()
    }
    
    fn check_sign(&mut self, adr: &Address) -> Rerr {
        adr.check_must_privakey()?;
        if self.check_sign_cache.contains_key(adr) {
            return self.check_sign_cache[adr].clone().map(|_|())
        }
        let isok = transaction::verify_target_signature(adr, self.txr);
        self.check_sign_cache.insert(*adr, isok.clone());
        isok.map(|_|())
    }

    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

