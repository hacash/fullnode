
pub struct VMEmpty {}
impl VMI for VMEmpty {}

/*
*/
pub struct ContextInst<'a> {
    pub env: Env,
    pub depth: u8,
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

    fn depth(&self) -> u8 { self.depth }
    fn depth_set(&mut self, d: u8) { self.depth = d }
    fn depth_add(&mut self) { self.depth += 1 }
    fn depth_sub(&mut self) { self.depth -= 1 }

    fn tx(&self) -> &dyn TransactionRead { self.txr }

    fn vm(&mut self) -> &mut dyn VMI { self.vmi.as_mut() }
    fn vm_set(&mut self, vm: Box<dyn VMI>) -> Box<dyn VMI> {
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

}

