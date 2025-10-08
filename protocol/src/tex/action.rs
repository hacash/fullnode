
/*
* simple hac to
*/
action_define!{ TexCellAct, 35, 
    ActLv::Top, // level
    false, // burn 90 fee
    [], // need sign
    {
        addr  : Address
        cells : DnyTexCellListW1
        sign  : Sign
    },
    (self, ctx, _gas {
        self.addr.must_privakey()?;
        // check signature
        let stf = vec![self.addr.serialize(), self.cells.serialize()].concat();
        let thx = Hash::from(sha3(&stf));
        if ! verify_signature(&thx, &self.addr, &self.sign) {
            return errf!("address {} signature verify failed", self.addr.readable())
        }
        // exec
        self.cells.execute(ctx, &self.addr).map(|_|vec![])
    })
}

