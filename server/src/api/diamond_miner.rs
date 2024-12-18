

/******************* diamondminer init *******************/


defineQueryObject!{ Q7846,
    ___nnn___, Option<bool>, None,
}

async fn diamondminer_init(State(ctx): State<ApiCtx>, _q: Query<Q7846>) -> impl IntoResponse {
    let cnf = ctx.engine.config();

    if ! cnf.dmer_enable {
        return api_error("diamond miner in config not enable");
    }

    let data = jsondata!{
        "bid_address", cnf.dmer_bid_account.readable(),
        "reward_address", cnf.dmer_reward_address.readable(),
    };

    api_data(data)
}


/******************* diamondminer success *******************/



defineQueryObject!{ Q6396,
    ___nnn___, Option<bool>, None,
}

async fn diamondminer_success(State(ctx): State<ApiCtx>, q: Query<Q6396>, body: Bytes) -> impl IntoResponse {
    ctx_state!(ctx, state);
    // q_must!(q, wait, 45); // 45 sec
    let cnf = ctx.engine.config();

    if ! cnf.dmer_enable {
        return api_error("diamond miner in config not enable");
    }

    let actdts = q_body_data_may_hex!(q, body);
    let Ok((mint, _)) = DiamondMint::create(&actdts) else {
        return api_error("upload action error");
    };

    let act = &mint.d;
    let mint_number = *act.number;
    let mint_name = act.diamond.to_readable();

    // check number and hash
    let lastdia = state.get_latest_diamond();
    if mint_number != *lastdia.number + 1 {
        return api_error("diamond number error");
    }
    if mint_number > 1 && act.prev_hash != lastdia.born_hash {
        return api_error("diamond prev hash error");
    }

    // create trs
    let bid_addr = Address::from(cnf.dmer_bid_account.address().clone());
    let mut tx = TransactionType2::new_by(bid_addr, cnf.dmer_bid_min.clone());
    tx.push_action(Box::new(mint)).unwrap();
    tx.fill_sign(&cnf.dmer_bid_account).unwrap();

    let txhx = tx.hash();

    // add to tx pool
    let txpkg = TxPkg::create(Box::new(tx));
    // try submit
    let in_async = true;
    if let Err(e) = ctx.hcshnd.submit_transaction(&txpkg, in_async) {
        return api_error(&e)
    }

    let hxstr = txhx.hex();
    println!("▒▒▒▒ DIAMOND SUCCESS: {}({}), tx hash: {}.", mint_name, mint_number, &hxstr);

    let data = jsondata!{
        "tx_hash", hxstr,
    };

    api_data(data)
}