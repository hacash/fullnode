

pub trait Scaner: Send + Sync {

    fn init(&mut self, _: &IniObj) -> Rerr { Ok(()) }
    fn exit(&self) {}

    fn start(&self, _: Worker) {} // handle loop
    fn serve(&self, _: Worker) {} // rpc server
    
    fn roll(&self, _: Arc<dyn Block>,  _: Arc<dyn State>, _: Arc<dyn DiskDB>) { }
}