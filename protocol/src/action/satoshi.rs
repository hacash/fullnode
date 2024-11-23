
/*
*
*/
action_define!{ SatoshiToTransfer, 9, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [], // need sign
    {
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}



action_define!{ SatoshiFromTransfer, 10, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        satoshi   : Satoshi   
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main; 
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}



action_define!{ SatoshiFromToTransfer, 11, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)
    })
}
