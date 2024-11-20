

async fn handle_new_tx(this: Arc<MsgHandler>, peer: Option<Arc<Peer>>, body: Vec<u8>) {
    // println!("1111111 handle_txblock_arrive Tx, peer={} len={}", peer.nick(), body.clone().len());
    let engcnf = this.engine.config();
    // parse
    let Ok(txpkg) = TxPkg::build(body) else {
        return // parse tx error
    };
    // tx hash with fee
    let hxfe = txpkg.objc.hash_with_fee();
    let (already, knowkey) = check_know(&this.knows, &hxfe, peer.clone());
    if already {
        return  // alreay know it
    }
    // println!("p2p recv new tx: {}, {}", txpkg.objc().hash().half(), hxfe.nonce());
    let txdatas = txpkg.data.clone();
    if engcnf.is_open_miner() {
        // try execute tx
        if let Err(..) = this.engine.try_execute_tx(txpkg.objc.as_read()) {
            return // tx execute fail
        }
        // add to pool
        let _ = this.txpool.insert(txpkg);
    }
    // broadcast
    let p2p = this.p2pmng.lock().unwrap();
    let p2p = p2p.as_ref().unwrap();
    p2p.broadcast_message(0/*not delay*/, knowkey, MSG_TX_SUBMIT, txdatas);
}


async fn handle_new_block(this: Arc<MsgHandler>, peer: Option<Arc<Peer>>, body: Vec<u8>) {
    // println!("222222222222 handle_txblock_arrive Block len={}",  body.clone().len());
    let mut blkhead = BlockIntro::default();
    if let Err(_) = blkhead.parse(&body) {
        return // parse tx error
    }
    let blkhei = blkhead.height().uint();
    let blkhx = blkhead.hash();
    let (already, knowkey) = check_know(&this.knows, &blkhx, peer.clone());
    if already {
        return  // alreay know it
    }
    // check height and difficulty (mint consensus)
    let eng = this.engine.clone();
    let engcnf = eng.config();
    let is_open_miner = engcnf.is_open_miner();
    let heispan = engcnf.unstable_block;
    let latest = eng.latest_block();
    let lathei = latest.height().uint();
    if blkhei > heispan && blkhei < lathei - heispan {
        return // height too late
    }
    let mintckr = eng.mint_checker();
    let stoptr = eng.disk();
    // may insert
    if blkhei <= lathei + 1 {
        // prepare check
        if let Err(_) = mintckr.prepare(stoptr.as_ref(), &blkhead) {
            return  // difficulty check fail
        }
        // do insert  ◆ ◇ ⊙ ■ □ △ ▽ ❏ ❐ ❑ ❒  ▐ ░ ▒ ▓ ▔ ▕ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▪ ▫    
        let hxstrt = &blkhx.as_bytes()[4..12];
        let hxtail = &blkhx.as_bytes()[30..];
        let txs = blkhead.transaction_count().uint() - 1;
        let _blkts = &timeshow(blkhead.timestamp().uint())[14..];
        print!("❏ block {} …{}…{} txs{:2} insert at {} ", 
            blkhei, hex::encode(hxstrt), hex::encode(hxtail), txs, &ctshow()[11..]);
        let bodycp = body.clone();
        let engptr = eng.clone();
        let txpool = this.txpool.clone();
        std::thread::spawn(move||{
            // create block
            let blkpkg = BlockPkg::build(bodycp);
            if let Err(..) = blkpkg {
                return // parse error
            }
            let blkp = blkpkg.unwrap();
            let thsx = blkp.objc.transaction_hash_list(false); // hash no fee
            if let Err(e) = engptr.insert(blkp) {
                println!("Error: {}", e);
            }else{
                println!("ok.");
                if is_open_miner {
                    drain_all_block_txs(engptr.as_ref().as_read(), txpool.as_ref(), thsx, blkhei);
                }
            }
        });
    }else{
        // req sync
        if let Some(ref pr) = peer {
            send_req_block_hash_msg(pr.clone(), (heispan+1) as u8, lathei).await;
        }
        return // not broadcast
    }
    // broadcast new block
    let p2p = this.p2pmng.lock().unwrap();
    let p2p = p2p.as_ref().unwrap();
    p2p.broadcast_message(0/*not delay*/, knowkey, MSG_BLOCK_DISCOVER, body);
}



// return already know
fn check_know(mine: &Knowledge, hxkey: &Hash, peer: Option<Arc<Peer>>) -> (bool, KnowKey) {
    let knowkey: [u8; KNOWLEDGE_SIZE] = hxkey.clone().into_array();
    if let Some(ref pr) = peer {
        pr.knows.add(knowkey.clone());
    }
    if mine.check(&knowkey) {
        return (true, knowkey) // alreay know it
    }
    mine.add(knowkey.clone());
    (false, knowkey)
}


// drain_all_block_txs
fn drain_all_block_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, txs: Vec<Hash>, blkhei: u64) {
    if blkhei % 15 == 0 {
        println!("{}.", txpool.print());
    }
    // drop all exist normal tx
    if txs.len() > 0 {
        let _ = txpool.drain(&txs);
    }
    // drop all overdue diamond mint tx
    if blkhei % 5 == 0 {
        clean_invalid_diamond_mint_txs(eng, txpool, blkhei);
    }
    // drop invalid normal
    if blkhei % 48 == 0 { // 4 hours
        clean_invalid_normal_txs(eng, txpool, blkhei);
    }
}


// clean_
fn clean_invalid_normal_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, _blkhei: u64) {
    // already minted hacd number
    let _ = txpool.drain_filter_at(&|a: &TxPkg| {
        match eng.try_execute_tx( a.objc.as_read() ) {
            Err(..) => true, // delete
            _ => false,
        }
    }, TXPOOL_GROUP_NORMAL);
}


// clean_
fn clean_invalid_diamond_mint_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, _blkhei: u64) {
    // already minted hacd number
    let sta = eng.state();
    let curdn = CoreStateRead::wrap(sta).get_latest_diamond().number.uint();
    let _ = txpool.drain_filter_at(&|a: &TxPkg| {
        let tx = a.objc.as_read();
        let dn = get_diamond_mint_number(tx);
        // println!("TXPOOL: drain_filter_at dmint, tx: {}, dn: {}, last dn: {}", tx.hash().hex(), dn, ldn);
        dn <= curdn // is not next diamond, delete
    }, TXPOOL_GROUP_DIAMOND_MINT);
}



// for diamond create action
fn get_diamond_mint_number(tx: &dyn TransactionRead) -> u32 {
    const DMINT: u16 = DiamondMint::KIND;
    for act in tx.actions() {
        if act.kind() == DMINT {
            let dm = DiamondMint::must(&act.serialize());
            return dm.head.number.uint();
        }
    }
    0
}