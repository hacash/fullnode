
macro_rules! q_acc {
    ( $pass: expr) => ({
        match SysAccount::create_by(&$pass) {
            Err(e) => return errf!("private key error: {}", &e),
            Ok(a) => a,
        }
    })
}



macro_rules! q_amt {
    ( $stuff: expr) => ({
        match Amount::from(&$stuff) {
            Err(e) => return errf!("amount error: {}", &e),
            Ok(a) => a,
        }
    })
}



macro_rules! q_adr {
    ( $addr: expr) => ({
        match Address::from_readable(&$addr) {
            Err(e) => return errf!("address error: {}", &e),
            Ok(a) => a,
        }
    })
}




