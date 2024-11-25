


macro_rules! diamond_operate_define {
    ($func_name: ident, $addr:ident, $hacd:ident, $oldhacd:ident, $newhacdblock:block) => (

pub fn $func_name(state: &mut CoreState, $addr: &Address, $hacd: &DiamondNumber) -> Ret<DiamondNumber> {
    let mut userbls = state.balance( $addr ).unwrap_or_default();
    let $oldhacd = &userbls.diamond.to_diamond();
    /* -------- */
    let newhacd = $newhacdblock;// operate
    /* -------- */
    // save
    userbls.diamond = DiamondNumberAuto::from_diamond( &newhacd );
    state.balance_set($addr, &userbls);
    Ok(newhacd)
}

    )
}


/**************************** */

diamond_operate_define!(hacd_add, addr, hacd, oldhacd, {
    // do add
    *oldhacd + *hacd
});

diamond_operate_define!(hacd_sub, addr, hacd, oldhacd, {  
    // check
    if *oldhacd < *hacd {
		return errf!("address {} diamond {} is insufficient, at least {}", 
            addr.readable(), oldhacd, hacd)
    }
    // do sub
    *oldhacd - *hacd
});



/**************************** */


pub fn hacd_transfer(state: &mut CoreState,
    addr_from: &Address, addr_to: &Address, hacd: &DiamondNumber, _dlist: &DiamondNameListMax200
) -> Ret<Vec<u8>> {
    if addr_from == addr_to {
		return errf!("cannot transfer to self")
    }
    // check contract
    /*use vm::rt::SystemCallType::*;
    let amtv = dlist.form(); // name bytes vec
    ctx.syscall_check_true(addr_from, PermitHACD  as u8, amtv.clone())?;
    ctx.syscall_check_true(addr_to,   PayableHACD as u8, amtv)?;*/
    // do transfer
    hacd_sub(state, addr_from, hacd)?;
    hacd_add(state, addr_to, hacd)?;
    // ok
    Ok(vec![])
}


/*********************************** */


pub fn hacd_move_one_diamond(state: &mut CoreState, addr_from: &Address, addr_to: &Address, hacd_name: &DiamondName) -> Rerr {
    if addr_from == addr_to {
		return errf!("cannot transfer to self")
    }
    // query
    let mut diaitem = check_diamond_status(state, addr_from, hacd_name)?;
	// transfer diamond
    diaitem.address = addr_to.clone();
    state.diamond_set(hacd_name, &diaitem);
    // ok
    Ok(())
}


pub fn check_diamond_status(state: &mut CoreState, addr_from: &Address, hacd_name: &DiamondName) -> Ret<DiamondSto> {
    // query
    let diaitem = must_have!(
        format!("diamond {}", hacd_name.to_readable()),
        state.diamond(hacd_name));
    if diaitem.status != DIAMOND_STATUS_NORMAL {
        return errf!("diamond {} has been mortgaged and cannot be transferred", hacd_name.to_readable())
    }
    if *addr_from != diaitem.address {
        return errf!("diamond {} not belong to address {}", hacd_name.to_readable(), addr_from.readable())
    }
    // ok
    Ok(diaitem)
}



/**
* 
* return total cost
*/
pub fn engraved_one_diamond(pending_height: u64, state: &mut CoreState, addr :&Address, diamond: &DiamondName, content: &BytesW1) -> Ret<Amount> {

    let mut diasto = check_diamond_status(state, addr, diamond)?;
    
    // check height
    let prev_insc_hei = diasto.prev_engraved_height.uint();
    let check_prev_block = 1000u64;
    if prev_insc_hei + check_prev_block > pending_height {
        return errf!("only one inscription can be made every {} blocks", check_prev_block)
    }

    // check insc
    let haveng = diasto.inscripts.count().uint();
    if haveng >= 200 {
        return errf!("maximum inscriptions for one diamond is 200")
    }

    let diaslt = must_have!(format!("diamond {}", diamond.to_readable()), state.diamond_smelt(&diamond));

    // cost
    let mut cost = Amount::default(); // zero
	if haveng >= 10 {
		// burning cost bid fee 1/10 from 11 insc
		cost = Amount::coin(diaslt.average_bid_burn.uint() as u64, 247);
	}

	// do engraved
    diasto.prev_engraved_height = BlockHeight::from(pending_height);
    diasto.inscripts.push(content.clone())?;
	// save
	state.diamond_set(diamond, &diasto);

	// ok finish
	Ok(cost)
}

/* 
* return total cost
*/
pub fn engraved_clean_one_diamond(_pending_height: u64, state: &mut CoreState, addr :&Address, diamond: &DiamondName) -> Ret<Amount> {

    let mut diasto = check_diamond_status(state, addr, diamond)?;
    let diaslt = must_have!(format!("diamond {}", diamond.to_readable()), state.diamond_smelt(&diamond));
    // check
    if diasto.inscripts.count().uint() <= 0 {
        return errf!("cannot find any inscriptions in HACD {}", diamond.to_readable())    }

    // burning cost bid fee
    let cost = Amount::mei(diaslt.average_bid_burn.uint() as u64);
	// do clean
    diasto.prev_engraved_height = BlockHeight::from(0);
    diasto.inscripts = Inscripts::default();
	// save
	state.diamond_set(diamond, &diasto);

	// ok finish
	Ok(cost)
}


/**
* diamond owned push or drop
*/
pub fn diamond_owned_push_one(state: &mut CoreState, address: &Address, name: &DiamondName) {
    let mut owned = state.diamond_owned(address).unwrap_or_default();
    owned.push_one(name);
    state.diamond_owned_set(address, &owned);
}

pub fn diamond_owned_move(state: &mut CoreState, from: &Address, to: &Address, list: &DiamondNameListMax200) -> Rerr {
    // do drop
    let from_owned = state.diamond_owned(from);
    if let None = from_owned {
        return errf!("from diamond owned form not find")
    }
    let mut from_owned = from_owned.unwrap();
    let blsnum = from_owned.drop(list)?;
    if blsnum > 0 {
        state.diamond_owned_set(from, &from_owned);
    }else{
        state.diamond_owned_del(from);
    }
    // do push
    let mut to_owned = state.diamond_owned(to).unwrap_or_default();
    to_owned.push(list);
    state.diamond_owned_set(to, &to_owned);
    Ok(())
}

