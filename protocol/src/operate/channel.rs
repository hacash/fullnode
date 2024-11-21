
// close default
pub fn close_channel_default(pdhei: u64, ctx: &mut dyn Context, channel_id: &ChannelId, paychan: &ChannelSto
) -> Ret<Vec<u8>> {
    close_channel_with_distribution(
        pdhei, ctx, channel_id, paychan, 
        &paychan.left_bill.hacsat.amount,
        &paychan.right_bill.hacsat.amount,
        &paychan.left_bill.hacsat.satoshi.value(),
        &paychan.right_bill.hacsat.satoshi.value(),
        false,
    )
}


/**
 * close
 * pdhei = pending height
 */
pub fn close_channel_with_distribution(pdhei: u64, ctx: &mut dyn Context, channel_id: &ChannelId, 
    paychan: &ChannelSto, 
    left_amt: &Amount,  right_amt: &Amount,
    left_sat: &Satoshi, right_sat: &Satoshi,
    is_final_closed: bool,
) -> Ret<Vec<u8>> {

    // check
    if paychan.status != CHANNEL_STATUS_OPENING {
        return errf!("channel status is not opening")
    }
    let left_addr = &paychan.left_bill.address;
    let right_addr = &paychan.right_bill.address;
	if left_amt.is_negative() || right_amt.is_negative() {
		return errf!("channel distribution amount cannot be negative.")
	}
    let ttamt = paychan.left_bill.hacsat.amount.add_mode_u64(&paychan.right_bill.hacsat.amount)?;
    if  left_amt.add_mode_u64(right_amt)? != ttamt {
        return errf!("HAC distribution amount must equal with lock in.")
    }
    let ttsat = paychan.left_bill.hacsat.satoshi.value() + paychan.right_bill.hacsat.satoshi.value();
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
        let (newamt1, newamt2) = calculate_interest_of_height(
            pdhei, *paychan.open_height, 
            paychan.interest_attribution, left_amt, right_amt
        )?;
        let ttnewhac = newamt1.add_mode_u64(&newamt2) ?;
        if ttnewhac < ttamt {
            return errf!("interest calculate error!")
        }
        let ttiesthac =  ttnewhac.sub_mode_u64(&ttamt) ? .to_zhu_unsafe() as u64;
        ttcount.channel_interest_zhu += ttiesthac;
        ttcount.channel_deposit_zhu -= ttamt.to_zhu_unsafe() as u64;
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
            sat_add(ctx, left_addr, left_sat)?;
        }
        if right_sat.uint() > 0 {
            sat_add(ctx, right_addr, right_sat)?;
        }
    }
    // save channel
    let distribution = ClosedDistributionDataOptional::must(ClosedDistributionData{
        left_bill: HacSat{
            amount: left_amt.clone(),
            satoshi: SatoshiOptional::must(left_sat.clone()),
        }
    });
    let mut savechan = paychan.clone();
    savechan.status = match is_final_closed {
        true => CHANNEL_STATUS_FINAL_ARBITRATION_CLOSED,
        false => CHANNEL_STATUS_AGREEMENT_CLOSED,
    };
    savechan.if_distribution = distribution;
    // save channel and count
    let mut state = CoreState::wrap(ctx.state());
    state.channel_set(&channel_id, &savechan);
    state.set_total_count(&ttcount);
    // ok finish
    Ok(vec![])
}





/****************  calculate ****************/




pub fn calculate_interest(user_distribute_amt: &Amount, interest_calc_base_amt: &Amount, caclloop: u64, wfzn: u64) -> Ret<Amount> {
    // check
    let uamt = user_distribute_amt;
    let bamt = interest_calc_base_amt;
    let mut newunit = bamt.unit() as i32 - 8; // base 1_0000_0000u64
    if newunit < 0 {
        // very small amount, ignored, balance unchanged
        return Ok(uamt.clone())
    }
    // calculate
    let zore = BigUint::from(0u64);
    let coinb = BigUint::from_bytes_be(bamt.byte());
    let mut coinnum = coinb.clone();
    coinnum *= 1_0000_0000u64;
    for _ in 0..caclloop {
        coinnum *= 10000u64 + wfzn;
        coinnum /= 10000u64;
    }
    // convert
    loop {
        if newunit >= 255 || coinnum.clone() % 10u64 != zore {
            break
        }
        coinnum /= 10u64;
        newunit += 1;
    }
    let realbest = Amount::from_unit_byte( newunit as u8, coinnum.to_bytes_be() )?;
    let realbest = realbest.sub_mode_u64(bamt)?;
    // println!("realest: {}", realbest.to_string());
    let newuamt = realbest.add_mode_u64( uamt )?;
    // ok
    return Ok(newuamt)
} 


pub fn both_interest(distribute_type: Uint1, amtl: &Amount, amtr: &Amount, caclloop: u64, wfzn: u64)-> Ret<(Amount, Amount)> {
    
    if CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT == distribute_type {
        let amt1 = calculate_interest(amtl, amtl, caclloop, wfzn)?;
        let amt2 = calculate_interest(amtr, amtr, caclloop, wfzn)?;
        return Ok((amt1, amt2))
    }

    let ttamt = amtl.add_mode_u64(amtr)?;
    let mut resamts = (amtl.clone(), amtr.clone());
    
    if CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_LEFT == distribute_type{
        resamts.0 = calculate_interest(amtl, &ttamt, caclloop, wfzn)?;
    }
    if CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_RIGHT == distribute_type{
        resamts.1 = calculate_interest(amtr, &ttamt, caclloop, wfzn)?;
    }
    
    Ok(resamts)
}

pub fn calculate_interest_of_height(curblkhei: u64, chanopenblkhei: u64, distribute_type: Uint1, amtl: &Amount, amtr: &Amount)-> Ret<(Amount, Amount)> {
    if curblkhei < chanopenblkhei {
        return Err("current block height cannot less than channel open height".to_string())
    }
	let mut caclloop = ((curblkhei - chanopenblkhei) / 2500 ) as u64;
	let mut wfzn: u64 = 1; // 1/10000
    // 
    if chanopenblkhei > 20_0000 {
        // increase interest calculation, compounding times: 
        // about 10000 blocks will be compounded once every 34 days, 
        // less than 34 days will be ignored, and the annual compound interest is about 1.06%
		caclloop = ((curblkhei - chanopenblkhei) / 10000) as u64;
		wfzn = 10 // 10/10000
    }
    if caclloop == 0 {
        return Ok((amtl.clone(), amtr.clone()))
    }
    // calculate_interest
    both_interest(distribute_type, amtl, amtr, caclloop, wfzn)
}





