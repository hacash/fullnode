

/******************* transaction sign *******************/



defineQueryObject!{ Q8375,
    // do sign
    prikey, Option<String>, None,
    // append
    pubkey, Option<String>, None,
    sigdts, Option<String>, None,
    //
    signature, Option<bool>, None,
    description, Option<bool>, None,
}


async fn transaction_sign(State(ctx): State<ApiCtx>, q: Query<Q8375>, body: Bytes) -> impl IntoResponse {
    q_unit!(q, unit);
    q_must!(q, prikey, s!(""));
    q_must!(q, pubkey, s!(""));
    q_must!(q, sigdts, s!(""));
    q_must!(q, signature, false);
    q_must!(q, description, false);

    let lasthei = ctx.engine.latest_block().height().uint();

    let txdts = q_body_data_may_hex!(q, body);
    let Ok((mut tx, _)) = transaction::create(&txdts) else {
        return api_error("transaction body error")
    };

    let (address, signobj) = match prikey.len() == 64 {
        true => {
            let Ok(prik) = hex::decode(&prikey) else {
                return api_error("prikey format error")
            };
            let Ok(acc) = Account::create_by_secret_key_value(prik.try_into().unwrap()) else {
                return api_error("prikey data error")
            };
            let fres = tx.fill_sign(&acc);
            if let Err(e) = fres {
                return api_error(&format!("fill sign error: {}", e))
            }
            (Address::from(*acc.address()), fres.unwrap())
        },
        false => {
            // replace
            if pubkey.len() != 33*2 || sigdts.len() != 64*2 {
                return api_error("pubkey or signature data error")
            }
            let Ok(pbk) = hex::decode(&pubkey) else {
                return api_error("pubkey format error")
            };
            let Ok(sig) = hex::decode(&sigdts) else {
                return api_error("sigdts format error")
            };
            let pbk: [u8; 33] = pbk.try_into().unwrap();
            let sig: [u8; 64] = sig.try_into().unwrap();
            let signobj = Sign{
                publickey: Fixed33::from( pbk ),
                signature: Fixed64::from( sig ),
            };
            if let Err(e) = tx.push_sign(signobj.clone()) {
                return api_error(&format!("fill sign error: {}", e))
            }
            (Address::from(Account::get_address_by_public_key(pbk)), signobj)
        },
    };

    // return info
    let mut data = render_tx_info(tx.as_read(), None, lasthei, &unit, true, signature, false, description);
    data.insert("sign_data", json!(jsondata!{
        "address", address.readable(),
        "pubkey", signobj.publickey.hex(),
        "sigdts", signobj.signature.hex(),
    }));
    api_data(data)

}





/******************* transaction build *******************/



defineQueryObject!{ Q2856,
    action, Option<bool>, None,
    signature, Option<bool>, None,
    description, Option<bool>, None,
}


/*
{
    main_address: "",
    timestamp: 123,
    fee: "",
    actions: [{...}],
}
*/
async fn transaction_build(State(_ctx): State<ApiCtx>, q: Query<Q2856>, body: Bytes) -> impl IntoResponse {
    q_unit!(q, unit);
    q_must!(q, action, false);
    q_must!(q, signature, false);
    q_must!(q, description, false);

    let txjsondts = q_body_data_may_hex!(q, body);
    let Ok(jsonstr) = std::str::from_utf8(&txjsondts) else {
        return api_error("transaction json body error")
    };
    let Ok(jsonv) = serde_json::from_str::<serde_json::Value>(jsonstr) else {
        return api_error("transaction json body error")
    };

    macro_rules! j_addr {
        ($k: expr) => ({
            let Some(adr) = jsonv[$k].as_str() else {
                return api_error("address format error")
            };
            let Ok(adrobj) = Address::from_readable(adr) else {
                return api_error(&format!("address {} error", adr))
            };
            adrobj
        })
    }

    macro_rules! j_hac { // hac
        ($k: expr) => ({
            let Some(amt) = jsonv[$k].as_str() else {
                return api_error("amount format error")
            };
            let Ok(amtobj) = Amount::from(amt) else {
                return api_error(&format!("amount {} error", amt))
            };
            amtobj
        })
    }

    // create trs
    let main_addr = j_addr!("main_address");
    let mut tx = TransactionType2::new_by(main_addr.clone(), j_hac!("fee"));
    if let Some(ts) = jsonv["timestamp"].as_u64() {
        tx.timestamp = Timestamp::from(ts);
    }

    // insert actions
    let Some(acts) = jsonv["actions"].as_array() else {
        return api_error("actions format error")
    };
    for act in acts {
        let a = action_from_json(&main_addr, &act);
        if let Err(e) = a {
            return api_error(&format!("push action error: {}", &e))
        }
        if let Err(e) = tx.push_action( a.unwrap()) {
            return api_error(&format!("push action error: {}", &e))
        }
    }

    // return info
    api_data(
        render_tx_info(tx.as_read(), None, 0, &unit, true, signature, action, description)
    )

}






/******************* transaction check *******************/



defineQueryObject!{ Q9764,
    set_fee, Option<String>, None,
    sign_address, Option<String>, None,
    body, Option<bool>, None,
    signature, Option<bool>, None,
    description, Option<bool>, None,
}


