

pub fn verify_signature(hash: &Hash, addr: &Address, sign: &Sign) -> bool {
    let curpubkey = sign.publickey.to_array();
    let curaddr = Account::get_address_by_public_key(curpubkey);
    if addr.as_array() == &curaddr {
        if Account::verify_signature(hash.as_array(), &curpubkey, sign.signature.as_array()) {
            return true
        }
    }
    // failed
    false
}


