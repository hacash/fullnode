
/*
    default to spend 32 gas each call
*/
action_define!{ContractMainCall, 121, 
    ActLv::Ast, // level
    false, [],
    {
        marks: Fixed3
        ctype: Uint1
        codes: BytesW2
    },
    (self, ctx, _gas {
        if self.marks.not_zero() {
            return errf!("marks bytes format error")
        }
        let depth = 0; // main call depth is 0
        setup_vm_run(depth, ctx, CallMode::Main as u8, *self.ctype, &self.codes, vec![])?;
        Ok(vec![])
    })
}


