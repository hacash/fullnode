

async fn handle_new_tx(this: Arc<MsgHandler>, peer: Option<Arc<Peer>>, body: Vec<u8>) {
    // println!("1111111 handle_txblock_arrive Tx, peer={} len={}", peer.nick(), body.clone().len());
    let engcnf = this.engine.config();
    let minter = this.engine.mint_checker();
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
    let txpr = txpkg.objc.as_read();
    // try execute and check tx
    if let Err(..) = this.engine.try_execute_tx(txpr) {
        return // tx execute fail
    }
    if let Err(..) = this.engine.mint_checker().tx_submit(this.engine.as_read(), &txpkg) {
        return // tx check fail
    }
    // add to tx pool
    let _ = this.txpool.insert_by(txpkg, &|tx|minter.tx_pool_group(tx));
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
    let heispan = engcnf.unstable_block;
    let latest = eng.latest_block();
    let lathei = latest.height().uint();
    if blkhei > heispan && blkhei < lathei - heispan {
        return // height too late
    }
    let mintckr = eng.mint_checker();
    let sto = eng.store();
    // may insert
    if blkhei <= lathei + 1 {
        // check block found
        if let Err(_) = mintckr.blk_found(&blkhead, &sto) {
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
        may_show_miner_detail(mintckr, &blkp);
        let thsx = blkp.objc.transaction_hash_list(false); // hash no fee
        if let Err(e) = engptr.discover(blkp) {
            println!("Error: {}, failed.", e);
            // println!("- error block data hex: {}", body.hex());
        }else{
            println!("ok.");
            mintckr.tx_pool_refresh(engptr.as_ref().as_read(), txpool.as_ref(), thsx, blkhei);
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


fn may_show_miner_detail(minter: &dyn Minter, blkp: &BlockPkg) {
    let Ok(cnf) = minter.config().downcast::<MintConf>() else {
        return
    };
    if !cnf.show_miner_name {
        return
    }
    // devtest start
    let Ok(cbtx) = blkp.objc.coinbase_transaction() else {
        return
    };
    let adrt = cbtx.main().readable().drain(..9).collect::<String>();
    print!("miner: {}...<{}> ", adrt, cbtx.message().to_readable_left());
    // devtest end
}

