
pub trait BlockExec {
    fn execute(&self) -> Rerr { unimplemented!() }
}


pub trait BlockRead : BlockExec + Serialize + Send + Sync + DynClone {

    fn hash(&self) -> Hash { unimplemented!() }

    fn version(&self) -> &Uint1 { unimplemented!() }
    fn height(&self) -> &BlockHeight { unimplemented!() }
    fn timestamp(&self) -> &Timestamp { unimplemented!() }
    fn nonce(&self) -> &Uint4 { unimplemented!() }
    fn difficulty(&self) -> &Uint4 { unimplemented!() } 
    fn prevhash(&self) -> &Hash { unimplemented!() }
    fn mrklroot(&self) -> &Hash { unimplemented!() }
    
    fn transaction_count(&self) -> &Uint4 { unimplemented!() }
    fn transactions(&self) -> &Vec<Box<dyn Transaction>> { unimplemented!() }
    fn transaction_hash_list(&self, _hash_with_fee: bool) -> Vec<Hash> { unimplemented!() }

}

pub trait Block : BlockRead + Field + Send + Sync + DynClone {

    fn as_read(&self) -> &dyn BlockRead { unimplemented!() }

    fn update_mrklroot(&mut self) { unimplemented!() }
    fn set_nonce(&mut self, _: Uint4) { unimplemented!() }
    fn set_mrklroot(&mut self, _: Hash) { unimplemented!() }
    
    fn replace_transaction(&mut self, _: usize, _: Box<dyn Transaction>) -> Rerr { unimplemented!() }
    fn push_transaction(&mut self, _: Box<dyn Transaction>) -> Rerr { unimplemented!() }

}


clone_trait_object!(BlockRead);
clone_trait_object!(Block);


