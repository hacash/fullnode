use base58check::*;

pub const ADDR_OR_PTR_DIV_NUM: u8 = 20;

pub type Address = Fixed21;
pub type Addrptr = Uint1;
 

pub static ADDRESS_ZERO: Address = Fixed21::from([0u8; Address::SIZE]);
pub static ADDRESS_ONEX: Address = Fixed21::from([0u8, 1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]);
pub static ADDRESS_TWOX: Address = Fixed21::from([0u8, 2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2]);


macro_rules! address_version_define {
    ( $($key:ident : $name:ident , $num:expr)+ ) => {

impl Address {
    
    $(
    pub const $key: u8 = $num; // leading symbol: 1
    )+

    pub fn version(&self) -> u8 {
        self[0]
    }

    pub fn check_version(&self) -> Rerr {
        let v = self.version();
        match v {
            $( $num )|+ => Ok(()),
            _ => errf!("address version {} not support", v)
        }
    }

    $(
    concat_idents::concat_idents!{ is_version = is_, $name {
    pub fn is_version(&self) -> bool {
        self.version() == Self::$key
    }
    }}
    concat_idents::concat_idents!{ must_version = must_, $name {
    pub fn must_version(&self) -> Rerr {
        match self.version() == Self::$key {
            true => Ok(()),
            false => errf!("address {} is not {} type", self.readable(), stringify!($key))
        }
    }
    }}
    )+

    pub fn from_bytes(stuff: &[u8]) -> Ret<Self> {
        if stuff.len() != Self::SIZE {
            return errf!("address size not match")
        }
        let addr = Self::from(stuff.to_vec().try_into().unwrap());
        addr.check_version()?;
        Ok(addr)
    }

}   

    }
}

/*
    version
    https://en.bitcoin.it/wiki/List_of_address_prefixes
    scriptmh: Pay-to-Script-Merkl-Hash
*/
#[cfg(feature = "hvm")]
address_version_define!{
    PRIVAKEY : privakey, 0 // leading symbol: 1
    CONTRACT : contract, 1 // leading symbol: Q-Z, a-k, m-o
    SCRIPTMH : scriptmh, 5 // leading symbol: 3
}


#[cfg(not(feature = "hvm"))]
address_version_define!{
    PRIVAKEY : privakey, 0 // leading symbol: 1
    SCRIPTMH : scriptmh, 5 // leading symbol: 3
}


/*
    readable
*/
impl Address {
    
    pub const UNKNOWN: Self = Fixed21::DEFAULT;

    pub fn readable(&self) -> String {
        Account::to_readable(&*self)
    }

    pub fn prefix(&self, n: usize) -> String {
        let mut s = self.readable();
        let _ = s.split_off(n);
        s
    }
    
    pub fn from_readable(addr: &str) -> Ret<Self> {
        let Ok((version, body)) = addr.from_base58check() else {
            return errf!("base58check error")
        };
        if body.len() != Self::SIZE - 1 {
            return Err("address length error".to_string())
        }
        let mut address = Self::default();
        address[0] = version;
        for i in 1..Self::SIZE {
            address[i] = body[i-1];
        }
        address.check_version()?;
        Ok(address)
    }
    
}





/*
*
*/
combi_list!{ AddressListW1, Uint1, Address }


/*
*
*/
combi_revenum!{ AddrOrList, Address, AddressListW1, ADDR_OR_PTR_DIV_NUM }

impl AddrOrList {

    #[allow(dead_code)]
    pub fn list(&self) -> Vec<Address> {
        match self {
            Self::Val1(v) => vec![*v],
            Self::Val2(v) => v.list().clone(),
        }
    }

    pub fn from_list(list: Vec<Address>) -> Ret<Self> {
        let mut v = AddressListW1::new();
        v.append(list)?;
        Ok(Self::Val2(v))
    }

    pub fn from_addr(v: Address) -> Self {
        Self::Val1(v)
    } 

}


/*
*
*/
combi_revenum!{ AddrOrPtr, Address, Addrptr, ADDR_OR_PTR_DIV_NUM }

impl Copy for AddrOrPtr {} 

impl From<Address> for AddrOrPtr {
    fn from(addr: Address) -> Self {
        Self::Val1(addr)
    }
}


impl AddrOrPtr {

    /**
    * real address by ptr in list 
    */
    #[allow(dead_code)]
    pub fn real(&self, addrs: &Vec<Address>) -> Ret<Address> {
        match self {
            Self::Val1(v) => Ok(*v),
            Self::Val2(v) => {
                let ix = v.uint();
                if ix < ADDR_OR_PTR_DIV_NUM {
                    return errf!("addr ptr index error")
                }
                let i = (ix - ADDR_OR_PTR_DIV_NUM) as usize;
                match i < addrs.len() {
                    true => Ok(addrs[i].clone()),
                    false => errf!("addr ptr index overflow")
                }
            },
        }
    }

    pub fn from_addr(v: Address) -> Self {
        Self::Val1(v)
    } 

    pub fn from_ptr(i: u8) -> Self {
        Self::Val2(Addrptr::from(i + ADDR_OR_PTR_DIV_NUM))
    } 

}





/************************ test ************************/





#[cfg(test)]
mod address_tests {
    use super::*;

    #[test]
    fn test1() {

        let adr0 = "1111111111111111111114oLvT2";
        let adr1 = Address::UNKNOWN;
        let adr2 = Address::from_readable(adr0).unwrap();
        
        assert_eq!(adr1.readable(), adr2.readable());

        let adra = "14Xrfwd7XWmvzjpinTxxc9PwdHf37Myryy";
        let privkey = "594ac10e33501c06e3fae0f9133f4701c204a1f9de62a97cc33754a051019db7";

        let adrb = Account::create_by(privkey).unwrap();
        assert_eq!(adra, adrb.readable());

        let adrc = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
        assert_eq!(adrc, Account::create_by("123456").unwrap().readable());

    }

}