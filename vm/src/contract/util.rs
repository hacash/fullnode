

#[allow(dead_code)]
pub fn curl_trs_1(acts: Vec<Box<dyn Action>>) {

    let acc = Account::create_by_password("123456").unwrap();
    let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    let fee = Amount::small(8, 244);

    let mut trs = TransactionType3::new_by(addr, fee, curtimes());
    
    for act in acts {
        trs.push_action(act).unwrap();
    }

    trs.gas_max = Uint1::from(4);
    trs.fill_sign(&acc).unwrap();

    // print
    println!("\n\n");
    println!(r#"curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "{}""#, trs.serialize().hex());
    println!("\n");
}


#[allow(dead_code)]
pub fn curl_trs_fee(acts: Vec<Box<dyn Action>>, fee: Amount) {

    let acc = Account::create_by_password("123456").unwrap();
    let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();

    let mut trs = TransactionType3::new_by(addr, fee, curtimes());
    
    for act in acts {
        trs.push_action(act).unwrap();
    }

    trs.gas_max = Uint1::from(4);
    trs.fill_sign(&acc).unwrap();

    // print
    println!("\n\n");
    println!(r#"curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "{}""#, trs.serialize().hex());
    println!("\n");
}

