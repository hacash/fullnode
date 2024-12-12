
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

    if_ret_act_ns!{ HacToTransfer,
        to,       j_addr
        hacash,   j_hac
    }

    if_ret_act_ns!{ HacFromTransfer, 
        from,     j_addr
        hacash,   j_hac
    }

    if_ret_act_ns!{ HacFromToTransfer, 
        from,     j_addr
        to,       j_addr
        hacash,   j_hac
    }

    /*********** Satoshi ***********/

    if_ret_act_ns!{ SatoshiToTransfer,
        to,       j_addr
        satoshi,  j_sat
    }

    if_ret_act_ns!{ SatoshiFromTransfer, 
        from,     j_addr
        satoshi,  j_sat
    }

    if_ret_act_ns!{ SatoshiFromToTransfer, 
        from,     j_addr
        to,       j_addr
        satoshi,  j_sat
    }

    /*********** Diamond ***********/

    if_ret_act!{ DiamondSingleTransfer,
        to,       j_addr!("to")
        diamond,  j_dias!("diamonds")[0]
    }

    if_ret_act_ns!{ DiamondToTransfer,
        to,       j_addr
        diamonds, j_dias
    }

    if_ret_act_ns!{ DiamondFromTransfer, 
        from,     j_addr
        diamonds, j_dias
    }

    if_ret_act_ns!{ DiamondFromToTransfer, 
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
pub fn action_to_json_desc(tx: &dyn TransactionRead, act: &dyn Action, 
    unit: &String, ret_kind: bool, ret_desc: bool
) -> JsonObject {

    let adrs = &tx.addrs();
    let main_addr = tx.main().readable();
    let kind = act.kind();

    let mut resjsonobj = jsondata!{
        "kind", kind,
    };

    let mut scan_by_any = |actany: &dyn Any| {

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

        if let Some(action) = actany.downcast_ref::<HacToTransfer>() {

            let to_addr = must_addr!(action.to);
            let amt_str = action.hacash.to_unit_string(unit);
            set_jsonobj!{
                "from", main_addr,
                "to", to_addr,
                "hacash", amt_str,
            };
            append_description!(
                "Transfer {} HAC from {} to {}",
                amt_str, main_addr, to_addr
            );

        }else if let Some(action) = actany.downcast_ref::<HacFromTransfer>() {

            let from_addr = must_addr!(action.from);
            let amt_str = action.hacash.to_unit_string(unit);
            set_jsonobj!{
                "from", from_addr,
                "to", main_addr,
                "hacash", amt_str,
            };
            append_description!(
                "Transfer {} HAC from {} to {}",
                amt_str, from_addr, main_addr
            );

        }else if let Some(action) = actany.downcast_ref::<HacFromToTransfer>() {

            let from_addr = must_addr!(action.from);
            let to_addr = must_addr!(action.to);
            let amt_str = action.hacash.to_unit_string(unit);
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


        }else if let Some(action) = actany.downcast_ref::<SatoshiToTransfer>() {

            let to_addr = must_addr!(action.to);
            let amt_str = *action.satoshi;
            set_jsonobj!{
                "from", main_addr,
                "to", to_addr,
                "satoshi", amt_str,
            };
            append_description!(
                "Transfer {} SAT from {} to {}",
                amt_str, main_addr, to_addr
            );

        }else if let Some(action) = actany.downcast_ref::<SatoshiFromTransfer>() {

            let from_addr = must_addr!(action.from);
            let amt_str = *action.satoshi;
            set_jsonobj!{
                "from", from_addr,
                "to", main_addr,
                "satoshi", amt_str,
            };
            append_description!(
                "Transfer {} SAT from {} to {}",
                amt_str, from_addr, main_addr
            );

        }else if let Some(action) = actany.downcast_ref::<SatoshiFromToTransfer>() {

            let from_addr = must_addr!(action.from);
            let to_addr = must_addr!(action.to);
            let amt_str = *action.satoshi;
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


        }else if let Some(action) = actany.downcast_ref::<DiamondSingleTransfer>() {

            let to_addr = must_addr!(action.to);
            let dia_num = 1u32;
            let dia_names = action.diamond.to_readable();
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

        }else if let Some(action) = actany.downcast_ref::<DiamondToTransfer>() {

            let to_addr = must_addr!(action.to);
            let dia_num = action.diamonds.length();
            let dia_names = action.diamonds.readable();
            set_jsonobj!{
                "from", main_addr,
                "to", to_addr,
                "diamond", dia_num,
                "diamonds", dia_names,
            };
            append_description!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), main_addr, to_addr
            );

        }else if let Some(action) = actany.downcast_ref::<DiamondFromTransfer>() {
            
            let from_addr = must_addr!(action.from);
            let dia_num = action.diamonds.length();
            let dia_names = action.diamonds.readable();
            set_jsonobj!{
                "from", from_addr,
                "to", main_addr,
                "diamond", dia_num,
                "diamonds", dia_names,
            };
            append_description!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), from_addr, main_addr
            );

        }else if let Some(action) = actany.downcast_ref::<DiamondFromToTransfer>() {

            let from_addr = must_addr!(action.from);
            let to_addr = must_addr!(action.to);
            let dia_num = action.diamonds.length();
            let dia_names = action.diamonds.readable();
            set_jsonobj!{
                "from", from_addr,
                "to", to_addr,
                "diamond", dia_num,
                "diamonds", dia_names,
            };
            append_description!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), from_addr, to_addr
            );


        /*************** Diamond mint & inscription ***************/


        }else if let Some(action) = actany.downcast_ref::<DiamondMint>() {

            let act = &action.d;
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

        }else if let Some(action) = actany.downcast_ref::<DiamondInscription>() {

            let dia_num = action.diamonds.length();
            let dia_names = action.diamonds.readable();
            let cost_str = action.protocol_cost.to_unit_string(unit);
            let ins_str = action.engraved_content.to_readable_or_hex();
            set_jsonobj!{
                "diamond", dia_num,
                "diamonds", dia_names,
                "protocol_cost", cost_str,
                "engraved_type", *action.engraved_type,
                "engraved_content", ins_str,
            };
            if ret_desc {
                let mut desc = format!("Inscript {} HACD ({}) with \"{}\"",
                    dia_num, action.diamonds.splitstr(), ins_str);
                if action.protocol_cost.is_positive() {
                    desc += &format!("  cost {} HAC fee", cost_str);
                }
                description = desc;
            }

        }else if let Some(action) = actany.downcast_ref::<DiamondInscriptionClear>() {

            let dia_num = action.diamonds.length();
            let dia_names = action.diamonds.readable();
            let cost_str = action.protocol_cost.to_unit_string(unit);
            set_jsonobj!{
                "diamond", dia_num,
                "diamonds", dia_names,
                "protocol_cost", cost_str,
            };
            append_description!(
                "Clean inscript {} HACD ({}) cost {} HAC fee",
                dia_num, action.diamonds.splitstr(), cost_str
            );



        /*************** Channel ***************/

        }else if let Some(action) = actany.downcast_ref::<ChannelOpen>() {

            let cid = action.channel_id.hex();
            let l_adr = action.left_bill.address.readable();
            let l_amt = action.left_bill.amount.to_unit_string(unit);
            let r_adr = action.right_bill.address.readable();
            let r_amt = action.right_bill.amount.to_unit_string(unit);
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


        }else if let Some(action) = actany.downcast_ref::<ChannelClose>() {

            let cid = action.channel_id.hex();
            set_jsonobj!{
                "channel_id", cid,
            };
            append_description!(
                "Close channel {}", cid
            );


        /*************** Others ***************/

        }else if let Some(action) = actany.downcast_ref::<SubmitHeightLimit>() {
            
            let s_hei = *action.start;
            let e_hei = *action.end;
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

        }else if let Some(action) = actany.downcast_ref::<SubChainID>() {
            
            let cid = *action.chain_id;
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

    };


    /*************** Hacash 


    if kind == HacToTransfer::KIND {

        let action = HacToTransfer::must(&act.serialize());
        let to_addr = action.to.real(adrs).unwrap().readable();
        let amt_str = action.hacash.to_unit_string(unit);
        resjsonobj = jsondata!{
            "from", main_addr,
            "to", to_addr,
            "hacash", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HAC from {} to {}",
                amt_str, main_addr, to_addr
            )));
        }

    }else if kind == HacFromTransfer::KIND {

        let action = HacFromTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let amt_str = action.hacash.to_unit_string(unit);
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", main_addr,
            "hacash", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HAC from {} to {}",
                amt_str, from_addr, main_addr
            )));
        }

    }else if kind == HacFromToTransfer::KIND {

        let action = HacFromToTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let to_addr = action.to.real(adrs).unwrap().readable();
        let amt_str = action.hacash.to_unit_string(unit);
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", to_addr,
            "hacash", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HAC from {} to {}",
                amt_str, from_addr, to_addr
            )));
        }
    

    /*************** Hacash ***************/


    }else if kind == SatoshiToTransfer::KIND {

        let action = SatoshiToTransfer::must(&act.serialize());
        let to_addr = action.to.real(adrs).unwrap().readable();
        let amt_str = *action.satoshi;
        resjsonobj = jsondata!{
            "from", main_addr,
            "to", to_addr,
            "satoshi", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} SAT from {} to {}",
                amt_str, main_addr, to_addr
            )));
        }

    }else if kind == SatoshiFromTransfer::KIND {

        let action = SatoshiFromTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let amt_str = *action.satoshi;
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", main_addr,
            "satoshi", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} SAT from {} to {}",
                amt_str, from_addr, main_addr
            )));
        }

    }else if kind == SatoshiFromToTransfer::KIND {

        let action = SatoshiFromToTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let to_addr = action.to.real(adrs).unwrap().readable();
        let amt_str = *action.satoshi;
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", to_addr,
            "satoshi", amt_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} SAT from {} to {}",
                amt_str, from_addr, to_addr
            )));
        }
    

    /*************** Diamonds ***************/


    }else if kind == DiamondSingleTransfer::KIND {

        let action = DiamondSingleTransfer::must(&act.serialize());
        let to_addr = action.to.real(adrs).unwrap().readable();
        let dia_num = 1u32;
        let dia_names = action.diamond.to_readable();
        resjsonobj =  jsondata!{
            "from", main_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, dia_names, main_addr, to_addr
            )));
        }

    }else if kind == DiamondToTransfer::KIND {

        let action = DiamondToTransfer::must(&act.serialize());
        let to_addr = action.to.real(adrs).unwrap().readable();
        let dia_num = action.diamonds.length();
        let dia_names = action.diamonds.readable();
        resjsonobj =  jsondata!{
            "from", main_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), main_addr, to_addr
            )));
        }

    }else if kind == DiamondFromTransfer::KIND {
        
        let action = DiamondFromTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let dia_num = action.diamonds.length();
        let dia_names = action.diamonds.readable();
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", main_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), from_addr, main_addr
            )));
        }

    }else if kind == DiamondFromToTransfer::KIND {

        let action = DiamondFromToTransfer::must(&act.serialize());
        let from_addr = action.from.real(adrs).unwrap().readable();
        let to_addr = action.to.real(adrs).unwrap().readable();
        let dia_num = action.diamonds.length();
        let dia_names = action.diamonds.readable();
        resjsonobj = jsondata!{
            "from", from_addr,
            "to", to_addr,
            "diamond", dia_num,
            "diamonds", dia_names,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Transfer {} HACD ({}) from {} to {}",
                dia_num, action.diamonds.splitstr(), from_addr, to_addr
            )));
        }


    /*************** Diamond mint & inscription ***************/


    }else if kind == DiamondMint::KIND {

        let action = DiamondMint::must(&act.serialize());
        let act = action.d;
        let name = act.diamond.to_readable();
        let miner = act.address.readable();
        resjsonobj = jsondata!{
            "name", name,
            "number", *act.number,
            "miner", miner,
            "nonce", act.nonce.hex(),
            "prev_hash", act.prev_hash.hex(), // prev block hash
            "custom_message", act.custom_message.hex(),
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Mint HACD ({}) to {}",
                name, miner
            )));
        }

    }else if kind == DiamondInscription::KIND {

        let action = DiamondInscription::must(&act.serialize());
        let dia_num = action.diamonds.length();
        let dia_names = action.diamonds.readable();
        let cost_str = action.protocol_cost.to_unit_string(unit);
        let ins_str = action.engraved_content.to_readable_or_hex();
        resjsonobj = jsondata!{
            "diamond", dia_num,
            "diamonds", dia_names,
            "protocol_cost", cost_str,
            "engraved_type", *action.engraved_type,
            "engraved_content", ins_str,
        };
        if ret_desc {
            let mut desc = format!("Inscript {} HACD ({}) with \"{}\"",
                dia_num, action.diamonds.splitstr(), ins_str);
            if action.protocol_cost.is_positive() {
                desc += &format!("  cost {} HAC fee", cost_str);
            }
            resjsonobj.insert("description", json!(desc));
        }

    }else if kind == DiamondInscriptionClear::KIND {

        let action = DiamondInscriptionClear::must(&act.serialize());
        let dia_num = action.diamonds.length();
        let dia_names = action.diamonds.readable();
        let cost_str = action.protocol_cost.to_unit_string(unit);
        resjsonobj = jsondata!{
            "diamond", dia_num,
            "diamonds", dia_names,
            "protocol_cost", cost_str,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Clean inscript {} HACD ({}) cost {} HAC fee",
                dia_num, action.diamonds.splitstr(), cost_str
            )));
        }



    /*************** Channel ***************/

    }else if kind == ChannelOpen::KIND {

        let action = ChannelOpen::must(&act.serialize());
        let cid = action.channel_id.hex();
        let l_adr = action.left_bill.address.readable();
        let l_amt = action.left_bill.amount.to_unit_string(unit);
        let r_adr = action.right_bill.address.readable();
        let r_amt = action.right_bill.amount.to_unit_string(unit);
        resjsonobj = jsondata!{
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
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Open channel {} with left {}: {}, right {}: {}",
                cid, l_adr, l_amt, r_adr, r_amt
            )));
        }


    }else if kind == ChannelClose::KIND {

        let action = ChannelClose::must(&act.serialize());
        let cid = action.channel_id.hex();
        resjsonobj = jsondata!{
            "channel_id", cid,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Close channel {}",
                cid
            )));
        }


    /*************** Others ***************/

    }else if kind == SubmitHeightLimit::KIND {
        
        let action = SubmitHeightLimit::must(&act.serialize());
        let s_hei = *action.start;
        let e_hei = *action.end;
        resjsonobj = jsondata!{
            "start_height", s_hei,
            "end_height", e_hei,
        };
        if ret_desc {
            let e_hei = match e_hei == 0 { 
                true=>"Unlimited".to_owned(), false=>e_hei.to_string(),
            };
            resjsonobj.insert("description", json!(format!(
                "Limit height range ({}, {}) ",
                s_hei, e_hei
            )));
        }

    }else if kind == SubChainID::KIND {
        
        let action = SubChainID::must(&act.serialize());
        let cid = *action.chain_id;
        resjsonobj = jsondata!{
            "chain_id", cid,
        };
        if ret_desc {
            resjsonobj.insert("description", json!(format!(
                "Valid chain ID {}",
                cid
            )));
        }

    }else{

    }
    
    
    ***************/

    scan_by_any(act.as_any());

    // ok
    if ret_kind {
        resjsonobj.insert("kind", json!(kind));
    }
    return resjsonobj
}