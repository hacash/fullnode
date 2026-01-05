

#[allow(dead_code)]
pub struct MinerBlockStuff {
    height: BlockHeight,
    block_nonce: Uint4,
    coinbase_nonce: Hash,
    target_hash: Hash,
    coinbase_tx: TransactionCoinbase,
    block: BlockV1,
    mrklrts: Vec<Hash>,
}


use std::sync::LazyLock;
static MINER_PENDING_BLOCK: LazyLock<Arc<Mutex<VecDeque<MinerBlockStuff>>>> 
    = LazyLock::new(|| { Arc::default()});

fn update_miner_pending_block(block: BlockV1, cbtx: TransactionCoinbase) {
    let mkrluphxs = calculate_mrkl_coinbase_modify(&block.transaction_hash_list(true));
    let mut stfs = MINER_PENDING_BLOCK.lock().unwrap();
    stfs.push_front(MinerBlockStuff{
        height: block.height().clone(),
        block_nonce: Uint4::default(),
        coinbase_nonce: Hash::default(),
        target_hash: Hash::from(u32_to_hash(block.difficulty().uint())),
        coinbase_tx: cbtx,
        block: block,
        mrklrts: mkrluphxs,
    });
    // max 3
    if stfs.len() > 3 {
        stfs.pop_back();
    }
}


fn get_miner_pending_block_stuff(is_detail: bool, is_transaction: bool, is_stuff: bool, is_base64: bool) -> (HeaderMap, String) {
    let mut stuff = MINER_PENDING_BLOCK.lock().unwrap();
    if stuff.len() == 0 {
        panic!("get miner pending block stuff error: block not init!");
    };
    let stuff = &mut stuff[0];
    
    // update mkrl
    stuff.coinbase_nonce.increase(); // + 1
    stuff.coinbase_tx.set_nonce(stuff.coinbase_nonce);
    let cbhx = stuff.coinbase_tx.hash();
    let mkrl = calculate_mrkl_coinbase_update(cbhx, &stuff.mrklrts);
    stuff.block.set_mrklroot( mkrl );
    let intro_data = stuff.block.intro.serialize().hex();

    macro_rules! hex_or_hase64 {
        ($v: expr) => {
            match is_base64 {
                true => $v.to_base64(),
                false => $v.to_hex(),
            }
        }
    }

    // return data
    let mut tg_hash = stuff.target_hash.to_vec();
    right_00_to_ff(&mut tg_hash);
    let mut data = jsondata!{
        "height", *stuff.height,
        "coinbase_nonce", hex_or_hase64!(stuff.coinbase_nonce),
        "block_intro", intro_data,
        "target_hash", hex_or_hase64!(tg_hash),
    };

    if is_detail {
        let addition = jsondata!{
            "version", stuff.block.version().uint(),
            "prevhash", hex_or_hase64!(stuff.block.prevhash()),
            "timestamp",stuff.block.timestamp().uint(),
            "transaction_count", stuff.block.transaction_count().uint() - 1, // real tx
            "reward_address", stuff.coinbase_tx.main().readable(),
        };
        // data.append(&mut addition);
        let _ = addition.into_iter().map(|(k, v)| data.insert(k, v) ).collect::<Vec<_>>();
    }

    if is_transaction {
        // get raw tx
        let txs = stuff.block.transactions();
        let mut tx_raws = Vec::with_capacity(txs.len());
        for tx in txs {
            let raw = hex_or_hase64!(tx.serialize());
            tx_raws.push(raw);
        };
        data.insert("transaction_body_list", json!{tx_raws});
    }

    if is_stuff {
        let cbbody = hex_or_hase64!(stuff.coinbase_tx.serialize());
        data.insert("coinbase_body", json!{cbbody});
        let mkrluphxs = calculate_mrkl_coinbase_modify(&stuff.block.transaction_hash_list(true));
        let mut mhxs = Vec::with_capacity(mkrluphxs.len());
        for hx in mkrluphxs {
            let h = hex_or_hase64!(hx.serialize());
            mhxs.push(h);
        };
        data.insert("mkrl_modify_list", json!(mhxs));
    }

    // ok
    api_data(data)
}


fn miner_reset_next_new_block(engine: ChainEngine, txpool: &dyn TxPool) {

    let block = engine.mint_checker().packing_next_block(engine.as_read(), txpool);
    let block = *block.downcast::<BlockV1>().unwrap(); //
    let cbtx: Box<dyn Transaction> = block.transactions()[0].clone();
    let cbtx: TransactionCoinbase = match cbtx.ty() == 0 {
        true => TransactionCoinbase::must(&cbtx.serialize()),
        false => never!(),
    };
    update_miner_pending_block(block, cbtx);
}



///////////////////////////////////////////////////



struct MWNCount {
    count: Arc<Mutex<u64>>,
}
impl MWNCount {
    fn new(c: Arc<Mutex<u64>>) -> MWNCount {
        {
            *c.lock().unwrap() += 1;
        }
        MWNCount {
            count: c,
        }
    }
}
impl Drop for MWNCount {
    fn drop(&mut self) {
        {
            *self.count.lock().unwrap() -= 1;
        }
    }
}



api_querys_define!{ Q4391,
    height, u64, 0,
    rqid, String, s!(""), // must random query id
    wait, Option<u64>, None,
}

