
// CoinbaseExtendDataV1
combi_struct!{ CoinbaseExtendDataV1, 
	miner_nonce   : Hash
	witness_count : Uint1 // Number of voting witnesses
}

// CoinbaseExtend
combi_option!{ CoinbaseExtend, CoinbaseExtendDataV1 }


// coinbase
combi_struct!{ TransactionCoinbase,
    ty      : Uint1
    address : Address
    reward  : Amount
    message : Fixed16
    extend  : CoinbaseExtend
}


impl TransactionRead for TransactionCoinbase {

    /*
    fn hash(&self) -> Hash { 
        let stuff = self.serialize();
        let hx = x16rs::calculate_hash(stuff);
        Hash::must(&hx[..])
    }
    
    fn hash_with_fee(&self) -> Hash {
        self.hash()
    }
    */

    fn ty(&self) -> u8 {
        self.ty.to_uint()
    }

    fn main(&self) -> Address {
        self.address.clone()
    }

    /*
    fn reward(&self) -> &Amount {
        &self.reward
    }

    fn message(&self) -> &StringTrim16 {
        &self.message
    }
    */

    // call ret error
    
}

impl Transaction for TransactionCoinbase {
    fn as_read(&self) -> &dyn TransactionRead {
        self
    }

    /*
    fn set_nonce(&mut self, nonce: Hash) { 
        match &mut self.extend.datas_v1 {
            Some(ref mut d) => d.miner_nonce = nonce,
            _ => (), // do nothing
        };
    }
    */
}

impl TxExec for TransactionCoinbase {
    
    fn execute(&self, _: &mut dyn Context) -> Rerr {
        unimplemented!()
    }

    /*
    fn execute(&self, _: u64, sta: &mut dyn State) -> RetErr {
        let mut state = CoreState::wrap(sta);
        let rwdadr = self.address()?;
        let amt = self.reward();
        operate::hac_add(&mut state, &rwdadr, amt)?;
        Ok(())
    }
    */
}


impl TransactionCoinbase {
    pub const TYPE: u8 = 0; // 0
}