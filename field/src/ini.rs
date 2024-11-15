


pub fn ini_must_address(sec: &HashMap<String, Option<String>>, key: &str) -> Address {
    let adr = ini_must(sec, key, "1AVRuFXNFi3rdMrPH4hdqSgFrEBnWisWaS");
    let Ok(addr) = Address::from_readable(&adr) else {
        panic!("[Config Error] address {} format error.", &adr)
    };
    addr
}


pub fn ini_must_amount(sec: &HashMap<String, Option<String>>, key: &str) -> Amount {
    let amt = ini_must(sec, key, "1:248");
    let Ok(amount) = Amount::from(&amt) else {
        panic!("[Config Error] amount {} format error.", &amt)
    };
    amount
}



