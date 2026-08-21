//! Channel interest calculation (moved from mint's genesis; pure field math, no consensus-state dependency).

use field::{Amount, Uint1};
use field::{divmod_u64_b256, mul_u64_b256};

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
    // Mantissa arithmetic on field's base-256 byte-array core (the single
    // production big-number implementation; BigUint is the test drift oracle).
    let mut coinnum = mul_u64_b256(interest_calc_base_amt.byte(), 1_0000_0000u64);
    for _ in 0..calc_loop {
        coinnum = mul_u64_b256(&coinnum, 10_000u64 + wfzn);
        let (quotient, _) = divmod_u64_b256(&coinnum, 10_000u64);
        coinnum = quotient;
    }
    let mut unit = newunit as u8;
    loop {
        let (quotient, remainder) = divmod_u64_b256(&coinnum, 10u64);
        if unit >= 255 || remainder != 0 {
            break;
        }
        coinnum = quotient;
        unit += 1;
    }
    let realbest = Amount::from_unit_byte(unit, coinnum)?.sub_mode_u64(interest_calc_base_amt)?;
    realbest.add_mode_u64(user_distribute_amt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;

    /// The production implementation that shipped before the base-256 core,
    /// kept as the drift oracle.
    fn reference_interest(
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

    /// Simple LCG, avoiding a rand dependency for tests.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn pick(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Random amount via the public parser: 1..=40-digit mantissa (crosses the u128
    /// boundary), random unit 0..=255, sometimes negative — `sub_mode_u64` then errors identically in both impls.
    fn random_amount(lcg: &mut Lcg) -> Amount {
        let mut s = String::with_capacity(48);
        if lcg.pick(4) == 0 {
            s.push('-');
        }
        s.push((b'0' + 1 + lcg.pick(9) as u8) as char); // no leading zero
        for _ in 0..lcg.pick(40) {
            s.push((b'0' + lcg.pick(10) as u8) as char);
        }
        let unit = lcg.pick(256) as u8;
        Amount::from(&format!("{s}:{unit}")).unwrap()
    }

    #[test]
    fn interest_matches_biguint_reference() {
        let mut lcg = Lcg(0x1e57);
        for _ in 0..3000 {
            let user = random_amount(&mut lcg);
            let base = random_amount(&mut lcg);
            let calc_loop = lcg.pick(500) as u64;
            let wfzn = lcg.pick(100) as u64;
            let got = calculate_interest(&user, &base, calc_loop, wfzn);
            let expected = reference_interest(&user, &base, calc_loop, wfzn);
            match (got, expected) {
                (Ok(g), Ok(e)) => {
                    assert_eq!(g, e, "user={user} base={base} loop={calc_loop} wfzn={wfzn}")
                }
                (Err(_), Err(_)) => {}
                (g, e) => panic!(
                    "interest diverged: got {g:?} expected {e:?} user={user} base={base} loop={calc_loop} wfzn={wfzn}"
                ),
            }
        }
        // Long-channel stress: many compounding loops on a small base (mantissa grows
        // past the u128 boundary; both implementations must agree on the error boundary too)
        let small = Amount::coin(1, 248);
        for (loops, wfzn) in [
            (10_000u64, 10u64),
            (50_000, 10),
            (100_000, 10),
            (100_000, 0),
        ] {
            let got = calculate_interest(&small, &small, loops, wfzn);
            let expected = reference_interest(&small, &small, loops, wfzn);
            match (got, expected) {
                (Ok(g), Ok(e)) => assert_eq!(g, e, "long loop={loops} wfzn={wfzn}"),
                (Err(_), Err(_)) => {}
                (g, e) => panic!(
                    "long interest diverged: got {g:?} expected {e:?} loop={loops} wfzn={wfzn}"
                ),
            }
        }
    }
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
