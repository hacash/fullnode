



action_define!{AssetToTrs, 17, 
    ActLv::MainCall,
    true, // burn 90 fee
    [], {
        to: AddrOrPtr
        asset: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main; 
        let to   = ctx.addr(&self.to)?;
        asset_transfer(ctx, &from, &to, &self.asset)
    })
}


action_define!{AssetFromTrs, 18, 
    ActLv::MainCall,
    true,  // burn 90 fee
    [ self.from ], {
        from: AddrOrPtr
        asset: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.env().tx.main; 
        asset_transfer(ctx, &from, &to, &self.asset)
    })
}


action_define!{AssetFromToTrs, 19, 
    ActLv::MainCall,
    true,  // burn 90 fee
    [ self.from ], {
        from: AddrOrPtr
        to: AddrOrPtr
        asset: AssetAmt
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.addr(&self.to)?;
        asset_transfer(ctx, &from, &to, &self.asset)
    })
}





