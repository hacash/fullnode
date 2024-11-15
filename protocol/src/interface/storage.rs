
pub trait DiskDB {
    // fn open(_: &Path) -> Self where Self: Sized { unimplemented!() }
    
    fn load(&self,   _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn save(&self,   _: &[u8], _: &[u8] ) { unimplemented!() }
    fn remove(&self, _: &[u8]) { unimplemented!() }    
}


pub trait State {
    // fn build(_: Arc<dyn DiskDB>, _: Weak<dyn State>) -> Self where Self: Sized { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }
    fn write_to_disk(&self) { unimplemented!() }

    fn get(&self,     _: Vec<u8>) -> Option<Vec<u8>> { unimplemented!() }
    fn set(&mut self, _: Vec<u8>, _: Vec<u8>) { unimplemented!() }
    fn del(&mut self, _: Vec<u8>) { unimplemented!() }
}








