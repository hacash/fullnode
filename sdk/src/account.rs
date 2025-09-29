

#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct Account {
    pub prikey:      String,
    pub pubkey:      String,
    pub address:     String,
    pub address_hex: String,
}


/*
    stuff is private key or password
*/
#[wasm_bindgen]
pub fn create_account(pass: &str) -> Ret<Account> {
    SysAccount::create_by(pass).map(|acc|{
        Account{
            prikey: hex::encode(&acc.secret_key().serialize()),
            pubkey: hex::encode(&acc.public_key().serialize_compressed()),
            address_hex: hex::encode(acc.address()),
            address: acc.readable().clone(),
        }
    })
}




/*
    verify address 
*/
#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct VerifyAddressResult {
    pub ok: bool,
    pub error:  String,
}

#[wasm_bindgen]
pub fn verify_address(pass: &str) -> VerifyAddressResult {
    let re = |e| VerifyAddressResult{ ok: false, error: e };

    let addr = match Address::from_readable(pass) {
        Ok(a) => a,
        Err(e) => return re(e.to_string())
    };

    if let Err(e) = addr.check_version() {
        return re(e.to_string())
    }

    // ok 
    VerifyAddressResult{ ok: true, error: "".into() }
}



