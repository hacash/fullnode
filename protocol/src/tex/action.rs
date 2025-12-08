
/*
* simple hac to
*/
action_define!{ TexCellAct, 35, 
    ActLv::Top, // level
    false, // burn 90 fee
    [], // need sign
    {
        addr  : Address
        cells : DnyTexCellW1
        sign  : Sign
    },
    (self, ctx, _gas {
        self.addr.must_privakey()?;
        // check signature
        let thx = self.get_sign_stuff();
        if ! verify_signature(&thx, &self.addr, &self.sign) {
            return errf!("address {} signature verify failed in tex cell action", self.addr.readable())
        }
        // exec
        self.cells.execute(ctx, &self.addr).map(|_|vec![])
    })
}


impl TexCellAct {

    fn get_sign_stuff(&self) -> Hash {
        let stf = vec![self.addr.serialize(), self.cells.serialize()].concat();
        Hash::from(sha3(&stf))
    } 

    pub fn create_by(addr: Address) ->  Self {
        Self {
            addr,
            ..Self::new()
        }
    }
    
    pub fn do_sign(&mut self, acc: &Account) -> Rerr {
        acc.check_addr(self.addr.as_bytes())?;
        let thx = self.get_sign_stuff();
        self.sign = Sign::create_by(acc, &thx);
        Ok(())
    }

    pub fn add_cell(&mut self, cell: Box<dyn TexCell>) -> Rerr {
        self.cells.push(cell)?;
        Ok(())
    }





}