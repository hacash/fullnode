
// close default
pub fn close_channel_default(pdhei: u64, ctx: &mut dyn Context, channel_id: &ChannelId, paychan: &ChannelSto
) -> Ret<Vec<u8>> {
    close_channel_with_distribution(
        pdhei, ctx, channel_id, paychan, 
        &paychan.left_bill.balance,
        &paychan.right_bill.balance,
        false,
    )
}


/**
 * close
 * pdhei = pending height
 */
pub fn close_channel_with_distribution(pdhei: u64, ctx: &mut dyn Context, channel_id: &ChannelId, 
    paychan: &ChannelSto, 
    left_bls: &Balance,  right_bls: &Balance,
    is_final_closed: bool,
) -> Ret<Vec<u8>> {
    // bls
    let left_amt  = &left_bls.hacash;
    let right_amt = &right_bls.hacash;
    let left_sat  = &left_bls.satoshi;
    let right_sat = &right_bls.satoshi;


    // check
    if paychan.status != CHANNEL_STATUS_OPENING {
        return errf!("channel status is not opening")
    }
    let left_addr = &paychan.left_bill.address;
    let right_addr = &paychan.right_bill.address;
	if left_amt.is_negative() || right_amt.is_negative() {
		return errf!("channel distribution amount cannot be negative.")
	}
    let ttamt = paychan.left_bill.balance.hacash.add_mode_u64(&paychan.right_bill.balance.hacash)?;
    if  left_amt.add_mode_u64(right_amt)? != ttamt {
        return errf!("HAC distribution amount must equal with lock in.")
    }
    let ttsat = paychan.left_bill.balance.satoshi + paychan.right_bill.balance.satoshi;
    if *left_sat + *right_sat != ttsat {
        return errf!("BTC distribution amount must equal with lock in.")
    }
    // let mut state = ;
    // total supply
    let mut ttcount = {
        CoreState::wrap(ctx.state()).get_total_count()
    };
    ttcount.opening_channel -= 1u64;
    // do close
    if ttamt.is_positive() {
        // calculate_interest
        let (newamt1, newamt2) = genesis::calculate_interest_of_height(
            pdhei, *paychan.open_height, 
            paychan.interest_attribution, left_amt, right_amt
        )?;
        let ttnewhac = newamt1.add_mode_u64(&newamt2) ?;
        if ttnewhac < ttamt {
            return errf!("interest calculate error!")
        }
        let ttiesthac = ttnewhac.sub_mode_u64(&ttamt) ? .to_zhu_u64().unwrap();
        ttcount.channel_interest_zhu += ttiesthac;
        ttcount.channel_deposit_zhu -= ttamt.to_zhu_u64().unwrap();
        if newamt1.is_positive() {
            hac_add(ctx, left_addr, &newamt1)?;
        }
        if newamt2.is_positive() {
            hac_add(ctx, right_addr, &newamt2)?;
        }
    }
    if *ttsat > 0 {
        ttcount.channel_deposit_sat -= *ttsat;
        if left_sat.uint() > 0 {
            sat_add(ctx, left_addr, &left_sat.to_satoshi())?;
        }
        if right_sat.uint() > 0 {
            sat_add(ctx, right_addr, &right_sat.to_satoshi())?;
        }
    }
    // save channel
    let distribution = ClosedDistributionDataOptional::must(ClosedDistributionData{
        left_bill: Balance {
            hacash: left_amt.clone(),
            satoshi: left_sat.clone(),
            diamond: DiamondNumberAuto::new(),
            assets: AssetAmtW1::new(),
        }
    });
    let mut savechan = paychan.clone();
    savechan.status = match is_final_closed {
        true => CHANNEL_STATUS_FINAL_ARBITRATION_CLOSED,
        false => CHANNEL_STATUS_AGREEMENT_CLOSED,
    };
    savechan.if_distribution = distribution;
    // save channel and count
    let mut state = MintState::wrap(ctx.state());
    state.channel_set(&channel_id, &savechan);
    state.set_total_count(&ttcount);
    // ok finish
    Ok(vec![])
}


