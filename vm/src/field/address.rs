use std::ops::*;
use std::hash::Hasher;

pub const CONTRACT_ADDRESS_WIDTH: usize = 21;

combi_struct!{ ContractAddress,
    addr: Address
}

impl std::hash::Hash for ContractAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
    }
}


impl Deref for ContractAddress {
    type Target = Address;
    fn deref(&self) -> &Address {
        &self.addr
    }
}


impl ContractAddress {

    // https://en.bitcoin.it/wiki/List_of_address_prefixes
	pub fn calculate(addr: &Address, nonce: &Uint4) -> Self {
		let dts = vec![addr.serialize(), nonce.serialize()].concat();
		let hx32 = sha3(dts);
		let hx20 = ripemd160(hx32);
        let addr = Address::create_contract(hx20);
		ContractAddress::from_addr(addr).unwrap()
	}

    pub fn must(bts: [u8; CONTRACT_ADDRESS_WIDTH]) -> Self {
        Self::from_addr(Address::from(bts)).unwrap()
    }

    pub fn new(addr: Address) -> Self {
        Self{addr}
    }

    pub fn check(&self) -> Rerr {
        let av = self.addr.version();
        if av !=  Address::CONTRACT {
            return errf!("address version {} is not CONTRACT", av)
        }
        Ok(())
    }

    pub fn from_addr(addr: Address) -> Ret<Self> {
        let av = addr.version();
        if av !=  Address::CONTRACT {
            return errf!("address version {} is not CONTRACT", av)
        }
        Ok(Self{addr})
    }

    pub fn to_addr(&self) -> Address {
        self.addr
    }

    pub fn into_addr(self) -> Address {
        self.addr
    }

    pub fn parse(dts: &[u8]) -> Ret<Self> {
        if dts.len() != Address::SIZE {
            return errf!("contract address length error")
        }
        let cadr = Address::from(dts.try_into().unwrap());
        ContractAddress::from_addr(cadr)
    } 

    pub fn readable(&self) -> String {
        self.addr.readable()
    }

    

}


combi_list!{ContractAddressW1,
    Uint1, ContractAddress
}

