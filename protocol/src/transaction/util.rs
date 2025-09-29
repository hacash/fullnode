
/**
* verify tx all needs signature
*/
pub fn verify_tx_signature(tx: &dyn TransactionRead) -> Rerr {
    let hx = tx.hash();
    let hxwf = tx.hash_with_fee();
    let signs = tx.signs();
    let addrs = tx.req_sign()?;
    let main_addr = tx.main();
    let txty = tx.ty();
    for adr in addrs {
        let mut ckhx = &hx;
        if adr == main_addr && txty != TransactionType1::TYPE {
            ckhx = &hxwf;
        }
        verify_one_sign(ckhx, &adr, signs)?;
    }
    Ok(())
}


pub fn check_tx_signature(tx: &dyn TransactionRead) -> Ret<HashMap<Address, bool>> {
    let hx = tx.hash();
    let hxwf = tx.hash_with_fee();
    let signs = tx.signs();
    let addrs = tx.req_sign()?;
    let main_addr = tx.main();
    let txty = tx.ty();
    let mut ckres = HashMap::new();
    for sig in signs {
        let adr = Address::from(Account::get_address_by_public_key(*sig.publickey));
        ckres.insert(adr, true);
    }
    for adr in addrs {
        let mut ckhx = &hx;
        if adr == main_addr && txty != TransactionType1::TYPE {
            ckhx = &hxwf;
        }
        let mut sigok = false;
        if let Ok(yes) = verify_one_sign(ckhx, &adr, signs) {
            if yes {
                sigok = true;
            }
        }
        ckres.insert(adr, sigok);
    }
    Ok(ckres)
}


pub fn verify_target_signature(adr: &Address, tx: &dyn TransactionRead) -> Ret<bool> {
    let hx = tx.hash();
    let hxwf = tx.hash_with_fee();
    let signs = tx.signs();
    // let ddrs = tx.req_sign();
    let main_addr = tx.main();
    let mut ckhx = &hx;
    if *adr == main_addr{
        ckhx = &hxwf;
    }
    verify_one_sign(ckhx, adr, signs)
}


fn verify_one_sign(hash: &Hash, addr: &Address, signs: &Vec<Sign>) -> Ret<bool> {
    let adrary = addr.into_array();
    for sig in signs {
        let curpubkey = sig.publickey.into_array();
        let curaddr = Account::get_address_by_public_key(curpubkey);
        if adrary == curaddr {
            if Account::verify_signature(&hash.into_array(), &curpubkey, &sig.signature.into_array()) {
                return Ok(true)
            }
        }
    }
    errf!("{} verify signature failed", addr.readable())
}

