
/*
*
*/
action_define!{ SatToTrs, 10, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [], // need sign
    {
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to   = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}



action_define!{ SatFromTrs, 11, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        satoshi   : Satoshi   
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.env().tx.main; 
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}



action_define!{ SatFromToTrs, 12, 
    ActLv::MainCall, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}
