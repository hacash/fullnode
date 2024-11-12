use base58check::*;

pub type Address = Fixed21;


impl Address {
    
    pub const PRIVAKEY: u8 = 0;
    pub const MULTISIG: u8 = 1;
    pub const CONTRACT: u8 = 2;

    pub const UNKNOWN: Self = Fixed21::DEFAULT;

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