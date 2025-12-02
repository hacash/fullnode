
pub trait BlockExec {
    fn execute(&self, _: ChainInfo, _: Box<dyn State>, _: Box<dyn Logs>) -> Ret<(Box<dyn State>, Box<dyn Logs>)> { never!() }
}

pub trait BlockRead : BlockExec + Serialize + Send + Sync + DynClone {

    fn hash(&self) -> Hash { never!() }

    fn version(&self) -> &Uint1 { never!() }
    fn height(&self) -> &BlockHeight { never!() }
    fn timestamp(&self) -> &Timestamp { never!() }
    fn nonce(&self) -> &Uint4 { never!() }
    fn difficulty(&self) -> &Uint4 { never!() } 
    fn prevhash(&self) -> &Hash { never!() }
    fn mrklroot(&self) -> &Hash { never!() }
    fn coinbase_transaction(&self) ->  Ret<&dyn TransactionRead> { never!() } // must have
    fn transaction_count(&self) -> &Uint4 { never!() }
    fn transactions(&self) -> &Vec<Box<dyn Transaction>> { never!() }
    fn transaction_hash_list(&self, _hash_with_fee: bool) -> Vec<Hash> { never!() }

}

pub trait Block : BlockRead + Field + Send + Sync + DynClone {

    fn as_read(&self) -> &dyn BlockRead { never!() }

    fn update_mrklroot(&mut self) { never!() }
    fn set_nonce(&mut self, _: Uint4) { never!() }
    fn set_mrklroot(&mut self, _: Hash) { never!() }
    
    fn replace_transaction(&mut self, _: usize, _: Box<dyn Transaction>) -> Rerr { never!() }
    fn push_transaction(&mut self, _: Box<dyn Transaction>) -> Rerr { never!() }

}


clone_trait_object!(BlockRead);
clone_trait_object!(Block);


