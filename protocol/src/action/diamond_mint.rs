
/*
    if self.number.to_u32() > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        skn = self.custom_message.parse(buf, skn)?;
    }
*/
combi_struct_field_more_than_condition!{ DiamondMintData, {
    diamond              : DiamondName    
    number               : DiamondNumber    
    prev_hash            : Hash         
    nonce                : Fixed8        
    address              : Address   
}, custom_message, Hash, number, DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE
}


/*
* simple hac to
*/
action_define!{ DiamondMint, 4, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign,
    {
        d: DiamondMintData
    },
    (self, ctx, _gas {
        diamond_mint(self, ctx)      
    })
}


/*

*/
fn diamond_mint(this: &DiamondMint, ctx: &mut dyn Context) -> Ret<Vec<u8>> {
    let act = &this.d;
    act.address.check_must_privakey()?;

    let env = ctx.env().clone();
    let mut state = CoreState::wrap(ctx.state());

    let pending_height = env.block.height;
    let pending_hash = env.block.hash;

    let number = act.number;
    let dianum = number.uint();
    let name = act.diamond;
    let namestr = name.to_readable();
    let prev_hash = act.prev_hash;
    let nonce = act.nonce;
    let address = act.address;
    let mut custom_message = Vec::new();
    if dianum > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        custom_message = act.custom_message.serialize();
    }
    // check mine
    let (sha3hx, mediumhx, diahx) = x16rs::mine_diamond(dianum, &prev_hash, &nonce, &address, &custom_message);

    let not_fast_sync = false == env.chain.fast_sync;
    if not_fast_sync {

        // check
        if pending_hash.is_not_zero() && pending_height % 5 != 0 {
            return errf!("diamond must be contained in block height are highly divisible by 5")
        }
        // number
        let latest_diamond = state.get_latest_diamond();
        let neednextnumber = 1 + *latest_diamond.number;
        if dianum != neednextnumber {
            return errf!("diamond number need {} but got {}", neednextnumber, dianum)
        }
        // prev hash
        if dianum > 1 && latest_diamond.born_hash != prev_hash {
            return errf!("diamond prev hash need {} but got {}", latest_diamond.born_hash, prev_hash)
        }

        // latest
        let latestdianum = *latest_diamond.number;
        if dianum != 1 + latestdianum {
            return errf!("latest diamond number need {} but got {}", dianum - 1, latestdianum)
        }

        // difficulty
        let diffok = x16rs::check_diamond_difficulty(dianum, &sha3hx, &mediumhx);
        if ! diffok {
            return errf!("diamond difficulty not match")
        }

        // name
        let dianame = x16rs::check_diamond_hash_result(diahx);
        let Some(dianame) = dianame else {
            let dhx = match String::from_utf8(diahx.to_vec()) {
                Err(_) => hex::encode(diahx),
                Ok(d) => d
            };
            return errf!("diamond hash result {} not a valid diamond name", dhx)
        };
        let dianame = Fixed6::from(dianame);
        if name != dianame {
            return errf!("diamond name need {} but got {}", dianame.to_readable(), namestr)
        }

        // exist
        let hav = state.diamond(&name);
        if let Some(_) = hav {
            return errf!("diamond {} already exist", namestr)
        }

    }

    // tx fee
    let tx_bid_fee = &env.tx.fee;

    // total count 
    let mut ttcount = state.get_total_count();
    ttcount.minted_diamond += 1u64;
    if dianum > DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES {
        let mut sub = tx_bid_fee.clone();
        if sub.unit() > 1 {
            sub.unit_sub(1);
        }
        let burn = tx_bid_fee.clone().sub_mode_u64(&sub)?; // 90%
        ttcount.hacd_bid_burn_zhu += Uint8::from(burn.to_zhu_unsafe() as u64);
    }

    // gene
    let (life_gene, _visual_gene) = calculate_diamond_gene(dianum, &mediumhx, &diahx, &pending_hash, &tx_bid_fee);

    // bid_burn    
    let average_bid_burn = calculate_diamond_average_bid_burn(dianum, *ttcount.hacd_bid_burn_zhu);

    // save diamond smelt
    let diasmelt = DiamondSmelt {
        diamond: name.clone(),
        number: number.clone(),
        born_height: BlockHeight::from(pending_height),
        born_hash: pending_hash.clone(),
        prev_hash: prev_hash.clone(),
        miner_address: act.address.clone(),
        bid_fee: tx_bid_fee.clone(),
        nonce: nonce.clone(),
        average_bid_burn: average_bid_burn,
        life_gene: life_gene,
    };
    state.set_latest_diamond(&diasmelt);
    state.diamond_smelt_set(&name, &diasmelt);

    // save diamond
    let diaitem = DiamondSto {
        status: DIAMOND_STATUS_NORMAL,
        address: act.address.clone(),
        prev_engraved_height: BlockHeight::default(), // 0
        inscripts: Inscripts::default() // none
    };
    state.diamond_set(&name, &diaitem);
    state.diamond_name_set(&number, &name);

    // add diamond belong
    if env.chain.diamond_form {
        diamond_owned_push_one(&mut state, &address, &name);
    }
    
    // save count
    state.set_total_count(&ttcount);

    // add balance
    hacd_add(&mut state, &act.address, &DiamondNumber::from(1))?;

    // ok
    Ok(vec![])
}







