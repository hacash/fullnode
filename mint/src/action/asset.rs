

pub const ASSET_ALIVE_HEIGHT: u64 = 700000;


#[allow(unused)]
fn check_alive_blk_hei(ctx: &mut dyn Context) -> Ret<(u64, u64)> {
    #[cfg(not(feature = "hip20"))]
    return err!("HIP20 asset just for test chain now");
    // open hip20 feature
    let chei = ctx.env().block.height;
    let is_mainnet = ctx.env().chain.id==0 && chei > ASSET_ALIVE_HEIGHT;
    let alive_hei: u64 = maybe!(is_mainnet, ASSET_ALIVE_HEIGHT, 0);
    let minsri:    u64 = maybe!(is_mainnet, 1025, 5);
    Ok((alive_hei, minsri))
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
        let serial = *amd.serial;
        // check serial
        let chei = ctx.env().block.height;
        let (alive_hei, minsri) = check_alive_blk_hei(ctx)?;
        if alive_hei > chei {
            return err!("The asset issuance has not yet begun")
        }
        if serial < minsri {
            return errf!("serial cannot less than {}", minsri)
        }
        let serial_limit = chei - alive_hei;
        if serial > serial_limit {
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
        if let Some(..) = sta.asset(&amd.serial) {
            return errf!("Asset serial {} already exists", serial)
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



