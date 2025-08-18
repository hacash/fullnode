

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
    pub timestamp:     u64,    // tx timestamp
}





/*
    stuff is private key or password
*/
#[wasm_bindgen]
pub fn sign_tx(param: SignTxParam) -> Ret<SignTxResult> {

    Ok(SignTxResult {
        hash:          "".into(),
        hash_with_fee: "".into(),
        body:          "".into(), 
        timestamp:     0,
    })
}






