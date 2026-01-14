
// json string
pub fn action_from_json(_main_addr: &Address, jsonv: &serde_json::Value) -> Ret<Box<dyn Action>> {
    let Some(..) = jsonv.as_object() else {
        return errf!("action format error")
    };

    let Some(kind) = jsonv["kind"].as_u64() else {
        return errf!("kind format error")
    };
    if kind > u16::MAX.into(){
        return errf!("kind {} value overflow", kind)
    }
    let kind = kind as u16;

    macro_rules! j_addr {
        ($k: expr) => ({
            let Some(adr) = jsonv[$k].as_str() else {
                return errf!("address format error")
            };
            let Ok(adrobj) = Address::from_readable(adr) else {
                return errf!("address {} error", adr)
            };
            AddrOrPtr::from_addr(adrobj)
        })
    }

    macro_rules! j_hac { // hac
        ($k: expr) => ({
            let Some(amt) = jsonv[$k].as_str() else {
                return errf!("amount format error")
            };
            let Ok(amtobj) = Amount::from(amt) else {
                return errf!("amount {} error", amt)
            };
            amtobj
        })
    }

    macro_rules! j_sat { // satoshi
        ($k: expr) => ({
            let Some(sat) = jsonv[$k].as_u64() else {
                return errf!("satoshi format error")
            };
            Satoshi::from(sat)
        })
    }

    macro_rules! j_dias { // diamonds
        ($k: expr) => ({
            let Some(dias) = jsonv[$k].as_str() else {
                return errf!("diamonds format error")
            };
            let dialist = DiamondNameListMax200::from_readable(dias);
            if let Err(e) = dialist {
                return errf!("diamonds {} error",  &e)
            }
            dialist.unwrap()
        })
    }

    macro_rules! j_uint {
        ($k: expr, $t1: ty, $t2: ty) => ({
            let Some(num) = jsonv[$k].as_u64() else {
                return errf!("{} format error", stringify!($k))
            };
            if num > <$t1>::MAX.into() {
                return errf!("{} value overflow", stringify!($k))
            }
            <$t2>::from(num as $t1)
        })
    }

    macro_rules! j_uint1 {
        ($k: expr) => (
            j_uint!($k, u8, Uint1)
        )
    }

    macro_rules! j_uint4 {
        ($k: expr) => (
            j_uint!($k, u32, Uint4)
        )
    }

    macro_rules! j_uint5 {
        ($k: expr) => (
            j_uint!($k, u64, Uint5)
        )
    }

    macro_rules! j_bytes {
        ($k: expr, $t1: ty, $t2: ty) => ({
            let Some(btstr) = jsonv[$k].as_str() else {
                return errf!("{} format error", stringify!($k))
            };
            let bts = match hex::decode(btstr) {
                Ok(b) => b,
                _ => btstr.as_bytes().to_vec(),
            };
            if bts.len() > <$t1>::MAX.into() {
                return errf!("{} length overflow", stringify!($k))
            }
            <$t2>::from(bts).unwrap()
        })
    }

    macro_rules! j_bytes1 {
        ($k: expr) => (
            j_bytes!($k, u8, BytesW1)
        )
    }

    macro_rules! ret_act {
        ( $cls: ident, $( $k: ident, $v: expr)+ ) => {
            Ok(Box::new({let mut act = <$cls>::new();
                $(
                    act.$k = $v;
                )+
                act
            }))
        }
    }

    macro_rules! if_ret_act {
        ( $cls: ident, $( $k: ident, $v: expr)+ ) => {
            if kind == <$cls>::KIND {
                return ret_act!{ $cls, 
                    $(
                        $k, $v
                    )+
                }
            }
        }
    }
    macro_rules! if_ret_act_ns {
        ( $cls: ident, $( $k: ident, $g: ident)+ ) => {
            if kind == <$cls>::KIND {
                return ret_act!{ $cls, 
                    $(
                        $k, $g!( stringify!($k) )
                    )+
                }
            }
        }
    }

    /*********** Hacash ***********/

    if_ret_act_ns!{ HacToTrs,
        to,       j_addr
        hacash,   j_hac
    }

    if_ret_act_ns!{ HacFromTrs, 
        from,     j_addr
        hacash,   j_hac
    }

    if_ret_act_ns!{ HacFromToTrs, 
        from,     j_addr
        to,       j_addr
        hacash,   j_hac
    }

    /*********** Satoshi ***********/

    if_ret_act_ns!{ SatToTrs,
        to,       j_addr
        satoshi,  j_sat
    }

    if_ret_act_ns!{ SatFromTrs, 
        from,     j_addr
        satoshi,  j_sat
    }

    if_ret_act_ns!{ SatFromToTrs, 
        from,     j_addr
        to,       j_addr
        satoshi,  j_sat
    }

    /*********** Diamond ***********/

    if_ret_act!{ DiaSingleTrs,
        to,       j_addr!("to")
        diamond,  j_dias!("diamonds")[0]
    }

    if_ret_act_ns!{ DiaToTrs,
        to,       j_addr
        diamonds, j_dias
    }

    if_ret_act_ns!{ DiaFromTrs, 
        from,     j_addr
        diamonds, j_dias
    }

    if_ret_act_ns!{ DiaFromToTrs, 
        from,     j_addr
        to,       j_addr
        diamonds, j_dias
    }

    if_ret_act_ns!{ DiamondInscription,
        diamonds,         j_dias
        protocol_cost,    j_hac
        engraved_type,    j_uint1
        engraved_content, j_bytes1
    }

    if_ret_act_ns!{ DiamondInscriptionClear,
        diamonds,         j_dias
        protocol_cost,    j_hac
    }


    /*********** Other ***********/


    if_ret_act_ns!{ SubmitHeightLimit,
        start,         j_uint5
        end,           j_uint5
    }

    if_ret_act_ns!{ SubChainID,
        chain_id,      j_uint4
    }


    // not support
    return errf!("kind {} not support", kind)
}



