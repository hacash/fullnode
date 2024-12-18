
// CoinbaseExtendDataV1
combi_struct!{ CoinbaseExtendDataV1, 
	miner_nonce   : Hash
	witness_count : Uint1 // Number of voting witnesses
}

// CoinbaseExtend
combi_optional!{ CoinbaseExtend, 
    extend: CoinbaseExtendDataV1 
}


// coinbase
combi_struct!{ TransactionCoinbase,
    ty      : Uint1
    address : Address
    reward  : Amount
    message : Fixed16
    extend  : CoinbaseExtend
}



impl TransactionRead for TransactionCoinbase {

    fn hash(&self) -> Hash { 
        let stuff = self.serialize();
        let hx = x16rs::calculate_hash(stuff);
        Hash::must(&hx[..])
    }
    
    fn hash_with_fee(&self) -> Hash {
        self.hash()
    }

    fn fee_pay(&self) -> Amount {
        Amount::zero()
    }

    fn fee_got(&self) -> Amount {
        Amount::zero()
    }

    fn fee_extend(&self) -> Ret<(u16, Amount)> {
        errf!("cannot get fee extend on coinbase tx")
    }

    fn ty(&self) -> u8 {
        *self.ty
    }

    fn main(&self) -> Address {
        self.address.clone()
    }

    fn addrs(&self) -> Vec<Address> {
        vec![self.main()]
    }

    fn reward(&self) -> &Amount {
        &self.reward
    }

    fn message(&self) -> &Fixed16 {
        &self.message
    }

    // call ret error
    fn verify_signature(&self) -> Rerr {
        errf!("cannot verify signature on coinbase tx")
    }
    
}

impl Transaction for TransactionCoinbase {
    fn as_read(&self) -> &dyn TransactionRead {
        self
    }

    fn set_nonce(&mut self, nonce: Hash) { 
        match &mut self.extend.extend {
            Some(ref mut d) => d.miner_nonce = nonce,
            _ => (), // do nothing
        };
    }
}





impl TxExec for TransactionCoinbase {
    
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        let addr = self.main();
        let amt = self.reward();
        operate::hac_add(ctx, &addr, amt)?;
        Ok(())
    }

}


impl TransactionCoinbase {
    pub const TYPE: u8 = 0; // 0
}