

pub const ASSET_ALIVE_HEIGHT: u64 = 600000;


fn alive_blk_hei(ctx: &mut dyn Context) -> Ret<u64> {
    let chei = ctx.env().block.height;
    let is_mainnet = ctx.env().chain.id==0 && chei > ASSET_ALIVE_HEIGHT;
    if is_mainnet {
        #[cfg(not(feature = "hip20"))]
        return err!("HIP20 asset just for test chain now")
    }
    let alive_hei: u64 = maybe!(is_mainnet, ASSET_ALIVE_HEIGHT, 0);
    Ok(alive_hei)
}


action_define!{AssetCreate, 16, 
    ActLv::TopUnique, // level
    false, // burn 90 fee
    [], {
        metadata: AssetSmelt
        protocol_fee: Amount
    },
    (self, ctx, _gas {
        let amd = self.metadata.clone();
        // check serial
        let chei = ctx.env().block.height;
        let alive_hei: u64 = alive_blk_hei(ctx)?;
        if chei <= alive_hei {
            return err!("The asset issuance has not yet begun")
        }
        let serial_limit = chei - alive_hei;
        if *amd.serial > serial_limit {
            return err!("The asset serial overflow")
        }
        // check meta
        amd.issuer.check_version()?;
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
        // support debug test
        let minsr = maybe!(chei>ASSET_ALIVE_HEIGHT, 1024, 0);
        if *amd.serial <= minsr {
            return errf!("serial cannot less than {}", minsr)
        }
        // check fee and burn
        let blkrw = super::genesis::block_reward(chei);
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
        ttcount.created_asset += 1;
        ttcount.asset_issue_burn_mei += pfee.to_mei_u64().unwrap();
        sta.set_total_count(&ttcount);
        // do mint
        let mut asset_obj = AssetAmt::new(amd.serial);
        asset_obj.amount = amd.supply; // total supply
        // issue
        let mut bls = sta.balance( &amd.issuer ).unwrap_or_default();
        bls.asset_set(asset_obj)?;
        sta.balance_set( &amd.issuer, &bls );
        // ok finish
        Ok(vec![])
    })
}



