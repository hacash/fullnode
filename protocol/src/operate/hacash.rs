use std::vec;

macro_rules! check_amount_is_positive {
    ($amt:expr) => {
        if ! $amt.is_positive() {
            return errf!("amount {} value is not positive", $amt)
        }
    };
}


macro_rules! amount_op_unsafe_func_define {
    ($fn:ident, $hac:ident, $addr:ident, $amt:ident, $exec:block) => (

        fn $fn(ctx: &mut dyn Context, $addr: &Address, $amt: &Amount) -> Ret<Amount> {
            let mut state = CoreState::wrap(ctx.state());
            let mut bls = state.balance( $addr ).unwrap_or_default();
            let $hac = bls.hacash;
            let newhac = $exec; // do add or sub
            if newhac.size() > 12 {
                return errf!("address {} amount {} size {} over 12 can not to store", 
                    $addr.readable(), newhac.size(), newhac)
            }
            bls.hacash = newhac.clone();
            state.balance_set($addr, &bls);
            Ok(newhac)
        }

    )

}

amount_op_unsafe_func_define!{hac_sub_unsafe, hac, addr, amt, {
    if hac < *amt {
        return errf!("address {} balance {} not enough, need {}", 
            addr.readable(), hac, amt)
    }
    hac.sub_mode_u128(amt)?
}}

amount_op_unsafe_func_define!{hac_add_unsafe, hac, addr, amt, {
    hac.add_mode_u128(amt)?
}}


pub fn hac_transfer(ctx: &mut dyn Context, from: &Address, to: &Address, amt: &Amount) -> Ret<Vec<u8>> {
    // is to self
    if from == to {
        if ctx.env().block.height >= 20_0000 {
            // you can transfer it to yourself without changing the status, which is a waste of service fees
            hac_check(ctx, from, amt)?;
        }
        return Ok(vec![]);
    }
    // do trs
    check_amount_is_positive!(amt);
    hac_sub_unsafe(ctx, from, amt)?;
    hac_add_unsafe(ctx, to, amt)?;
    Ok(vec![])
}



pub fn hac_check(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Amount> {
    check_amount_is_positive!(amt);
    let state = CoreState::wrap(ctx.state());
    if let Some(bls) = state.balance( addr ) {
        // println!("address {} balance {}", addr.readable(), bls.hacash );
        if bls.hacash >= *amt {
            return Ok(bls.hacash)
        }
    }
    errf!("address {} balance not enough need {}", addr.readable(), amt )
}


pub fn hac_add(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Vec<u8>> {
    check_amount_is_positive!(amt);
    hac_add_unsafe(ctx, addr, amt)?;
    Ok(vec![])
}


pub fn hac_sub(ctx: &mut dyn Context, addr: &Address, amt: &Amount) -> Ret<Vec<u8>> {
    check_amount_is_positive!(amt);
    hac_sub_unsafe(ctx, addr, amt)?;
    Ok(vec![])
}

