

#[macro_export]
macro_rules! ctx_state{
    ($ctx:expr, $state:ident) => (
        let _s1_db = $ctx.engine.state();
        let _s1_db = _s1_db.as_ref();
        let $state = CoreStateRead::wrap(_s1_db);
    )
}

#[macro_export]
macro_rules! ctx_store{
    ($ctx:expr, $sto:ident) => (
        let $sto = $ctx.engine.store();
    )
}

#[macro_export]
macro_rules! ctx_mintcnf{
    ($ctx:expr) => (
        $ctx.engine.mint_checker().config().downcast::<MintConf>().unwrap()
    )
}

#[derive(Clone, Debug)]
pub struct CoinKind {
    pub hacash: bool,
    pub satoshi: bool,
    pub diamond: bool,
}
impl CoinKind {
    pub fn new(s: String) -> CoinKind {
        match s.to_lowercase().as_str() {
            "all" | "hsd" => CoinKind {
                hacash: true,
                satoshi: true,
                diamond: true,
            },
            _ => CoinKind {
                hacash: s.contains("h"),
                satoshi: s.contains("s"),
                diamond: s.contains("d"),
            }
        }
    }
}

#[macro_export]
macro_rules! q_coinkind {
    ( $q: ident, $k: ident ) => (
        q_must!($q, $k, s!("hsd"));
        let $k = CoinKind::new( $k );
    )
}

#[macro_export]
macro_rules! q_unit {
    ( $q: ident, $k: ident ) => (
        q_must!($q, $k, s!("fin"));
    )
}

#[macro_export]
macro_rules! q_must {
    ( $q: ident, $k: ident, $dv: expr ) => (
        #[allow(unused_mut)]
        let mut $k = match $q.$k.clone() 
        {
            Some(v) => v,
            _ => $dv,
        };
    )
}

#[macro_export]
macro_rules! q_body_data_may_hex {
    ( $q: ident, $d: expr) => (
        { 
            q_must!($q, hexbody, false);
            let bddt = $d.to_vec();
            match hexbody {
                false => bddt,
                true => {
                    let res = hex::decode(&bddt);
                    if let Err(_) = res {
                        return api_error("hex format error")
                    }
                    res.unwrap()
                }
            }
        }
    )
}

#[macro_export]
macro_rules! q_addr {
    ($adr: expr) => ({
        let adr = Address::from_readable(&$adr);
        if let Err(e) = adr {
            return api_error(&format!("address {} format error: {}", &$adr, &e))
        }
        adr.unwrap()
    })
}

#[macro_export]
macro_rules! q_data_addr {
    ( $q: ident, $adr: ident) => (
        q_addr!(&$q.$adr)
    )
}

#[macro_export]
macro_rules! q_amt {
    ( $amt: expr) => ({
        let amt = Amount::from(&$amt);
        if let Err(e) = amt {
            return api_error(&format!("amount {} format error: {}", &$amt, &e))
        }
        amt.unwrap()
    })
}

#[macro_export]
macro_rules! q_data_amt {
    ( $q: ident, $amt: ident) => (
        q_amt!($q.$amt)
    )
}

#[macro_export]
macro_rules! q_data_acc_from {
    ( $acc: expr) => ({
        let acc = Account::create_by(&$acc);
        if let Err(e) = acc {
            return api_error(&format!("prikey error: {}", &e))
        }
        acc.unwrap()
    })
}

#[macro_export]
macro_rules! q_data_acc {
    ( $q: ident, $acc: ident) => (
        q_data_acc_from!($q.$acc)
    )
}

#[macro_export]
macro_rules! q_data_hash {
    ( $hxstr: ident) => ({
        let hx = hex::decode($hxstr);
        if let Err(e) = hx {
            return api_error(&format!("hash parse error: {}", &e))
        }
        let hx = hx.unwrap();
        if hx.len() != Hash::SIZE {
            return api_error(&format!("hash size error"))
        }
        Hash::from(hx.try_into().unwrap())
    })
}




#[macro_export]
macro_rules! api_querys_define {
    ( $name: ident, $( $item: ident, $ty: ty, $dv: expr,)+ ) => (

        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct $name {
            $(
                $item: $ty,
            )+
            unit: Option<String>,
            coinkind: Option<String>,
            hexbody: Option<bool>,
            base64body: Option<bool>,
            hex: Option<bool>,
            base64: Option<bool>,
            extendpath: Option<String>,
        }

        impl Default for $name {
            fn default() -> Self {
                Self { 
                    $(
                        $item: $dv,
                    )+
                    unit: None,
                    coinkind: None,
                    hexbody: None,
                    base64body: None,
                    hex: None,
                    base64: None,
                    extendpath: None,
                }
            }
        }

    )
}



/*******************************/
