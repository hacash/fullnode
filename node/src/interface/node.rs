
// Hacash node
pub trait HNode: Send + Sync {

    fn submit_transaction(&self, _: &Box<TxPkg>, is_async: bool) -> Rerr { unimplemented!() }
    fn submit_block(&self, _: &Box<BlockPkg>, is_async: bool) -> Rerr { unimplemented!() }

    fn engine(&self) -> Arc<dyn Engine> { unimplemented!() }
    fn txpool(&self) -> Arc<dyn TxPool> { unimplemented!() }

    fn all_peer_prints(&self) -> Vec<String> { unimplemented!() }
}

