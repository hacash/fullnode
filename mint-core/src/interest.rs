//! Channel interest calculation (moved from mint's genesis; pure field math, no consensus-state dependency).

use field::{Amount, Uint1};
use num_bigint::BigUint;

pub fn calculate_interest(
    user_distribute_amt: &Amount,
    interest_calc_base_amt: &Amount,
    calc_loop: u64,
    wfzn: u64,
) -> sys::Ret<Amount> {
    let newunit = interest_calc_base_amt.unit() as i32 - 8;
    if newunit < 0 {
        return Ok(user_distribute_amt.clone());
    }
    let zero = BigUint::from(0u64);
    let mut coinnum = BigUint::from_bytes_be(interest_calc_base_amt.byte());
    coinnum *= 1_0000_0000u64;
    for _ in 0..calc_loop {
        coinnum *= 10_000u64 + wfzn;
        coinnum /= 10_000u64;
    }
    let mut unit = newunit as u8;
    loop {
        if unit >= 255 || coinnum.clone() % 10u64 != zero {
            break;
        }
        coinnum /= 10u64;
        unit += 1;
    }
    let realbest = Amount::from_unit_byte(unit, coinnum.to_bytes_be())?
        .sub_mode_u64(interest_calc_base_amt)?;
    realbest.add_mode_u64(user_distribute_amt)
}

pub fn both_interest(
    distribute_type: Uint1,
    amtl: &Amount,
    amtr: &Amount,
    calc_loop: u64,
    wfzn: u64,
) -> sys::Ret<(Amount, Amount)> {
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT == distribute_type {
        let amt1 = calculate_interest(amtl, amtl, calc_loop, wfzn)?;
        let amt2 = calculate_interest(amtr, amtr, calc_loop, wfzn)?;
        return Ok((amt1, amt2));
    }

    let total = amtl.add_mode_u64(amtr)?;
    let mut res = (amtl.clone(), amtr.clone());
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_LEFT == distribute_type {
        res.0 = calculate_interest(amtl, &total, calc_loop, wfzn)?;
    }
    if field::CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_RIGHT == distribute_type {
        res.1 = calculate_interest(amtr, &total, calc_loop, wfzn)?;
    }
    Ok(res)
}

pub fn calculate_interest_of_height(
    curblkhei: u64,
    chanopenblkhei: u64,
    distribute_type: Uint1,
    amtl: &Amount,
    amtr: &Amount,
) -> sys::Ret<(Amount, Amount)> {
    if curblkhei < chanopenblkhei {
        return sys::errf!("current block height cannot be less than channel open height");
    }
    let calc_loop = (curblkhei - chanopenblkhei) / 10_000;
    let wfzn = 10;
    if calc_loop == 0 {
        return Ok((amtl.clone(), amtr.clone()));
    }
    both_interest(distribute_type, amtl, amtr, calc_loop, wfzn)
}
