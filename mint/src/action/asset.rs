


action_define!{AssetCreate, 16, 
    ActLv::TopOnly, // level
    false, // burn 90 fee
    [], {
        metadata: AssetSmelt
        protocol_fee: Amount
    },
    (self, ctx, _gas {
        let amd = self.metadata.clone();
        // check serial
        let chei = ctx.env().block.height;
        let is_mainnet = ctx.env().chain.id==0 && chei > 600000;
        if is_mainnet {
            return err!("asset just for test chain now")
        }
        let alive_blk_hei: u64 = maybe!(is_mainnet, 600000, 0);
        if chei <= alive_blk_hei {
            return err!("The asset issuance has not yet begun")
        }
        let serial_limit = chei - alive_blk_hei;
        if *amd.serial > serial_limit {
            return err!("The asset serial overflow")
        }
        // check meta
        amd.issuer.check_version()?; // address version
        let tl = amd.ticket.length();
        let nl = amd.name.length();
        if tl < 1 || tl > 8 {
            return err!("ticket length must be 1 ~ 8")
        }
        if nl < 1 || nl > 32 {
            return err!("name length must be 1 ~ 32")
        }
        if *amd.decimal > 16 {
            return err!("decimal cannot more than 16")
        }
        if *amd.serial <= 1024 {
            return err!("serial cannot less than 1024")
        }
        // check fee and burn
        let blkrw = block_reward(chei);
        let pfee = self.protocol_fee.clone();
        if pfee != blkrw {
            return errf!("Protocol fee need {} but got {}", blkrw, pfee)
        }
	    // sub main addr balance for protocol fee
        let main_addr = ctx.env().tx.main; 
        hac_sub(ctx, &main_addr, &pfee)?;
        // state and check exists
        let mut sta = MintState::wrap(ctx.state());
        if let Some(_) = sta.asset(&amd.serial) {
            return errf!("Asset serial {} already exists", amd.serial)
        }
        sta.asset_set(&amd.serial, &amd); // store asset object
        // total count update
        let mut ttcount = sta.get_total_count();
        ttcount.asset_issue_burn_mei += pfee.to_mei_u64().unwrap();
        sta.set_total_count(&ttcount);
        // do mint
        let mut asset_obj = AssetAmt::new(amd.serial);
        asset_obj.amount = amd.supply; // total supply
        // save
        let mut bls = sta.balance( &amd.issuer ).unwrap_or_default();
        bls.asset_set(asset_obj)?;
        sta.balance_set( &amd.issuer, &bls );
        // ok finish
        Ok(vec![])
    })
}



