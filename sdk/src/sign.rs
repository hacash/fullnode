

#[derive(Default)]
#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct SignTxParam {
    pub prikey: String,
    pub body:   String, // hex
}



#[wasm_bindgen]
impl SignTxParam {

    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

}



#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct SignTxResult {
    pub hash:          String,
    pub hash_with_fee: String,
    pub body:          String, // tx body with signature
    pub signature:     String,
    pub timestamp:     u64,    // tx timestamp
}





/*
    sign one tx
*/
#[wasm_bindgen]
pub fn sign_transaction(param: SignTxParam) -> Ret<SignTxResult> {

    use protocol::transaction;

    let acc = q_acc!(param.prikey);
    // let accadr = Address::from(acc.address().clone());
    let Ok(body) = hex::decode(&param.body) else {
        return errf!("tx body error")
    };
    let Ok((mut trs, _)) = transaction::create(&body) else {
        return errf!("tx parse error")
    };
    let Ok(signature) = trs.fill_sign(&acc) else {
        return errf!("do sign error")
    };
    // ok finish
    Ok(SignTxResult {
        hash:          trs.hash().hex(),
        hash_with_fee: trs.hash_with_fee().hex(),
        body:          trs.serialize().hex(), 
        signature:     signature.signature.hex(),
        timestamp:     trs.timestamp().uint(),
    })
}






