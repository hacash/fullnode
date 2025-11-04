
fn impl_packing_next_block(this: &HacashMinter, engine: &dyn EngineRead, txpool: &dyn TxPool) -> Box<dyn Any> {
        
    let engcnf = engine.config();

    let mtcnf = this.cnf;
    let oldblk = engine.latest_block();
    
    let prevhash = oldblk.hash();
    let mut newdifn = oldblk.difficulty().clone();
    if *newdifn == 0 {
        newdifn = Uint4::from(LOWEST_DIFFICULTY);
    }
    let nexthei = oldblk.height().uint() + 1;
    // update difficulty number
    if nexthei % mtcnf.difficulty_adjust_blocks == 0 {
        let sto = engine.store();
        let (difn, ..) = this.next_difficulty(oldblk.as_read(), sto.as_ref());
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
        engine, txpool,
    );

    // set mrkl & trs count
    intro.head.mrklroot = calculate_mrklroot(&trshxs);
    intro.head.transaction_count = Uint4::from(trslen as u32);

    // ok
    let block = BlockV1{ intro, transactions };

    Box::new(block)

}



pub fn create_coinbase_tx(hei: u64, msg: Fixed16, adr: Address) -> TransactionCoinbase {
    let rwdamt = genesis::block_reward(hei);
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




/*
    park txs to block
*/
fn append_valid_tx_pick_from_txpool(pending_hei: u64, trslen: &mut usize, trshxs: &mut Vec<Hash>, 
    trs: &mut DynVecTransaction, engine: &dyn EngineRead, txpool: &dyn TxPool,
) {
    let engcnf = engine.config();
    let txmaxn = engcnf.max_block_txs;
    let txmaxsz = engcnf.max_block_size;
    let mut allfee = Amount::zero();
    let mut txallsz: usize = 80; // 80 is coinbase tx size
    let txallsz = &mut txallsz;
    let mut invalidtxhxs = Vec::new();

    let mut sub_state = engine.fork_sub_state();

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
            if let Err(..) = engine.try_execute_tx_by(txr, pending_hei, &mut sub_state) {
                invalidtxhxs.push(txr.hash());
                return true // execute fail, ignore, next
            };
            let Ok(nf) = allfee.add_mode_u64(&$a.objc.fee_got()) else {
                invalidtxhxs.push(txr.hash());
                return true; // fee size err, ignore, next
            };
            allfee = nf;
        }

    }

    // pick one diamond mint tx
    if pending_hei % 5 == 0 {
        let mut pick_dmint = |a: &TxPkg| {
            // check tx
            check_pick_one_tx!(a);
            // ok push
            ok_push_one_tx!(a);
            false // end
        };
        txpool.iter_at(TXGID_DIAMINT, &mut pick_dmint).unwrap();
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
    txpool.iter_at(TXGID_NORMAL, &mut pick_normal_tx).unwrap();

    // delete invalid txs
    if invalidtxhxs.len() > 0 {
        let _ = txpool.drain(&invalidtxhxs);
    }
    // ok
}




/********************************************/



fn impl_tx_pool_refresh(_this: &HacashMinter, eng: &dyn EngineRead, txpool: &dyn TxPool, txs: Vec<Hash>, blkhei: u64) {

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
    let mut sub_state = eng.fork_sub_state();
    // already minted hacd number
    let _ = txpool.retain_at(TXGID_NORMAL, &mut |a: &TxPkg| {
        let exec = eng.try_execute_tx_by( a.objc.as_read(), pdhei, &mut sub_state);
        exec.is_ok() // keep or delete 
    });
}


// clean_
fn clean_invalid_diamond_mint_txs(eng: &dyn EngineRead, txpool: &dyn TxPool, _blkhei: u64) {
    // already minted hacd number
    let sta = eng.state();
    let sta = sta.as_ref();
    let curdn = CoreStateRead::wrap(sta.as_ref()).get_latest_diamond().number.uint();
    let nextdn = curdn + 1;
    let _ = txpool.retain_at(TXGID_DIAMINT, &mut |a: &TxPkg| {
        // must be next diamond number, or delete
        nextdn == action::get_diamond_mint_number(a.objc.as_read())
    });
}




