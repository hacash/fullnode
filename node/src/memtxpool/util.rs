
fn scan_group_rng_by_feep(txpkgs: &Vec<TxPkg>, feep: u64, fee: &Amount, fpmd: bool, wsz: (usize, usize)) -> (usize, usize) {
    let mut rxl = wsz.0;
    let mut rxr = wsz.1;
    // scan rng
    loop {
        let rng = rxr-rxl;
        if rng <= 10 {
            break // end
        }
        let fct = rxl + rng/2;
        let ct = &txpkgs[fct];
        let lcd = match fpmd { true => feep > ct.fepr, false => fee > ct.objc.fee() }; // fee
        let rcd = match fpmd { true => feep < ct.fepr, false => fee < ct.objc.fee() }; // fee puery
        if lcd {
            rxr = fct; // in left
        } else if rcd {
            rxl = fct; // in right
        }else {
            // feep == cfp
            break // end
        }
    }
    // ok
    (rxl, rxr)
}