// json string
pub fn action_to_json_desc(tx: &dyn TransactionRead, act: &Box<dyn Action>, 
    unit: &str, ret_kind: bool, ret_desc: bool
) -> JsonObject {

    let adrs = &tx.addrs();
    let main_addr = tx.main().readable();
    let kind = act.kind();

    let mut resjsonobj = jsondata!{
        "kind", kind,
    };

    let mut description: String = "".to_owned();
    macro_rules! set_jsonobj { ( $( $p:expr, )+ ) => {
        resjsonobj = jsondata!{ $($p,)+ };
    }}
    macro_rules! must_addr { ( $k:expr ) => {
        $k.real(adrs).unwrap().readable()
    }}
    macro_rules! append_description { ($( $p:expr ),+) => {
        if ret_desc { description = format!( $( $p ),+); }
    }}

    if let Some(a) = HacToTrs::downcast(act) {

        let to_addr = must_addr!(a.to);
        let amt_str = a.hacash.to_unit_string(unit);
        set_jsonobj!{
            "from", main_addr,
            "to", to_addr,
            "hacash", amt_str,
        };
        append_description!(
            "Transfer {} HAC from {} to {}",
            amt_str, main_addr, to_addr
        );

    } else if let Some(a) = HacFromTrs::downcast(act) {

        let from_addr = must_addr!(a.from);
        let amt_str = a.hacash.to_unit_string(unit);
        set_jsonobj!{
            "from", from_addr,
            "to", main_addr,
            "hacash", amt_str,
        };
        append_description!(
            "Transfer {} HAC from {} to {}",
            amt_str, from_addr, main_addr
        );

    }else if let Some(a) = HacFromToTrs::downcast(act) {

        let from_addr = must_addr!(a.from);
        let to_addr = must_addr!(a.to);
        let amt_str = a.hacash.to_unit_string(unit);
        set_jsonobj!{
            "from", from_addr,
            "to", to_addr,
            "hacash", amt_str,
        };
        append_description!(
            "Transfer {} HAC from {} to {}",
            amt_str, from_addr, to_addr
        );
    

    /*************** satoshi ***************/


    }else if let Some(a) = SatToTrs::downcast(act) {

        let to_addr = must_addr!(a.to);
        let amt_str = *a.satoshi;
        set_jsonobj!{
            "from", main_addr,
            "to", to_addr,
            "satoshi", amt_str,
        };
        append_description!(
            "Transfer {} SAT from {} to {}",
            amt_str, main_addr, to_addr
        );

    }else if let Some(a) = SatFromTrs::downcast(act) {

        let from_addr = must_addr!(a.from);
        let amt_str = *a.satoshi;
        set_jsonobj!{
            "from", from_addr,
            "to", main_addr,
            "satoshi", amt_str,
        };
        append_description!(
            "Transfer {} SAT from {} to {}",
            amt_str, from_addr, main_addr
        );

    }else if let Some(a) = SatFromToTrs::downcast(act) {

        let from_addr = must_addr!(a.from);
        let to_addr = must_addr!(a.to);
        let amt_str = *a.satoshi;
        set_jsonobj!{
            "from", from_addr,
            "to", to_addr,
            "satoshi", amt_str,
        };
        append_description!(
            "Transfer {} SAT from {} to {}",
            amt_str, from_addr, to_addr
        );
    

    /*************** Diamonds ***************/


    }else if let Some(a) = DiaSingleTrs::downcast(act) {

        let to_addr = must_addr!(a.to);
        let dia_num = 1u32;
        let dia_names = a.diamond.to_readable();
        set_jsonobj!{
            "from", main_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        append_description!(
            "Transfer {} HACD ({}) from {} to {}",
            dia_num, dia_names, main_addr, to_addr
        );

    }else if let Some(a) = DiaToTrs::downcast(act) {

        let to_addr = must_addr!(a.to);
        let dia_num = a.diamonds.length();
        let dia_names = a.diamonds.readable();
        set_jsonobj!{
            "from", main_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        append_description!(
            "Transfer {} HACD ({}) from {} to {}",
            dia_num, a.diamonds.splitstr(), main_addr, to_addr
        );

    }else if let Some(a) = DiaFromTrs::downcast(act) {
        
        let from_addr = must_addr!(a.from);
        let dia_num = a.diamonds.length();
        let dia_names = a.diamonds.readable();
        set_jsonobj!{
            "from", from_addr,
            "to", main_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        append_description!(
            "Transfer {} HACD ({}) from {} to {}",
            dia_num, a.diamonds.splitstr(), from_addr, main_addr
        );

    }else if let Some(a) = DiaFromToTrs::downcast(act) {

        let from_addr = must_addr!(a.from);
        let to_addr = must_addr!(a.to);
        let dia_num = a.diamonds.length();
        let dia_names = a.diamonds.readable();
        set_jsonobj!{
            "from", from_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        append_description!(
            "Transfer {} HACD ({}) from {} to {}",
            dia_num, a.diamonds.splitstr(), from_addr, to_addr
        );


    /*************** Diamond mint & inscription ***************/


    }else if let Some(a) = DiamondMint::downcast(act) {

        let act = &a.d;
        let name = act.diamond.to_readable();
        let miner = act.address.readable();
        set_jsonobj!{
            "name", name,
            "number", *act.number,
            "miner", miner,
            "nonce", act.nonce.hex(),
            "prev_hash", act.prev_hash.hex(), // prev block hash
            "custom_message", act.custom_message.hex(),
        };
        append_description!(
            "Mint HACD ({}) to {}", name, miner
        );

    }else if let Some(a) = DiamondInscription::downcast(act) {

        let dia_num = a.diamonds.length();
        let dia_names = a.diamonds.readable();
        let cost_str = a.protocol_cost.to_unit_string(unit);
        let ins_str = a.engraved_content.to_readable_or_hex();
        set_jsonobj!{
            "diamond", dia_num,
            "diamonds", dia_names,
            "protocol_cost", cost_str,
            "engraved_type", *a.engraved_type,
            "engraved_content", ins_str,
        };
        if ret_desc {
            let mut desc = format!("Inscript {} HACD ({}) with \"{}\"",
                dia_num, a.diamonds.splitstr(), ins_str);
            if a.protocol_cost.is_positive() {
                desc += &format!("  cost {} HAC fee", cost_str);
            }
            description = desc;
        }

    }else if let Some(a) = DiamondInscriptionClear::downcast(act) {

        let dia_num = a.diamonds.length();
        let dia_names = a.diamonds.readable();
        let cost_str = a.protocol_cost.to_unit_string(unit);
        set_jsonobj!{
            "diamond", dia_num,
            "diamonds", dia_names,
            "protocol_cost", cost_str,
        };
        append_description!(
            "Clean inscript {} HACD ({}) cost {} HAC fee",
            dia_num, a.diamonds.splitstr(), cost_str
        );



    /*************** Channel ***************/

    }else if let Some(a) = ChannelOpen::downcast(act) {

        let cid =   a.channel_id.hex();
        let l_adr = a.left_bill.address.readable();
        let l_amt = a.left_bill.amount.to_unit_string(unit);
        let r_adr = a.right_bill.address.readable();
        let r_amt = a.right_bill.amount.to_unit_string(unit);
        set_jsonobj!{
            "channel_id", cid,
            "left", jsondata!{
                "address", l_adr,
                "hacash", l_amt,
            },
            "right", jsondata!{
                "address", r_adr,
                "hacash", r_amt,
            },
        };
        append_description!(
            "Open channel {} with left {}: {}, right {}: {}",
            cid, l_adr, l_amt, r_adr, r_amt
        );


    }else if let Some(a) = ChannelClose::downcast(act) {

        let cid = a.channel_id.hex();
        set_jsonobj!{
            "channel_id", cid,
        };
        append_description!(
            "Close channel {}", cid
        );


    /*************** Others ***************/

    }else if let Some(a) = SubmitHeightLimit::downcast(act) {
        
        let s_hei = *a.start;
        let e_hei = *a.end;
        set_jsonobj!{
            "start_height", s_hei,
            "end_height", e_hei,
        };
        let e_hei = maybe!(e_hei== 0, 
            "Unlimited".to_owned(), 
            e_hei.to_string()
        );
        append_description!(
            "Limit height range ({}, {}) ",
            s_hei, e_hei
        );

    }else if let Some(a) = SubChainID::downcast(act) {
        
        let cid = *a.chain_id;
        set_jsonobj!{
            "chain_id", cid,
        };
        append_description!(
            "Valid chain ID {}", cid
        );

    }else{

    }
    
    if ret_desc {
        resjsonobj.insert("description", json!(description));
    }

    // ok
    if ret_kind {
        resjsonobj.insert("kind", json!(kind));
    }
    return resjsonobj
}