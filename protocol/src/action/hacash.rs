
/*
* simple hac to
*/
action_define!{ HacToTransfer, 1, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [], // need sign
    {
        to : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}


action_define!{ HacFromTransfer, 13, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from],
    {
        from   : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main; 
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}




action_define!{ HacFromToTransfer, 14, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from],
    {
        from   : AddrOrPtr
        to     : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}




