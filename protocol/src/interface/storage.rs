
pub trait DiskDB {
    fn open() -> Self where Self: Sized { unimplemented!() }
    
    fn load(&self, _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn save(&self, _: &[u8], _: &[u8] ) -> Rerr { unimplemented!() }
    fn remove(&self, _: &[u8]) -> Rerr { unimplemented!() }    
}


pub trait State {
    fn build(_: Arc<dyn DiskDB>, _: Arc<dyn State>) -> Self where Self: Sized { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }

    fn get(&self, _: u16, _: &dyn Serialize) -> Option<Vec<u8>> { unimplemented!() }
    fn set(&self, _: u16, _: &dyn Serialize, _: &dyn Serialize) -> Rerr { unimplemented!() }
    fn del(&self, _: u16, _: &dyn Serialize) -> Rerr { unimplemented!() }
}








