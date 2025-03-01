

pub trait Scaner: Send + Sync {

    fn init(&mut self, _: &IniObj) -> Rerr { Ok(()) }
    fn exit(&self) {}

    fn start(&self) -> Rerr { Ok(()) } // handle loop
    fn serve(&self) -> Rerr { Ok(()) } // rpc server
    
    fn roll(&self, _: Arc<dyn Block>,  _: Arc<dyn State>, _: Arc<DiskKV>) { }
}