

/******************* diamond *******************/



api_querys_define!{ Q3946,
    name, Option<String>, None,
    number, Option<u32>, None,
}

async fn diamond(State(ctx): State<ApiCtx>, q: Query<Q3946>) -> impl IntoResponse {
    ctx_state!(ctx, state);
    q_unit!(q, unit);
    q_must!(q, name, s!(""));
    q_must!(q, number, 0);
    // id
    if number > 0 {
        let dian = state.diamond_name(&DiamondNumber::from(number));
        if let None = dian {
            return api_error("cannot find diamond")
        }
        name = dian.unwrap().to_readable();
    }else if !DiamondName::is_valid(&name.as_bytes()) {
        return api_error("diamond name error")
    }
    // data
    let dian = DiamondName::from(name.as_bytes().try_into().unwrap());
    let diaobj = state.diamond(&dian);
    if let None = diaobj {
        return api_error("cannot find diamond")
    }
    let diaobj = diaobj.unwrap();
    // load smelt
    let diasmelt = state.diamond_smelt(&dian);
    if let None = diasmelt {
        return api_error("cannot find diamond")
    }
    let diasmelt = diasmelt.unwrap();
    // return data
    let data = jsondata!{
        "name", dian.to_readable(),
        "belong", diaobj.address.readable(),
        "inscriptions", diaobj.inscripts.array(),
        // smelt
        "number", *diasmelt.number,
        "miner", diasmelt.miner_address.readable(),
        "born", jsondata!{
            "height", *diasmelt.born_height, // born block height
            "hash", diasmelt.born_hash.hex(), // born block hash
        },
        "prev_hash", diasmelt.prev_hash.hex(),
        "bid_fee", diasmelt.bid_fee.to_unit_string(&unit),
        "average_bid_burn", *diasmelt.average_bid_burn,
        "life_gene", diasmelt.life_gene.hex(),
        "visual_gene", calculate_diamond_visual_gene(&dian, &diasmelt.life_gene).hex(),
    };
    api_data(data)
}


/******************* diamond bidding *******************/


api_querys_define!{ Q8346,
    limit, Option<usize>, None,
    number, Option<usize>, None,
    since, Option<bool>, None,
}

async fn diamond_bidding(State(ctx): State<ApiCtx>, q: Query<Q8346>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    ctx_state!(ctx, state);
    let lastdia = state.get_latest_diamond();
    q_unit!(q, unit);
    q_must!(q, limit, 20);
    q_must!(q, number, 0);
    q_must!(q, since, false);
    let number = number as u32;

    let mut datalist = vec![];
    // load from txpool
    let txpool = ctx.hcshnd.txpool();

    // loop diamond mit tx
    let mut pick_dmint = |a: &TxPkg| {

        if datalist.len() >= limit {
            return false // end
        }
        let txhx = a.hash;
        let txr = a.objc.as_ref().as_read();
        let Some(diamtact) = pickout_diamond_mint_action(txr) else {
            return true // continue
        };
        let act = diamtact.d;
        if number > 0 && number != *act.number {
            return true // number not match, continue
        }
        // append
        let mut one = jsondata!{
            // "purity", a.fee purity(),
            "tx", txhx.hex(),
            "fee", txr.fee().to_unit_string(&unit),
            "bid", txr.main().readable(),
            "name", act.diamond.to_readable(),
            "belong", act.address.readable(),
        };
        if number == 0 {
            one.insert("number", json!(*act.number));
        }
        datalist.push(one);
        true // next

    };
    txpool.iter_at(&mut pick_dmint, MemTxPool::DIAMINT).unwrap();

    let mut data = jsondata!{
        "number", *lastdia.number + 1, // current bidding diamond
        "list", datalist,
    };

    if since {
        // let mut acution_start = curtimes(); 
        if let Ok(blk) = ctx.load_block( &store, &lastdia.born_height.to_string() ) {
            let acution_start = blk.objc.timestamp().uint();
            data.insert("since", json!(acution_start));
        }
    }

    // return data
    api_data(data)
}



/******************* diamond views *******************/


api_querys_define!{ Q5395,
    name, Option<String>, None,
    limit, Option<i64>, None,
    page, Option<i64>, None,
    start, Option<i64>, None,
    desc, Option<bool>, None,
}

