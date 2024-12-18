

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


lazy_static! {
    static ref MINER_PENDING_BLOCK: Arc<Mutex<VecDeque<MinerBlockStuff>>> = Arc::default();
}

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

pub fn create_coinbase_tx(hei: u64, msg: Fixed16, adr: Address) -> TransactionCoinbase {
    let rwdamt = block_reward(hei);
    TransactionCoinbase {
        ty      : Uint1::from(0), // ccoinbase type = 0
        address : adr,
        reward  : rwdamt,
        message : msg,
        extend  : CoinbaseExtend::must(CoinbaseExtendDataV1 {
            miner_nonce: Hash::default(),
            witness_count: Uint1::from(0),
        })
    }
}

fn miner_reset_next_new_block(sto: &BlockDisk, engine: ChainEngine, hnode: ChainNode) {

    let engcnf = engine.config();

    let mtckr = engine.mint_checker();
    let mtcnf = mtckr.config().downcast::<MintConf>().unwrap();
    let oldblk = engine.latest_block();
    
    let prevhash = oldblk.hash();
    let mut newdifn = oldblk.difficulty().clone();
    if *newdifn == 0 {
        newdifn = Uint4::from(LOWEST_DIFFICULTY);
    }
    let nexthei = oldblk.height().uint() + 1;
    // update difficulty number
    if nexthei % mtcnf.difficulty_adjust_blocks == 0 {
        let difn = mtckr.next_difficulty(oldblk.as_read(), sto);
        newdifn = Uint4::from(difn);
    }
    // create coinbase tx
    let cbtx = create_coinbase_tx(nexthei, engcnf.miner_message.clone(), 
        engcnf.miner_reward_address.clone());
    // create block v1
    let mut intro = BlockIntro {
        head: BlockHead {
            version           : Uint1::from(1),
            height            : BlockHeight::from(nexthei),
            timestamp         : Timestamp::from(curtimes()),
            prevhash          : prevhash,
            mrklroot          : Hash::default(),
            transaction_count : Uint4::default()
        },
        meta: BlockMeta {
            nonce         : Uint4::default(), 
            difficulty    : newdifn, 
            witness_stage : Fixed2::default()
        }
    };
    /* debug test
    // intro.head.timestamp = Timestamp::from(1723385108);
    // intro.meta.nonce = Uint4::from(4191621845);
    // cbtx.message = StringTrim16::must(&hex::decode("62616f6b756169000000000000006f56").unwrap());
    // test end*/
    // trs with cbtx
    let mut trslen: usize = 1;
    let mut trshxs: Vec<Hash> = vec![cbtx.hash()];
    // trs
    let mut transactions = DynVecTransaction::default();
    transactions.push(Box::new(cbtx.clone())).unwrap();
    
    append_valid_tx_pick_from_txpool( nexthei, 
        &mut trslen, &mut trshxs, &mut transactions, 
        engine.clone(), hnode.txpool(),
    );

    // set mrkl & trs count
    intro.head.mrklroot = calculate_mrklroot(&trshxs);
    intro.head.transaction_count = Uint4::from(trslen as u32);

    // ok
    let block = BlockV1 { intro, transactions};
    update_miner_pending_block(block, cbtx);
}



/*
    park txs to block
*/
fn append_valid_tx_pick_from_txpool(nexthei: u64, trslen: &mut usize, trshxs: &mut Vec<Hash>, 
    trs: &mut DynVecTransaction, engine: ChainEngine, txpool: Arc<dyn TxPool>,
) {
    let engcnf = engine.config();
    let txmaxn = engcnf.max_block_txs;
    let txmaxsz = engcnf.max_block_size;
    let mut allfee = Amount::zero();
    let mut txallsz: usize = 80; // coinbase tx size
    let txallsz = &mut txallsz;
    let mut invalidtxhxs = Vec::new();

    macro_rules! ok_push_one_tx {
        ($a: expr) => {
            trs.push($a.objc.clone()).unwrap();
            trshxs.push($a.hash.clone());
            *trslen += 1; 
        }
    }

    macro_rules! check_pick_one_tx {
        ($a: expr) => {
            let txr = $a.objc.as_ref().as_read();
            if let Err(..) = txr.verify_signature() {
                invalidtxhxs.push(txr.hash());
                return true // sign fail, ignore, next
            }
            if let Err(..) = engine.try_execute_tx(txr) {
                invalidtxhxs.push(txr.hash());
                return true // execute fail, ignore, next
            }
            let Ok(nf) = allfee.add_mode_u64(&$a.objc.fee_got()) else {
                invalidtxhxs.push(txr.hash());
                return true; // fee size err, ignore, next
            };
            allfee = nf;
        }

    }

    // pick one diamond mint tx
    if nexthei % 5 == 0 {
        let mut pick_dmint = |a: &TxPkg| {
            // check tx
            check_pick_one_tx!(a);
            // ok push
            ok_push_one_tx!(a);
            false // end
        };
        txpool.iter_at(&mut pick_dmint, TXPOOL_GROUP_DIAMOND_MINT).unwrap();
    }

    // pick normal tx
    let mut pick_normal_tx = |a: &TxPkg| {
        let txsz = a.data.len();
        // check tx
        check_pick_one_tx!(a);
        // check size
        if txsz + *txallsz > txmaxsz || *trslen >= txmaxn {
            return false // end, num or size enough
        }
        ok_push_one_tx!(a);
        true // next
    };
    txpool.iter_at(&mut pick_normal_tx, TXPOOL_GROUP_NORMAL).unwrap();

    // delete invalid txs
    if invalidtxhxs.len() > 0 {
        let _ = txpool.drain(&invalidtxhxs);
    }
    // ok
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



defineQueryObject!{ Q4391,
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
        asleep(1).await; // sleep 1 dec
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


defineQueryObject!{ Q2954,
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
            &BlockDisk::wrap(ctx.engine.disk()),
            ctx.engine.clone(),
            ctx.hcshnd.clone(),
        );
    }

    // return data
    get_miner_pending_block_stuff(detail, transaction, stuff, base64)
}




///////////////////////////////////////////////////


defineQueryObject!{ Q9347,
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

