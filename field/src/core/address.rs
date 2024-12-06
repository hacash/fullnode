use base58check::*;

const ADDR_OR_PTR_DIV_NUM: u8 = 10;

pub type Address = Fixed21;
pub type Addrptr = Uint1;

impl Address {
    
    pub const PRIVAKEY: u8 = 0;
    pub const MULTISIG: u8 = 1;
    pub const CONTRACT: u8 = 2;

    pub const UNKNOWN: Self = Fixed21::DEFAULT;

    pub fn must_privakey(&self) -> Rerr {
        match self.version() == Self::PRIVAKEY {
            true => Ok(()),
            false => errf!("address {} is not PRIVAKEY type", self.readable())
        }
    }
    
    pub fn is_contract(&self) -> bool {
        self.version() == Self::CONTRACT
    }

    pub fn must_contract(&self) -> Rerr {
        match self.version() == Self::CONTRACT {
            true => Ok(()),
            false => errf!("address {} is not CONTRACT type", self.readable())
        }
    }

    pub fn version(&self) -> u8 {
        self[0]
    }

    pub fn readable(&self) -> String {
        let btcon = self.serialize();
        let bts: [u8; Self::SIZE] = btcon.try_into().unwrap();
        Account::to_readable(&bts)
    }
    
    pub fn from_readable(addr: &str) -> Ret<Self> {
        let res = addr.from_base58check();
        if let Err(_) = res {
            return Err("base58check error".to_string())
        }
        let (version, body) = res.unwrap();
        if version > Self::CONTRACT { // > 3
            return Err("address version error".to_string())
        }
        if body.len() != Self::SIZE - 1 {
            return Err("address length error".to_string())
        }
        let mut address = Self::default();
        address[0] = version;
        for i in 1..Self::SIZE {
            address[i] = body[i-1];
        }
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
                let ix = v.uint() as usize;
                match ix < addrs.len() {
                    true => Ok(addrs[ix].clone()),
                    false => errf!("addr ptr index overflow")
                }
            },
        }
    }

    pub fn from_addr(v: Address) -> Self {
        Self::Val1(v)
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