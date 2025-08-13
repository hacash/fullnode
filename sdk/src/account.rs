

#[wasm_bindgen(getter_with_clone)]
pub struct ExpAccount {
    prikey:      String,
    pubkey:      String,
    address:     String,
    address_hex: String,
}


/*
    stuff is private key or password
*/
#[wasm_bindgen]
pub fn create_account(pass: &str) -> Ret<ExpAccount> {
    Account::create_by(pass).map(|acc|{
        ExpAccount{
            prikey: hex::encode(&acc.secret_key().serialize()),
            pubkey: hex::encode(&acc.public_key().serialize_compressed()),
            address_hex: hex::encode(acc.address()),
            address: acc.readable().clone(),
        }
    })
}



