
/*
* simple hac to
*/
action_define!{ HacToTrs, 1, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [], // need sign
    {
        to : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to   = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}

impl HacToTrs {
    pub fn create_by(to: Address, hacash: Amount) -> Self {
        Self{
            to: AddrOrPtr::from_addr(to), 
            hacash,
            ..Self::new()
        }
    }
}


action_define!{ HacFromTrs, 13, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [self.from],
    {
        from   : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.env().tx.main; 
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}

impl HacFromTrs {
    pub fn create_by(from: Address, hacash: Amount) -> Self {
        Self{
            from: AddrOrPtr::from_addr(from), 
            hacash,
            ..Self::new()
        }
    }
}




action_define!{ HacFromToTrs, 14, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [self.from],
    {
        from   : AddrOrPtr
        to     : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}




