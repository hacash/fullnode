



action_define!{AssetToTrs, 17, 
    ActLv::MAIN_CALL,
    true,
    [], {
        to: AddrOrPtr
        amount: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to   = ctx.addr(&self.to)?;
        asset_transfer(ctx, &from, &to, &self.amount)
    })
}


action_define!{AssetFromTrs, 18, 
    ActLv::MAIN_CALL,
    true,
    [
        self.from
    ], {
        from: AddrOrPtr
        amount: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.env().tx.main; 
        asset_transfer(ctx, &from, &to, &self.amount)
    })
}


action_define!{AssetFromToTrs, 19, 
    ActLv::MAIN_CALL,
    true,
    [
        self.from
    ], {
        from: AddrOrPtr
        to: AddrOrPtr
        amount: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.addr(&self.to)?;
        asset_transfer(ctx, &from, &to, &self.amount)
    })
}





