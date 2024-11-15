
pub trait DiskDB {
    fn open(_: &str) -> Self where Self: Sized { unimplemented!() }
    
    fn load(&self,   _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn save(&self,   _: &[u8], _: &[u8] ) -> Rerr { unimplemented!() }
    fn remove(&self, _: &[u8]) -> Rerr { unimplemented!() }    
}

// state change
pub trait StaChg {
}

pub trait State {
    fn build(_: Arc<dyn DiskDB>, _: Weak<dyn State>) -> Self where Self: Sized { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }

    fn get(&self,     _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn set(&mut self, _: &[u8], _: &[u8]) -> Rerr { unimplemented!() }
    fn del(&mut self, _: &[u8]) -> Rerr { unimplemented!() }
}