async fn miner_notice(State(ctx): State<ApiCtx>, q: Query<Q4391>) -> impl IntoResponse {
    q_must!(q, wait, 45); // 45 sec
    set_in_range!(wait, 1, 300);
    let mut lasthei = 0;
    let mut getlasthei = || {
        lasthei = ctx.engine.latest_block().height().uint();
        lasthei
    };
    // count + 1
    let mwnc = MWNCount::new(ctx.miner_worker_notice_count.clone());
    for _i in 0..wait {
        if getlasthei() >= q.height {
            break // finish!
        }
        asleep(1.0).await; // sleep 1 dec
    }
    drop(mwnc); // count - 1
    getlasthei();
    // return data
    let data = jsondata!{
        "height", lasthei,
    };
    api_data(data)
}


///////////////////////////////////////////////////


api_querys_define!{ Q2954,
    detail, Option<bool>, None,
    transaction, Option<bool>, None,
    stuff, Option<bool>, None,
}


async fn miner_pending(State(ctx): State<ApiCtx>, q: Query<Q2954>) -> impl IntoResponse {
    q_must!(q, detail, false);
    q_must!(q, transaction, false);
    q_must!(q, stuff, false); // coinbase and mkrl
    q_must!(q, base64, false);

    if ! ctx.engine.config().miner_enable {
        return api_error("miner not enable")
    }

    // get highest bid tx from other node

    // just for test develop
    #[cfg(not(debug_assertions))] 
    { 
        let gotdmintx = ctx.hcshnd.txpool().first_at(TXGID_DIAMINT).unwrap().is_some();
        if  ctx.engine.config().is_mainnet() && ! gotdmintx && curtimes() < ctx.launch_time + 30 {
            return api_error("miner worker need launch after 30 secs for node start")
        }
    }


    let lasthei = ctx.engine.latest_block().height().uint();

    let is_need_create_new = || {
        let stf = MINER_PENDING_BLOCK.lock().unwrap();
        if stf.len() == 0 {
            return true
        }
        let stf = &stf[0];
        if *stf.height <= lasthei {
            return true
        }
        // not need
        false
    };

    if is_need_create_new() {
        // create next block
        miner_reset_next_new_block(
            ctx.engine.clone(),
            ctx.hcshnd.txpool().as_ref(),
        );
    }

    // return data
    get_miner_pending_block_stuff(detail, transaction, stuff, base64)
}




///////////////////////////////////////////////////


api_querys_define!{ Q9347,
    height, u64, 0,
    block_nonce, u32, 0,
    coinbase_nonce, String, s!(""),
}


async fn miner_success(State(ctx): State<ApiCtx>, q: Query<Q9347>) -> impl IntoResponse {
    if ! ctx.engine.config().miner_enable {
        return api_error("miner not enable")
    }

    let mut success_stuff = {
        // search
        let mut stf = MINER_PENDING_BLOCK.lock().unwrap();
        let stfidx: usize = {
            if stf.len() == 0 {
                return api_error("pending block not yet")
            }
            let mut res: Option<usize> = None;
            for i in 0..stf.len() {
                let s = &stf[i];
                if *s.height == q.height {
                    res = Some(i);
                    break
                }
            }
            match res {
                Some(v) => v,
                None => return api_error(&format!("pending block height {} not find", q.height)),
            }
        };

        // find it
        let tarstf = &mut stf[stfidx];
        let Ok(coinbase_nonce) = hex::decode( &q.coinbase_nonce ) else {
            return api_error("coinbase nonce format error");
        };
        if coinbase_nonce.len() != Hash::SIZE {
            return api_error("coinbase nonce length error");
        }
        
        // check difficulty
        tarstf.block.set_nonce( Uint4::from(q.block_nonce) );
        tarstf.coinbase_tx.set_nonce( Hash::from(coinbase_nonce.try_into().unwrap()) );
        let cbhx = tarstf.coinbase_tx.hash();
        let mkrl = calculate_mrkl_coinbase_update(cbhx, &tarstf.mrklrts);
        tarstf.block.set_mrklroot( mkrl );
        let blkhx = tarstf.block.hash();
        // diff hash
        if 1 == hash_diff(&blkhx, &tarstf.target_hash) {
            return api_error(&format!(
                "difficulty check fail: at least need {} but got {}", 
                &tarstf.target_hash.hex(), &blkhx.hex(),
            ));
        }
        
        // mining successfully !!!
        // pick out
        let one = stf.drain(stfidx..stfidx+1).next_back().unwrap();
        one
    };

    // mining successfully !!!
    // replace coinbase tx
    let height = success_stuff.block.height().uint();
    success_stuff.block.replace_transaction(0, Box::new(success_stuff.coinbase_tx.clone())).unwrap();
    
    let blkpkg = BlockPkg::create(Box::new(success_stuff.block));

    // try submit
    let is_async = true;
    if let Err(e) = ctx.hcshnd.submit_block(&blkpkg, is_async) {
        return api_error(&format!("submit block error: {}", &e))
    }

    // return data
    let data = jsondata!{
        "height", height,
        "mining", "success",
    };
    api_data(data)
}



fn hash_diff(dst: &Hash, tar: &Hash) -> i8 {
    for i in 0..Hash::SIZE {
        if dst[i] > tar[i] {
            return 1
        }else if dst[i] < tar[i] {
            return -1
        }
    }
    // equarl
    0
}




/*







sync insert height: 574787, body: 

01
000008c543
0066b8c514
0000000000006fdb5b7a687a283733080ae845faf5653336f5acd31423130d78
8ec60a1c5f2b2531cbbdbf1dcae8f952c4a8be2128f9ba76e3cf0ae59876d631
00000001
f9d712d5
d3d64377
0000
0000
538b308868c9db1756fa62e80b890a84df72da80
f80108
62616f6b756169000000000000006f56
01
0000000000000000000000000000000000000000000000000000000000000000
00




*/

