

/******************* block intro *******************/



api_querys_define!{ Q2953,
    height, Option<u32>, None,
    hash, Option<String>, None,
    tx_hash_list, Option<bool>, None,
}

async fn block_intro(State(ctx): State<ApiCtx>, q: Query<Q2953>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    q_unit!(q, unit);
    q_must!(q, hash, s!(""));
    q_must!(q, height, 0);
    q_must!(q, tx_hash_list, false);
    // read
    let mut key = hash;
    if height > 0 {
        key = height.to_string();
    }
    let blkpkg = ctx.load_block(store.as_ref(), &key);
    if let Err(_) = blkpkg {
        return api_error("cannot find block")
    }
    let blkpkg = blkpkg.unwrap();
    let blkobj = &blkpkg.objc;
    let cbtx = create_recent_block_info(blkobj.as_read());
    
    // return data
    let txnum = blkobj.transaction_count().uint() as usize - 1; // drop coinbase
    let mut data = jsondata!{
        "hash", blkpkg.hash.hex(),
        // head
        "version", blkobj.version().uint(),
        "height", blkobj.height().uint(),
        "timestamp", blkobj.timestamp().uint(),
        "mrklroot", blkobj.mrklroot().hex(),
        "prevhash", blkobj.prevhash().hex(),
        // meta
        "nonce", blkobj.nonce().uint(),
        "difficulty", blkobj.difficulty().uint(),
        // coinbase
        "miner", cbtx.miner.readable(),
        "reward", cbtx.reward.to_unit_string(&unit),
        "message", cbtx.message,
        // tx list
        "transaction", txnum, // no coinbase
    };

    // tx_hash_list
    if tx_hash_list {
        let mut txhxs: Vec<String> = vec![];
        let alltrs = blkobj.transactions();
        for i in 1..txnum+1 {
            txhxs.push( alltrs[i].hash().hex() );
        }
        data.insert("tx_hash_list", json!(txhxs));
    }
    
    api_data(data)
}



/******************* block recents *******************/


api_querys_define!{ Q7456,
    __nnn__, Option<u32>, None,
}

async fn block_recents(State(ctx): State<ApiCtx>, q: Query<Q7456>) -> impl IntoResponse {
    q_unit!(q, unit);
    let mut datalist = vec![];

    for li in  ctx.engine.recent_blocks() {
        datalist.push(jsondata!{
            "height", li.height,
            "hash", li.hash.hex(),
            "prev", li.prev.hex(),
            "txs", li.txs - 1,
            "miner", li.miner.readable(),
            "message", li.message,
            "reward", li.reward.to_unit_string(&unit),
            "time", li.time,
            "arrive", li.arrive,
        });
    }

    // ok
    api_data(jsondata!{
        "list", datalist,
    })
}






/******************* block views *******************/


api_querys_define!{ Q4935,
    limit, Option<i64>, None,
    page, Option<i64>, None,
    start, Option<i64>, None,
    desc, Option<bool>, None,
}

async fn block_views(State(ctx): State<ApiCtx>, q: Query<Q4935>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    let lasthei = ctx.engine.latest_block().height().uint() as i64;
    q_unit!(q, unit);
    q_must!(q, limit, 20);
    q_must!(q, page, 1);
    q_must!(q, start, i64::MAX);
    q_must!(q, desc, false);
    if limit > 200 {
        limit = 200;
    }

    // load by list
    let mut datalist = vec![];

    // ids
    let diarng = get_id_range(lasthei, page, limit, start, desc);
    // println!("{:?}", diarng);
    for id in diarng {
        let Some((blkhx, blkdts)) = store.block_data_by_height(&BlockHeight::from(id as u64)) else {
            continue
        };
        let dts = blkdts.as_ref();
        let Ok((intro, seek)) = BlockIntro::create(dts) else {
            continue
        };
        let Ok((cbtx, _)) = TransactionCoinbase::create(&dts[seek..]) else {
            continue
        };
        let data = jsondata!{
            "height", intro.height().uint(),
            "hash", blkhx.hex(),
            "msg", cbtx.message().to_readable_left(),
            "reward", cbtx.reward().to_unit_string(&unit),
            "miner", cbtx.main().readable(),
            "time", intro.timestamp().uint(),
            "txs", intro.transaction_count().uint() - 1,
        };
        datalist.push(data);
    }

    // return data
    api_data(jsondata!{
        "latest_height", lasthei,
        "list", datalist,
    })
}





/******************* block datas *******************/


api_querys_define!{ Q8538,
    start_height, Option<u64>, None,
    limit, Option<u64>, None,
    max_size, Option<usize>, None,
    confirm, Option<bool>, None, // only confirm block
}

async fn block_datas(State(ctx): State<ApiCtx>, q: Query<Q8538>) -> impl IntoResponse {
    ctx_store!(ctx, store);
    let unsblk = ctx.engine.config().unstable_block;
    let mut lasthei = ctx.engine.latest_block().height().uint() as u64;
    const MB: usize = 1024*1024;
    q_must!(q, hexbody, false);
    q_must!(q, base64body, false);
    q_must!(q, start_height, 0);
    q_must!(q, limit, u64::MAX);
    q_must!(q, max_size, MB); // 1mb
    q_must!(q, confirm, false);
    if max_size > 10*MB {
        max_size = 10*MB;
    }
    if confirm && lasthei > unsblk {
        lasthei -= unsblk; // -4
    }

    // blocks
    let mut alldatas: Vec<u8> = Vec::with_capacity(max_size);

    let mut count: u64 = 0;
    for hei in start_height..u64::MAX {
        if hei > lasthei {
            break // end
        }
        if count >= limit {
            break // ok
        }
        if alldatas.len() >= max_size {
            break // ok
        }
        // load
        let Some((_, mut blkdts)) = store.block_data_by_height(&BlockHeight::from(hei)) else {
            break
        };
        alldatas.append(&mut blkdts);
        count += 1;
    }

    // convert
    if hexbody {
        alldatas = alldatas.hex().into_bytes();
    }else if base64body {
        alldatas = alldatas.base64().into_bytes();
    }

    // return raw data
    alldatas
}