async fn transaction_check(State(_ctx): State<ApiCtx>, q: Query<Q9764>, bodydata: Bytes) -> impl IntoResponse {
    q_unit!(q, unit);
    q_must!(q, set_fee, s!(""));
    q_must!(q, sign_address, s!(""));
    q_must!(q, body, false);
    q_must!(q, signature, false);
    q_must!(q, description, false);

    let txdts = q_body_data_may_hex!(q, bodydata);
    let Ok((mut tx, _)) = transaction::create(&txdts) else {
        return api_error("transaction body error")
    };

    // if set fee
    if set_fee.len() > 0 {
        let fee = q_amt!(set_fee);
        tx.set_fee(fee);
    }

    let tx = tx.as_read();
    let main_addr = tx.main();

    let mut data = render_tx_info(tx, None, 0, &unit, body, signature, true, description);

    // sign_address
    if sign_address.len() > 0 {
        let addr = q_addr!(sign_address);
        let sign_hash = match main_addr == addr {
            true => tx.hash_with_fee(),
            false => tx.hash(),
        };
        data.insert("sign_hash", json!(sign_hash.hex()));
    }

    // return info
    api_data(data)

}



/******************* transaction exist *******************/



defineQueryObject!{ Q3457,
    hash, Option<String>, None,
    body, Option<bool>, None,
    action, Option<bool>, None,
    signature, Option<bool>, None,
    description, Option<bool>, None,
}


async fn transaction_exist(State(ctx): State<ApiCtx>, q: Query<Q3457>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    ctx_state!(ctx, state);
    q_unit!(q, unit);
    q_must!(q, hash, s!(""));
    q_must!(q, body, false);
    q_must!(q, action, false);
    q_must!(q, signature, false);
    q_must!(q, description, false);

    let lasthei = ctx.engine.latest_block().height().uint();

    macro_rules! tx_info {
        ($tx: expr, $ifblk: expr) => {
            render_tx_info($tx.as_read(), $ifblk, lasthei, &unit, 
                body, signature, action, description
            )
        }
    }

    // parse tx hash
    let Ok(hx) = hex::decode(&hash) else {
        return api_error("transaction hash format error")
    };
    if hx.len() != 32 {
        return api_error("transaction hash format error")
    }
    let txhx = Hash::must(&hx);

    // find from txpool
    let txpool = ctx.hcshnd.txpool();
    if let Some(txp) = txpool.find(&txhx) {
        let mut info = tx_info!(txp.objc, None);
        info.insert("pending", json!(true));
        return api_data(info)
    }

    // load from disk block data
    let Some(txp) = state.tx_exist(&txhx) else {
        return api_error("transaction not find")
    };
    let bkey = txp.to_string();
    let blkpkg = ctx.load_block(&store, &bkey);
    if let Err(_) = blkpkg {
        return api_error("cannot find block by transaction ptr")
    }
    let blkpkg = blkpkg.unwrap();
    let blkobj = &blkpkg.objc;
    let blktrs = blkobj.transactions();

    // search tx hash
    let tx = match (|hx: &Hash|{
        let txnum = blkobj.transaction_count().uint() as usize; // drop coinbase
        for i in 1..txnum {
            if *hx == blktrs[i].hash() {
                return Some(blktrs[i].clone())
            }
        }
        None
    })(&txhx) {
        None => return api_error("transaction not find in the block"),
        Some(tx) => tx,
    };

    // return info
    api_data(tx_info!(tx, Some(blkobj.as_read())))
}





////////////////////////////////////


/*
* params: belong_block_obj, 
*/
fn render_tx_info(tx: &dyn TransactionRead, 
    blblk: Option<&dyn BlockRead>, lasthei: u64, unit: &String, 
    body: bool, signature: bool, action: bool, description: bool,
) -> JsonObject {


    let fee_str = tx.fee().to_unit_string(unit);
    let main_addr = tx.main().readable();
    let mut data = jsondata!{
        // tx
        "hash", tx.hash().hex(),
        "hash_with_fee", tx.hash_with_fee().hex(),
        "type", tx.ty(),
        "timestamp", tx.timestamp().uint(),
        "fee", fee_str,
        "fee_got", tx.fee_got().to_unit_string(unit),
        "main_address", main_addr,
        "action", tx.action_count().uint(),
    };

    if body {
        data.insert("body", json!(tx.serialize().hex()));
    }

    if signature {
        check_signature(&mut data, tx);
    }

    if description {
        data.insert("description", json!(format!(
            "Main account {} pay {} HAC tx fee",
            main_addr, fee_str
        )));
    }

    if let Some(blkobj) = blblk {
        let txblkhei = blkobj.height().uint();
        // belong block info
        data.insert("block", json!(jsondata!{
            "height", txblkhei,
            "timestamp", blkobj.timestamp().uint(),
        }));
        // confirm block height
        data.insert("confirm", json!(lasthei - txblkhei));
    }

    if action {
        let acts = tx.actions();
        let mut actobjs = Vec::with_capacity(acts.len());
        for act in acts {
            actobjs.push( action_to_json_desc(tx, act.as_ref(), unit, true, description) );
        }
        data.insert("actions", json!(actobjs));
    }

    data
}



fn check_signature(data: &mut JsonObject, tx: &dyn TransactionRead) {
    let Ok(sigstats) = check_tx_signature(tx) else {
        return
    };
    let mut sigchs = vec![];
    for (adr, sg) in sigstats {
        sigchs.push(jsondata!{
            "address", adr.readable(),
            "complete", sg, // is sign ok
        });
    }
    data.insert("signatures", json!(sigchs));
}


