

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
    pub hash:              String,
    pub hash_with_fee:     String,
    pub body:          String, // tx body with signature
    pub timestamp:     u64,
}







/*
    stuff is private key or password
*/
#[wasm_bindgen]
pub fn create_coin_transfer(param: CoinTransferParam) -> Ret<CoinTransferResult> {

    use protocol::transaction::*;
    use protocol::action::*;
    use protocol::interface::*;

    let main = q_acc!(param.main_prikey);
    let mainaddr = Address::from(main.address().clone());
    let mut from = main.clone();
    if ! param.from_prikey.is_empty() {
        from = q_acc!(param.from_prikey);
    }
    let fromaddr = Address::from(from.address().clone());
    let other_from = from != main;
    // let _from = q_acc!(param.from_prikey);
    let fee = q_amt!(param.fee);
    let toaddr = q_adr!(param.to_address);
    let ts  = param.timestamp;
    if ts == 0 {
        return errf!("timestamp must give")
    }

    // create trs
    let mut trsobj = TransactionType2::new_by(mainaddr, fee, ts);
    // append action
    // hac
    if ! param.hacash.is_empty() {
        let hac = match Amount::from(&param.hacash) {
            Err(e) => return errf!("hacash amount {} error: {}", param.hacash, &e),
            Ok(h) => h,
        };
        let act: Box<dyn Action> = maybe!(other_from, {
            let mut obj = HacFromToTrs::new();
            obj.from = AddrOrPtr::from_addr(fromaddr);
            obj.to = AddrOrPtr::from_addr(toaddr);
            obj.hacash = hac;
            Box::new(obj)
        }, {
            let mut obj = HacToTrs::new();
            obj.to = AddrOrPtr::from_addr(toaddr);
            obj.hacash = hac;
            Box::new(obj)
        });
        trsobj.push_action(act).unwrap();
    }
    // sat
    if param.satoshi > 0 {
        let sat = Satoshi::from(param.satoshi);
        let act: Box<dyn Action> = maybe!(other_from, {
            let mut obj = SatFromToTrs::new();
            obj.from = AddrOrPtr::from_addr(fromaddr);
            obj.to = AddrOrPtr::from_addr(toaddr);
            obj.satoshi = sat;
            Box::new(obj)
        }, {
            let mut obj = SatToTrs::new();
            obj.to = AddrOrPtr::from_addr(toaddr);
            obj.satoshi = sat;
            Box::new(obj)
        });
        trsobj.push_action(act).unwrap();
    }
    // hacd
    if param.diamonds.len() >= DiamondName::SIZE {
        let act: Box<dyn Action>;
        let dialist = match DiamondNameListMax200::from_readable(&param.diamonds) {
            Err(e) => return errf!("diamonds error: {}", &e),
            Ok(d) => d,
        };
        let act: Box<dyn Action> = maybe!(other_from, {
                let mut obj = DiaFromToTrs::new();
                obj.from = AddrOrPtr::from_addr(fromaddr);
                obj.to = AddrOrPtr::from_addr(toaddr);
                obj.diamonds = dialist;
                Box::new(obj)
            }, maybe!(dialist.length() == 1, {
                    let mut obj = DiaSingleTrs::new();
                    obj.to = AddrOrPtr::from_addr(toaddr);
                    obj.diamond = DiamondName::from(*dialist.list()[0]);
                    Box::new(obj)
                }, {
                    let mut obj = DiaToTrs::new();
                    obj.to = AddrOrPtr::from_addr(toaddr);
                    obj.diamonds = dialist;
                    Box::new(obj)
                }
            )
        );
        trsobj.push_action(act).unwrap();
    }
    // do sign
    if let Err(e) = trsobj.fill_sign(&main) {
        return errf!("fill main sgin error: {}", e)
    }
    if other_from {
        if let Err(e) = trsobj.fill_sign(&from) {
            return errf!("fill from sgin error: {}", e)
        }
    }
    // finish
    Ok(CoinTransferResult{
        hash: trsobj.hash().hex(),
        hash_with_fee: trsobj.hash_with_fee().hex(),
        body: trsobj.serialize().hex(),
        timestamp: ts,
    })
}
