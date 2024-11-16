


fn check_bidding_step(hnode: Arc<dyn HNode>, engcnf: &EngineConf, pending_height: u64, bidding_number: &mut u32) {
    if pending_height % 5 == 0  {
        return // not need bid in mining block tail 5 and 10
    }

    let txpool = hnode.txpool();
    let txplptr = txpool.as_ref();
    let my_acc = &engcnf.dmer_bid_account;
    let my_addr = Address::cons(*my_acc.address());

    macro_rules! retry {
        ($ms: expr) => {
            thread::sleep( Duration::from_millis($ms) );
            return
        }
    }
    
    macro_rules! printerr {
        ( $f: expr, $( $v: expr ),+ ) => {
            println!("\n\n{} {}\n\n", 
                "[Diamond Auto Build Error]",
                format!($f, $( $v ),+)
            );
        }
    }
    
    let Some(first_bid_txp) = pick_first_bid_tx(txplptr) else {
        retry!(3); // tx pool empty
    };

    let first_bid_addr = first_bid_txp.objc().address().unwrap();
    if my_addr == first_bid_addr {
        retry!(1); // im the first
    }

    let first_bid_fee = first_bid_txp.objc().fee();
    if first_bid_fee.more_than(&engcnf.dmer_bid_max) {
        retry!(10); // my max too low
    }

    let Some(my_bid_txp) = pick_my_bid_tx(txplptr, &my_addr) else {
        retry!(3); // have no my tx
    };

    let my_bid_addr = my_bid_txp.objc().address().unwrap();
    if my_bid_addr == first_bid_addr {
        retry!(1); // im the first
    }

    let my_bid_fee = my_bid_txp.objc().fee();
    if my_bid_fee.more_or_equal(&engcnf.dmer_bid_max) {
        retry!(5); // my fee up max
    }

    let Ok(mut new_bid_fee) = first_bid_fee.add(&engcnf.dmer_bid_step) else {
        printerr!("cannot add fee {} with {}, ", 
            &first_bid_fee.to_fin_string(), &engcnf.dmer_bid_step.to_fin_string()
        );
        retry!(10); // move step fail
    };
    let Ok(mut new_bid_fee) = new_bid_fee.compress(4, true) else {
        printerr!("cannot compress fee {} to 4 legnth", 
            &new_bid_fee.to_fin_string()
        );
        retry!(10); // move step fail
    };
    if new_bid_fee.more_than(&engcnf.dmer_bid_max) {
        new_bid_fee = engcnf.dmer_bid_max.clone()
    }
    if new_bid_fee.less_or_equal(first_bid_fee) {
        retry!(10); // my max too low
    }

    // ok
    if let Some(mint) = checkout_diamond_mint_action(my_bid_txp.objc().as_read()) {
        let dia = mint.head.diamond.readable();
        let dnum = mint.head.number.uint();
        let dfee = new_bid_fee.to_fin_string();
        if *bidding_number != dnum {
            *bidding_number = dnum;
            flush!("✵✵✵✵ Diamond Auto Bid {}({}) by {} raise fee to ⇨ {}", dia, dnum, my_addr.readable(), dfee);
        }else{
            flush!(" ⇨ {}", dfee);
        }
    }
    
    // raise fee
    let mut my_tx = my_bid_txp.objc().clone();
    my_tx.set_fee(new_bid_fee.clone());
    my_tx.fill_sign(&engcnf.dmer_bid_account);
    let txp: Box<dyn TxPkg> = Box::new(TxPackage::new(my_tx));

    // submit tx
    if let Err(e) = hnode.submit_transaction(&txp, false) {
        printerr!("ㄨㄨㄨ submit tx error: {}", e);
        retry!(3); // submit error
    }

    // next check step
}


///////////////////////////////////////////////


fn pick_my_bid_tx(tx_pool: &dyn TxPool, my_addr: &Address) -> Option<Box<dyn TxPkg>> {
    let mut my_bid_tx: Option<Box<dyn TxPkg>> = None;
    let mut pick_dmint = |a: &Box<dyn TxPkg>| {
        if *my_addr == a.objc().address().unwrap() {
            my_bid_tx = Some(a.clone());
            return false // end
        }
        true // next
    };
    tx_pool.iter_at(&mut pick_dmint, TXPOOL_GROUP_DIAMOND_MINT);
    // ok
    my_bid_tx
}




fn pick_first_bid_tx(tx_pool: &dyn TxPool) -> Option<Box<dyn TxPkg>> {
    let mut first: Option<Box<dyn TxPkg>> = None;
    let mut pick_dmint = |a: &Box<dyn TxPkg>| {
        first = Some(a.clone());
        return false // end
    };
    tx_pool.iter_at(&mut pick_dmint, TXPOOL_GROUP_DIAMOND_MINT);
    // ok
    first
}


// for diamond create action
fn checkout_diamond_mint_action(tx: &dyn TransactionRead) -> Option<mint_action::DiamondMint> {
    const DMINT: u16 = mint_action::ACTION_KIND_ID_DIAMOND_MINT;
    let mut num: u32 = 0;
    for act in tx.actions() {
        if act.kind() == DMINT {
            let dm = mint_action::DiamondMint::must(&act.serialize());
            return Some(dm);
        }
    }
    None
} 



