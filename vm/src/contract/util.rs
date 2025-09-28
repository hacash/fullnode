

#[allow(dead_code)]
pub fn curl_trs_1(acts: Vec<Box<dyn Action>>) {
    curl_trs_2(acts, "")
}

pub fn curl_trs_2(acts: Vec<Box<dyn Action>>, fee: &str) {
    let acc = Account::create_by_password("123456").unwrap();
    let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    let fee = Amount::from(maybe!(fee.len()>0 ,fee, "8:244")).unwrap();
    let trs = TransactionType3::new_by(addr, fee, curtimes());
    curl_trs_fee(trs, acts, acc)
}

#[allow(dead_code)]
pub fn curl_trs_3(acts: Vec<Box<dyn Action>>, fee: &str) {
    let acc = Account::create_by_password("123456").unwrap();
    let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    let addr2 = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();
    let fee = Amount::from(maybe!(fee.len()>0 ,fee, "8:244")).unwrap();
    let mut trs = TransactionType3::new_by(addr, fee, curtimes());
    trs.addrlist = AddrOrList::from_list(vec![addr, addr2]).unwrap();
    curl_trs_fee(trs, acts, acc)
}


#[allow(dead_code)]
pub fn curl_trs_fee(mut trs: TransactionType3, acts: Vec<Box<dyn Action>>, acc: Account) {

    for act in acts {
        trs.push_action(act).unwrap();
    }

    trs.gas_max = Uint1::from(4);
    trs.fill_sign(&acc).unwrap();

    println!("txsize:{}, feepay: {}, feegot: {}, feepurity: {}", 
        trs.size(), trs.fee_pay(), trs.fee_got(), trs.fee_purity() 
    );

    // print
    println!("\n\n");
    println!(r#"curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "{}""#, trs.serialize().hex());
    println!("\n");
}

