

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





