/*

    contract loader

*/


impl Resoure {

    pub fn load_contract(&mut self, vmsta: &mut VMState, addr: &ContractAddress) -> VmrtRes<Arc<ContractObj>> {
        use ItrErrCode::*;
        if let Some(c) = self.contracts.get(addr) {
            return Ok(c.clone())
        }
        if self.contracts.len() >= self.space_cap.load_contract {
            return itr_err_code!(OutOfLoadContract)
        }
        match vmsta.contract(addr) {
            Some(c) => {
                let cobj = Arc::new(c.into_obj()?);
                self.contracts.insert(addr.clone(), cobj.clone()); // cache
                Ok(cobj)
            },
            None => itr_err_fmt!(NotFindContract, "cannot find contract {}", addr.readable())
        }
    }


    fn load_fn_by_search_inherits(&mut self, vmsta: &mut VMState, addr: &ContractAddress, fnkey: FnKey) -> VmrtRes<Option<Arc<FnObj>>> {
        let csto = self.load_contract(vmsta, addr)?;
        macro_rules! do_get {($csto : expr) => (
            match fnkey {
                FnKey::Abst(s) => $csto.abstfns.get(&s),
                FnKey::User(u) => $csto.userfns.get(&u),
            }
        )}
        if let Some(c) = do_get!(csto) {
            return Ok(Some(c.clone()))
        }
        let inherits = csto.sto.inherits.list();
        if inherits.is_empty() {
            return Ok(None)
        }
        // search from inherits
        for ih in inherits {
            let csto = self.load_contract(vmsta, ih)?;
            if let Some(c) = do_get!(csto) {
                return Ok(Some(c.clone()))
            }
        }
        // not find
        return Ok(None)

    }

    fn load_fn_by_search_librarys(&mut self, vmsta: &mut VMState, srcadr: &ContractAddress, lib: u8, fnsg: FnSign) -> VmrtRes<(ContractAddress, Option<Arc<FnObj>>)> {
        use ItrErrCode::*;
        let csto = self.load_contract(vmsta, srcadr)?;
        let librarys = csto.sto.librarys.list();
        let libidx = lib as usize;
        if libidx <= librarys.len() {
            return itr_err_code!(CallLibOverflow)
        }
        let taradr = librarys.get(libidx).unwrap();
        let csto = self.load_contract(vmsta, taradr)?;
        Ok((taradr.clone(), csto.userfns.get(&fnsg).map(|f|f.clone())))
    }

    pub fn load_usrfun(&mut self, vmsta: &mut VMState, addr: &ContractAddress, fnsg: FnSign) -> VmrtRes<Option<Arc<FnObj>>> {
        self.load_fn_by_search_inherits(vmsta, addr, FnKey::User(fnsg))
    }


    pub fn load_abst(&mut self, vmsta: &mut VMState, addr: &ContractAddress, scty: AbstCall) -> VmrtRes<Option<Arc<FnObj>>> {
        self.load_fn_by_search_inherits(vmsta, addr, FnKey::Abst(scty))
    }

    pub fn load_must_call(&mut self, vmsta: &mut VMState, fptr: Funcptr, dstadr: &ContractAddress, srcadr: &ContractAddress) -> VmrtRes<(Option<ContractAddress>, Arc<FnObj>)> {
        use CallTarget::*;
        use ItrErrCode::*;
        match match fptr.target {
            Location => (None, self.load_usrfun(vmsta, dstadr, fptr.fnsign)?),
            Addr(ctxadr) => (None, self.load_usrfun(vmsta, &ctxadr, fptr.fnsign)?),
            Libidx(lib) => self.load_fn_by_search_librarys(vmsta, srcadr, lib, fptr.fnsign).map(|(a,b)|(Some(a), b))?,
        }  {
            (a, Some(b)) => Ok((a, b)),
            _ => itr_err_code!(CallNotExist), // 
        }
    }




}
