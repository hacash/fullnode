
pub trait DiskDB {
    fn open() -> Self where Self: Sized { unimplemented!() }
    
    fn load(&self, _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn save(&self, _: &[u8], _: &[u8] ) -> Rerr { unimplemented!() }
    fn remove(&self, _: &[u8]) -> Rerr { unimplemented!() }    
}


pub trait State {
    fn build() -> Self where Self: Sized {
        
    }
}








