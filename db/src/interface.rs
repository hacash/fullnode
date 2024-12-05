
pub trait DiskDB : Send + Sync {
    // fn open(_: &Path) -> Self where Self: Sized { unimplemented!() }
    
    fn load(&self,   _: &[u8]) -> Option<Vec<u8>> { unimplemented!() }
    fn save(&self,   _: &[u8], _: &[u8] ) { unimplemented!() }
    fn remove(&self, _: &[u8]) { unimplemented!() }  
    fn save_batch(&self, _: Writebatch) { unimplemented!() }
}
