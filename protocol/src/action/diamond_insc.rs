
/*
*
*/
action_define!{ DiamondInscription, 32, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        diamonds         : DiamondNameListMax200
        protocol_cost    : Amount
        engraved_type    : Uint1
        engraved_content : BytesW1  
    },
    (self, ctx, _gas {
        diamond_inscription(self, ctx)
    })
}


fn diamond_inscription(this: &DiamondInscription, ctx: &mut dyn Context) -> Ret<Vec<u8>> {

    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pcost = &this.protocol_cost;

    // check
    this.diamonds.check()?;
	if pcost.size() > 4 {
		return errf!("protocol cost amount size cannot over 4 bytes")
	}
	// check insc size and visible
    let insc_len = this.engraved_content.length();
    if insc_len == 0 {
		return errf!("engraved content cannot be empty")
    }
    if insc_len > 64 {
		return errf!("engraved content size cannot over 64 bytes")
    }
    let insc_ty = this.engraved_type.uint();
    if insc_ty <= 50 {
        if ! check_readable_string(this.engraved_content.as_ref()) {
            return errf!("engraved content must readable string")
        }
    }

    // cost
    let mut ttcost = Amount::zero();
    let pdhei = env.block.height;

    // do
    let mut state = CoreState::wrap(ctx.state());
    for dia in this.diamonds.list() {
        let cc = engraved_one_diamond(pdhei, &mut state, &main_addr, &dia, &this.engraved_content)?;
        ttcost = ttcost.add_mode_u64(&cc)?;
    }

	// check cost
	if pcost < &ttcost {
		return errf!("diamond inscription cost error need {} but got {}", ttcost, pcost)
	}

    // change count
    let mut ttcount = state.get_total_count();
    ttcount.diamond_engraved += this.diamonds.count().uint() as u64;
    ttcount.diamond_insc_burn_zhu += pcost.to_zhu_unsafe() as u64;
    state.set_total_count(&ttcount);

	// sub main addr balance
	if pcost.is_positive() {
        hac_sub(ctx, &main_addr, &pcost)?;
	}

    // finish
    Ok(vec![])

}




/************************************ */


action_define!{ DiamondInscriptionClear, 33, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        diamonds      : DiamondNameListMax200    
        protocol_cost : Amount
    },
    (self, ctx, _gas {
        diamond_inscription_clean(self, ctx)
    })
}



fn diamond_inscription_clean(this: &DiamondInscriptionClear, ctx: &mut dyn Context) -> Ret<Vec<u8>> {

    let env = ctx.env().clone();
    let main_addr = env.tx.main;
    let pcost = &this.protocol_cost;

    // check
    this.diamonds.check()?;
	if pcost.size() > 4 {
		return errf!("protocol cost amount size cannot over 4 bytes")
	}

    // cost
    let mut ttcost = Amount::zero();
    let pdhei = env.block.height;

    // do
    let mut state = CoreState::wrap(ctx.state());
    for dia in this.diamonds.list() {
        let cc = engraved_clean_one_diamond(pdhei, &mut state, &main_addr, &dia)?;
        ttcost = ttcost.add_mode_u64(&cc)?;
    }

	// check cost
	if pcost < &ttcost {
		return errf!("diamond inscription cost error need {} but got {}", ttcost, pcost)
	}

    // change count
    let mut ttcount = state.get_total_count();
    ttcount.diamond_insc_burn_zhu += pcost.to_zhu_unsafe() as u64;
    state.set_total_count(&ttcount);
	// sub main addr balance
	if pcost.is_positive() {
        hac_sub(ctx, &main_addr, &pcost)?;
	}

    // finish
    Ok(vec![])

}




















/************************************** */





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
    diasto.inscripts.push(content.clone()).unwrap();
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

