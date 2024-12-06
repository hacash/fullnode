
pub struct VMEmpty {}
impl VMI for VMEmpty {}

/*
*/
pub struct ContextInst<'a> {
    pub env: Env,
    pub depth: i8,
    pub txr: &'a dyn TransactionRead,

    pub vmi: Box<dyn VMI>,

    sta: Box<dyn State>,
    check_sign_cache: HashMap<Address, Ret<bool>>,
}


impl ContextInst<'_> {

    pub fn new<'a>(env: Env, sta: Box<dyn State>, txr: &'a dyn TransactionRead) -> ContextInst<'a> {
        ContextInst{ env, sta, txr,
            depth: 0,
            check_sign_cache: HashMap::new(),
            vmi: Box::new(VMEmpty{}),
        }
    }

    pub fn into_state(self) -> Box<dyn State> {
        self.sta
    }
}


impl Context for ContextInst<'_> {
    fn env(&self) -> &Env { &self.env}

    fn state(&mut self) -> &mut dyn State { self.sta.as_mut() }
    fn state_replace(&mut self, sta: Box<dyn State>) -> Box<dyn State> {
        std::mem::replace(&mut self.sta, sta)
    }

    fn depth(&self) -> i8 { self.depth }
    fn depth_set(&mut self, d: i8) { self.depth = d }
    fn depth_add(&mut self) { self.depth += 1 }
    fn depth_sub(&mut self) { self.depth -= 1 }

    fn tx(&self) -> &dyn TransactionRead { self.txr }

    fn vm(&mut self) -> &mut dyn VMI { self.vmi.as_mut() }
    fn vm_replace(&mut self, vm: Box<dyn VMI>) -> Box<dyn VMI> {
        std::mem::replace(&mut self.vmi, vm)
    }
    
    fn addr(&self, ptr :&AddrOrPtr) -> Ret<Address> {
        ptr.real(&self.env.tx.addrs)
    }
    
    fn check_sign(&mut self, adr: &Address) -> Rerr {
        adr.must_privakey()?;
        if self.check_sign_cache.contains_key(adr) {
            return self.check_sign_cache[adr].clone().map(|_|())
        }
        let isok = transaction::verify_target_signature(adr, self.txr);
        self.check_sign_cache.insert(*adr, isok.clone());
        isok.map(|_|())
    }

    fn action_call(&mut self, k: u16, b: Vec<u8>) -> Ret<(i64, Vec<u8>)> {
        ctx_action_call(self, k, b)
    }

    fn fork_sub(&mut self) -> Box<dyn State> {
        ctx_state_fork_sub(self)
    }

    fn swap_sub(&mut self, sta: Box<dyn State>) -> Box<dyn State> {
        ctx_state_swap_sub(self, sta)
    }

    fn merge_sub(&mut self, old: Box<dyn State>){
        ctx_state_merge_sub(self, old)
    }


}

