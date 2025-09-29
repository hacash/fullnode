
// Hacash node
pub trait HNoder: Send + Sync {

    fn start(&self, _: Worker) {}

    fn submit_transaction(&self, _: &TxPkg, _is_async: bool) -> Rerr { never!() }
    fn submit_block(&self, _: &BlockPkg, _is_async: bool) -> Rerr { never!() }

    fn engine(&self) -> Arc<dyn Engine> { never!() }
    fn txpool(&self) -> Arc<dyn TxPool> { never!() }

    fn all_peer_prints(&self) -> Vec<String> { never!() }

    fn exit(&self) {}
    
}

