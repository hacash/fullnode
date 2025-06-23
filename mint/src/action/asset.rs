


action_define!{AssetCreate, 16, 
    ActLv::TOP_ONLY, // level
    true, // burn 90 fee
    [], {
        prev_hash: Hash
        mint_addr: Address
        serial: Fold64
        supply: Fold64
        decimal: Uint1
        ticket: BytesW1
        name:   BytesW1
    },
    (self, ctx, _gas {
        let is_mainnet = ctx.env().chain.id==0 && ctx.env().block.height > 600000;
        if is_mainnet {
            return err!("asset just for test chain now")
        }
        if self.ticket.length() > 8 {
            return err!("ticket length cannot more than 8")
        }
        if self.name.length() > 32 {
            return err!("name length cannot more than 32")
        }
        if *self.decimal > 16 {
            return err!("decimal cannot more than 16")
        }
        if *self.serial <= 1024 {
            return err!("serial cannot less than 1024")
        }
        // check serial & prev_hash

        // TODO::
        
        // do mint
        let mut asset_obj = AssetAmt::new(self.serial);
        asset_obj.amount = self.supply; // total supply
        // save
        let mut sta = CoreState::wrap(ctx.state());
        let mut bls = sta.balance( &self.mint_addr ).unwrap_or_default();
        bls.asset_set(asset_obj)?;
        sta.balance_set( &self.mint_addr, &bls );
        Ok(vec![])
    })
}



