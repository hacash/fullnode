

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
        /*
        VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
        oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L
        cmhfWCVLLosyQujfPnf86spZVW4exD2yr
        WzK23CAKQFzoPpMEioBztv9yaASvJxNZM
        ezFkqc6Smyk5DGvMY6bMoYx6vsU4gs7ba
        UJ7Ypo4SpQibMudmEjJKbMUN7Zy9viyKS
        cCBdc3vTmsBzPXbn2SaQy6dfbpvM6aJmK
        bX96F9rJNYSBi3iE7vj2bQ75ChaTq5KsU
        SckiYHndzCkKApYhAa9fK2vLfkAunN3w3
        Td6MYJaoEbwo9JdebnCfcZs9qPAKuJz8A
        */
    let acc = Account::create_by_password("123456").unwrap();
    let addrs = vec![
        Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap(),
        Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap(),
        Address::from_readable("nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK").unwrap(),
        Address::from_readable("hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH").unwrap(),
        Address::from_readable("oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L").unwrap(),
        Address::from_readable("cmhfWCVLLosyQujfPnf86spZVW4exD2yr").unwrap(),
        Address::from_readable("WzK23CAKQFzoPpMEioBztv9yaASvJxNZM").unwrap(),
        Address::from_readable("ezFkqc6Smyk5DGvMY6bMoYx6vsU4gs7ba").unwrap(),
        Address::from_readable("UJ7Ypo4SpQibMudmEjJKbMUN7Zy9viyKS").unwrap(),
        Address::from_readable("cCBdc3vTmsBzPXbn2SaQy6dfbpvM6aJmK").unwrap(),
        Address::from_readable("bX96F9rJNYSBi3iE7vj2bQ75ChaTq5KsU").unwrap(),
        Address::from_readable("SckiYHndzCkKApYhAa9fK2vLfkAunN3w3").unwrap(),
        Address::from_readable("Td6MYJaoEbwo9JdebnCfcZs9qPAKuJz8A").unwrap(),
    ];
    let fee = Amount::from(maybe!(fee.len()>0 ,fee, "8:244")).unwrap();
    let mut trs = TransactionType3::new_by(addrs[0], fee, curtimes());
    trs.addrlist = AddrOrList::from_list(addrs).unwrap();
    curl_trs_fee(trs, acts, acc)
}


#[allow(dead_code)]
pub fn curl_trs_fee(mut trs: TransactionType3, acts: Vec<Box<dyn Action>>, acc: Account) {

    for act in acts {
        trs.push_action(act).unwrap();
    }

    trs.gas_max = Uint1::from(8);
    trs.fill_sign(&acc).unwrap();

    // println!("txsize:{}, feepay: {}, feegot: {}, feepurity: {}", 
    //     trs.size(), trs.fee_pay(), trs.fee_got(), trs.fee_purity() 
    // );

    // print
    println!("\n");
    println!(r#"curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "{}""#, trs.serialize().hex());
    // println!("\n");
}