async fn diamond_views(State(ctx): State<ApiCtx>, q: Query<Q5395>) -> impl IntoResponse {
    ctx_state!(ctx, state);
    let lastdianum = *state.get_latest_diamond().number as i64;
    q_unit!(q, unit);
    q_must!(q, limit, 20);
    q_must!(q, page, 1);
    q_must!(q, start, i64::MAX);
    q_must!(q, desc, false);
    q_must!(q, name, s!(""));

    if limit > 200 {
        limit = 200;
    }

    // load by list
    let mut datalist = vec![];

    let mut query_item = |dian: &DiamondName|{
        let Some(..) = state.diamond(dian) else {
            return
        };
        let Some(diasmelt) = state.diamond_smelt(dian) else {
            return
        };
        let data = jsondata!{
            "name", dian.to_readable(),
            "number", *diasmelt.number,
            "bid_fee", diasmelt.bid_fee.to_unit_string(&unit),
            "life_gene", diasmelt.life_gene.hex(),
            // "visual_gene", calculate_diamond_visual_gene(&dian, &diasmelt.life_gene).hex(),
        };
        datalist.push(data);
    };

    // read diamonds
    if name.len() >= DiamondName::SIZE {

        let Ok(names) = DiamondNameListMax200::from_readable(&name) else {
            return api_error("diamond name error")
        };
        for dian in names.list() {
            query_item(&dian);
        }

    }else{

        // ids
        let diarng = get_id_range(lastdianum, page, limit, start, desc);
        // println!("{:?}", diarng);
        for id in diarng {
            let Some(dian) = state.diamond_name(&DiamondNumber::from(id as u32)) else {
                continue
            };
            query_item(&dian);
        }
    }

    // return data
    api_data(jsondata!{
        "latest_number", lastdianum,
        "list", datalist,
    })
}




/******************* diamond engrave *******************/


api_querys_define!{ Q5733,
    height, u64, 0,
    txposi, Option<isize>, None, // -1,
    tx_hash, Option<bool>, None, // if return txhash
}

async fn diamond_engrave(State(ctx): State<ApiCtx>, q: Query<Q5733>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    q_must!(q, tx_hash, false);
    q_must!(q, txposi, -1);

    let mut datalist = vec![];

    // load block
    let blkpkg = ctx.load_block(&store, &q.height.to_string());
    if let Err(e) = blkpkg {
        return api_error(&e)
    }
    let blkobj = blkpkg.unwrap();
    let trs = blkobj.objc.transactions();
    if trs.len() == 0 {
        return api_error("transaction len error")
    }
    if txposi >= 0 {
        if txposi >= trs.len() as isize - 1 {
            return api_error("txposi overflow")
        }
    }

    // parse
    let pick_engrave = |tx: &dyn TransactionRead| -> Option<Vec<_>> {
        let mut res = vec![];
        let txhx = tx.hash();
        let mut append_one = |data: JsonObject| {
            let mut engobj = data;
            if tx_hash {
                engobj.insert("tx_hash", json!(txhx.to_hex()));
            }
            res.push(json!(engobj));
        };
        for act in tx.actions() {
            if act.kind() == DiamondInscription::KIND {
                let action = DiamondInscription::must(&act.serialize());
                append_one(jsondata!{
                    "diamonds", action.diamonds.readable(),
                    "inscription", action.engraved_content.to_readable_or_hex(),
                });
            }else if act.kind() == DiamondInscriptionClear::KIND {
                let action = DiamondInscriptionClear::must(&act.serialize());
                append_one(jsondata!{
                    "diamonds", action.diamonds.readable(),
                    "clear", true,
                });
            }
        }
        Some(res)
    };

    let tx_ary = match txposi >= 0 {
        true => { let i=txposi as usize; &trs[1..][i..i+1] },
        false => &trs[1..],
    };

    // ignore coinbase tx
    for tx in tx_ary {
        if let Some(mut egrs) = pick_engrave(tx.as_read()) {
            datalist.append(&mut egrs);
        }
    }

    // return data
    api_data(jsondata!{
        "list", datalist,
    })
}


/******************* diamond inscription protocol cost *******************/


api_querys_define!{ Q5543,
    name, String, s!(""), // diamond names
}

async fn diamond_inscription_protocol_cost(State(ctx): State<ApiCtx>, q: Query<Q5543>) -> impl IntoResponse {
    ctx_state!(ctx, state);
    q_unit!(q, unit);

    let Ok(names) = DiamondNameListMax200::from_readable(&q.name) else {
        return api_error("diamond name format or count error")
    };

    let mut cost = Amount::new();
    for dia in names.list() {
        let Some(diaobj) = state.diamond(dia) else {
            return api_error(&format!("cannot find diamond {}", dia))
        };
        if diaobj.inscripts.length() < 10 {
            continue // no need cost
        }
        let Some(diasmelt) = state.diamond_smelt(dia) else {
            return api_error(&format!("cannot find diamond {}", dia))
        };
        let camt = Amount::coin(*diasmelt.average_bid_burn as u64, 247);
        let Ok(newcost) = cost.add_mode_u128( &camt ) else {
            return api_error(&format!("cannot add cost {} and {}", camt, cost))
        };
        cost = newcost
    }

    // return data
    api_data(jsondata!{
        "cost", cost.to_unit_string(&unit),
    })

}

