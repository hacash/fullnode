

macro_rules! satoshi_operate_define {
    ($func_name: ident, $addr:ident, $sat:ident, $oldsat:ident,  $newsatblock:block) => (

        pub fn $func_name(ctx: &mut dyn Context, $addr: &Address, $sat: &Satoshi) -> Ret<Satoshi> {
            $addr.check_version()?;
            if $sat.uint() == 0 {
                return errf!("satoshi value cannot zore")
            }    
            let mut state = CoreState::wrap(ctx.state());
            let mut userbls = state.balance( $addr ).unwrap_or_default();
            let $oldsat = &userbls.satoshi.to_satoshi();
            /* -------- */
            let newsat = $newsatblock;// operate
            /* -------- */
            // save
            userbls.satoshi = SatoshiAuto::from_satoshi( &newsat );
            state.balance_set($addr, &userbls);
            Ok(newsat)
        }

    )
}


/**************************** */

satoshi_operate_define!(sat_add, addr, sat, oldsat, {
    // do add
    *oldsat + *sat 
});

satoshi_operate_define!(sat_sub, addr, sat, oldsat, {  
    // check
    if *oldsat < *sat {
		return errf!("address {} satoshi {} is insufficient, at least {}", 
            addr.readable(), oldsat, sat)
    }
    // do sub
    *oldsat - *sat
});



/**************************** */


pub fn sat_transfer(ctx: &mut dyn Context, from: &Address, to: &Address, sat: &Satoshi
) -> Ret<Vec<u8>> {
    if from == to {
		return errf!("cannot trs to self")
    }
    /*p2sh check*/
    #[cfg(not(feature = "p2sh"))]
    if from.is_scriptmh() {
        return errf!("scriptmh address cannot be from yet")
    }
    // do transfer
    sat_sub(ctx, from, sat)?;
    sat_add(ctx, to,   sat)?;
    let state = &mut CoreState::wrap(ctx.state());
    blackhole_engulf(state, to);
    // ok
    Ok(vec![])
}


pub fn sat_check(ctx: &mut dyn Context, addr: &Address, sat: &Satoshi) -> Ret<Satoshi> {
    addr.check_version()?;
    if sat.uint() == 0 {
        return errf!("check satoshi is cannot empty")
    }
    let state = CoreState::wrap(ctx.state());
    if let Some(bls) = state.balance( addr ) {
        let usrsat = bls.satoshi.to_satoshi();
        if usrsat >= *sat {
            return Ok(usrsat)
        }
    }
    errf!("address {} satoshi is insufficient", addr.readable())
}






