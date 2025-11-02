
/*
    default to spend 32 gas each call
*/
action_define!{ContractMainCall, 124, 
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
        // check codes
        let cap = SpaceCap::new(ctx.env().block.height);
        let cty = map_itr_err!(CodeType::parse(self.ctype.to_uint()))?;
        map_itr_err!(convert_and_check(&cap, cty, &self.codes))?;
        let depth = 0; // main call depth is 0
        setup_vm_run(depth, ctx, CallMode::Main as u8, *self.ctype, &self.codes, Value::Nil)?;
        Ok(vec![])
    })
}


impl ContractMainCall {
    pub fn from_bytecode(codes: Vec<u8>) -> Ret<Self> {
        let mut s = Self::new();
        s.ctype = Uint1::from(CodeType::Bytecode as u8);
        s.codes = BytesW2::from(codes)?;
        Ok(s)
    }
}


