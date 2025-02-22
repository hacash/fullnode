

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
    // println!("- devtest p2p recv new tx: {}, {}", txpkg.objc.hash().half(), hxfe.nonce());
    // check fee purity
    if txpkg.fepr < engcnf.lowest_fee_purity {
        return // tx fee purity too low to broadcast
    }
    if txpkg.data.len() > engcnf.max_tx_size {
        return // tx size overflow
    }
    let txdatas = txpkg.data.clone();
    let next_hei = this.engine.latest_block().height().uint() + 1;
    let txpr = txpkg.objc.as_read();
    // try execute and check tx
    if let Err(..) = this.engine.try_execute_tx(txpr) {
        return // tx execute fail
    }
    if let Err(..) = this.engine.mint_checker().tx_check(txpr, next_hei) {
        return // tx check fail
    }
    if engcnf.is_open_miner() {
        // add to tx pool
        let _ = this.txpool.insert(txpkg);
    }
    // broadcast
    let p2p = this.p2pmng.lock().unwrap();
    let p2p = p2p.as_ref().unwrap();
    p2p.broadcast_message(0/*not delay*/, knowkey, MSG_TX_SUBMIT, txdatas);
}


async fn handle_new_block(this: Arc<MsgHandler>, peer: Option<Arc<Peer>>, body: Vec<u8>) {
    let eng = this.engine.clone();
    let engcnf = eng.config();
    if body.len() > engcnf.max_block_size {
        return // block size overflow
    }
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
    let is_open_miner = engcnf.is_open_miner();
    let heispan = engcnf.unstable_block;
    let latest = eng.latest_block();
    let lathei = latest.height().uint();
    if blkhei > heispan && blkhei < lathei - heispan {
        return // height too late
    }
    let mintckr = eng.mint_checker();
    let stoptr = BlockDisk::wrap(eng.disk().clone());
    // may insert
    if blkhei <= lathei + 1 {
        // prepare check
        if let Err(_) = mintckr.prepare(&blkhead, &stoptr) {
            return  // difficulty check fail
        }
        // do insert  ◆ ◇ ⊙ ■ □ △ ▽ ❏ ❐ ❑ ❒  ▐ ░ ▒ ▓ ▔ ▕ ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▪ ▫    
        let hxstrt = &blkhx.as_bytes()[4..12];
        let hxtail = &blkhx.as_bytes()[30..];
        let txs = blkhead.transaction_count().uint() - 1;
        let _blkts = &timeshow(blkhead.timestamp().uint())[14..];
        // lock to inserting
        let isrlk = this.inserting.lock().unwrap();
        print!("❏ block {} …{}…{} txs{:2} insert at {} ", 
            blkhei, hex::encode(hxstrt), hex::encode(hxtail), txs, &ctshow()[11..]);
        let bodycp = body.clone();
        let engptr = eng.clone();
        let txpool = this.txpool.clone();
        // create block
        let blkpkg = BlockPkg::build(bodycp);
        if let Err(..) = blkpkg {
            return // parse error
        }
        let mut blkp = blkpkg.unwrap();
        blkp.set_origin( BlkOrigin::DISCOVER );
        let thsx = blkp.objc.transaction_hash_list(false); // hash no fee
        if let Err(e) = engptr.insert(blkp) {
            println!("Error: {}, failed.", e);
            // println!("- error block data hex: {}", body.hex());
        }else{
            println!("ok.");
            if is_open_miner {
                drain_all_block_txs(engptr.as_ref().as_read(), txpool.as_ref(), thsx, blkhei);
            }
        }
        drop(isrlk); // close lock
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


// drain all block txs
fn drain_all_block_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, txs: Vec<Hash>, blkhei: u64) {
    if blkhei % 15 == 0 {
        println!("{}.", txpool.print());
    }
    // drop all overdue diamond mint tx
    if blkhei % 5 == 0 {
        clean_invalid_diamond_mint_txs(eng, txpool, blkhei);
    }
    // drop all exist normal tx
    if txs.len() > 1 {
        let _ = txpool.drain(&txs[1..]); // over coinbase tx
    }
    // drop invalid normal
    if blkhei % 11 == 0 { // 1 hours
        clean_invalid_normal_txs(eng, txpool, blkhei);
    }
}


// clean_
fn clean_invalid_normal_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, blkhei: u64) {
    let pdhei = blkhei + 1;
    let mut sub_state = eng.sub_state();
    // already minted hacd number
    let _ = txpool.retain_at(&mut |a: &TxPkg| {
        let exec = eng.try_execute_tx_by( a.objc.as_read(), pdhei, &mut sub_state);
        exec.is_ok() // keep or delete 
    }, MemTxPool::NORMAL);
}


// clean_
fn clean_invalid_diamond_mint_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, _blkhei: u64) {
    // already minted hacd number
    let sta = eng.state();
    let curdn = CoreStateRead::wrap(sta.as_ref()).get_latest_diamond().number.uint();
    let nextdn = curdn + 1;
    let _ = txpool.retain_at(&mut |a: &TxPkg| {
        // must be next diamond number, or delete
        nextdn == get_diamond_mint_number(a.objc.as_read())
    }, MemTxPool::DIAMINT);
}



// for diamond create action
fn get_diamond_mint_number(tx: &dyn TransactionRead) -> u32 {
    const DMINT: u16 = DiamondMint::KIND;
    for act in tx.actions() {
        if act.kind() == DMINT {
            let dm = DiamondMint::must(&act.serialize());
            return *dm.d.number;
        }
    }
    0
}