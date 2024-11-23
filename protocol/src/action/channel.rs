
/*
*
*/
action_define!{ ChannelOpen, 2, 
    ActLv::TOP, // level
    false, // burn 90 fee
    [], // need sign
    {
        channel_id     : ChannelId
        left_bill      : AddrHac
        right_bill     : AddrHac
    },
    (self, ctx, _gas {
        channel_open(self, ctx)
    })
}


fn channel_open(this: &ChannelOpen, ctx: &mut dyn Context) -> Ret<Vec<u8>> {

    this.left_bill.address.check_must_privakey()?;
    this.right_bill.address.check_must_privakey()?;

    let (cid, left_addr, left_amt, right_addr, right_amt ) = (
        &this.channel_id,
        &this.left_bill.address,
        &this.left_bill.amount,
        &this.right_bill.address,
        &this.right_bill.amount
    );

    let env = ctx.env().clone();

    // sub balance
    if left_amt.not_zero() {
        hac_sub(ctx, left_addr,  left_amt)?;
    }
    if right_amt.not_zero() {
        hac_sub(ctx, right_addr, right_amt)?;
    }
    // check id size
    check_vaild_store_item_key("channel", &cid, ChannelId::SIZE)?;
    // check format
    if left_addr == right_addr {
        return errf!("left address cannot equal with right address")
    }
    if left_amt.size() > 6 || right_amt.size() > 6 {
        return errf!("left or right amount bytes too long")
    }
    if left_amt.is_negative() || right_amt.is_negative() ||
        (left_amt.is_zero() && right_amt.is_zero()) {
        return errf!("left or right amount is not positive or two both is empty")
    }

    // check exist
    let mut reuse_version = Uint4::from(1);
	// channel ID with the same left and right addresses and closed by consensus can be reused
    let mut state = CoreState::wrap(ctx.state());
    let havcha = state.channel(cid);
    if let Some(chan) = havcha {
        let chan_stat = chan.status;
        let samebothaddr = *left_addr==chan.left_bill.address && *right_addr == chan.right_bill.address;
        if !samebothaddr || CHANNEL_STATUS_AGREEMENT_CLOSED != chan_stat {
            // exist or cannot reuse
            return errf!("channel {} is openning or cannot reuse.", cid)
        }
        reuse_version = chan.reuse_version.clone();
        reuse_version += 1u64;
    }

    // save channel
    let pd_hei = env.block.height;
    let channel = ChannelSto{
        status: CHANNEL_STATUS_OPENING,
        reuse_version: reuse_version,
        open_height: Uint5::from(pd_hei),
        arbitration_lock_block: Uint2::from(5000), // lock period is about 17 days
        interest_attribution: CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT,
        left_bill: AddrHacSat{
            address: left_addr.clone(),
            hacsat: HacSat{amount: left_amt.clone(), satoshi: SatoshiOptional::default()}},
        right_bill: AddrHacSat{
            address: right_addr.clone(),
            hacsat: HacSat{amount: right_amt.clone(), satoshi: SatoshiOptional::default()}},
        if_challenging: ChallengePeriodDataOptional::default(), // none
        if_distribution: ClosedDistributionDataOptional::default(), // none
    };
    state.channel_set(cid, &channel);

    // update total count
    let mut ttcount = state.get_total_count();
    ttcount.opening_channel += 1u64;
    ttcount.channel_deposit_zhu += left_amt.add_mode_u64(right_amt)?.to_zhu_unsafe() as u64;
    state.set_total_count(&ttcount);

    // ok finish
    Ok(vec![])
}



/*******************************************/



action_define!{ ChannelClose, 3, 
    ActLv::TOP, // level
    false, // burn 90 fee
    [], // need sign
    {
        channel_id     : ChannelId 
    },
    (self, ctx, _gas {
        channel_close(self, ctx)
    })
}


fn channel_close(this: &ChannelClose, ctx: &mut dyn Context) -> Ret<Vec<u8>> {
    
    let cid = &this.channel_id;
    check_vaild_store_item_key("channel", cid, ChannelId::SIZE)?;

    let pending_height = ctx.env().block.height;
    let state = CoreState::wrap(ctx.state());

    // query
    let chan = must_have!("channel", state.channel(cid));
    chan.left_bill.address.check_must_privakey()?;
    chan.right_bill.address.check_must_privakey()?;

	// verify two address sign
    ctx.check_sign( &chan.left_bill.address )?;
    ctx.check_sign( &chan.right_bill.address )?;
    
    // do close
    close_channel_default( pending_height, ctx, cid, &chan)
}

