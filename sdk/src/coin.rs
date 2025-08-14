

#[derive(Default)]
#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct CoinTransferParam {
    pub main_prikey: String,
    pub from_prikey: String,
    pub fee:         String,
    pub to_address:  String,
    pub timestamp:   u64,
    // coin
    pub hacash:      String,
    pub satoshi:     u64,
    pub diamonds:    String,
    // util
    pub chain_id:    u64,
}



#[wasm_bindgen]
impl CoinTransferParam {

    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

}



#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct CoinTransferResult {
    // hash:          String,
    // hash_with_fee: String,
    pub body:          String, // tx body with signature
    pub timestamp:     u64,
}







/*
    stuff is private key or password
*/
#[wasm_bindgen]
pub fn create_coin_transfer(param: CoinTransferParam) -> Ret<CoinTransferResult> {

    let main = q_acc!(param.main_prikey);
    let mut _from = main.clone();
    if ! param.from_prikey.is_empty() {
        _from = q_acc!(param.from_prikey);
    }
    // let _from = q_acc!(param.from_prikey);
    let fee = q_amt!(param.fee);
    let _to = q_adr!(param.to_address);
    let ts  = param.timestamp;

    if ts == 0 {
        return errf!("timestamp must give")
    }

    let mainaddr = Address::from(main.address().clone());

    // create trs
    let _trsobj = protocol::transaction::TransactionType2::new_by(mainaddr, fee, ts);
    


    Ok(CoinTransferResult{
        body: _trsobj.serialize().hex(),
        timestamp: ts,
    })
}
