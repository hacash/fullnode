

pub trait MemDB : Send + Sync {
    fn new() -> Self where Self: Sized { never!() }
    fn del(&mut self, _: Vec<u8>) {}
    fn put(&mut self, _: Vec<u8>, _: Vec<u8>) {}
    fn get(&self, _: &Vec<u8>) -> Option<Option<Vec<u8>>> { None }
    fn for_each(&self, _:&mut dyn FnMut(&Vec<u8>, &Option<Vec<u8>>)) {}
}


pub trait MemBatch {
    fn new() -> Self where Self: Sized { never!() }
    fn from_memkv(_: &dyn MemDB) -> Self where Self: Sized { never!() }
    fn del(&mut self, _: &[u8]) {}
    fn put(&mut self, _: &[u8], _: &[u8]) {}
}


pub trait DiskDB : Send + Sync {
    // fn open(dir: &Path) -> Self where Self: Sized;
    fn read(&self, _: &[u8]) -> Option<Vec<u8>> { None }
    fn save(&self, _: &[u8], _: &[u8] ) {}
    fn drop(&self, _: &[u8]) {}
    fn write(&self, _: &dyn MemDB) {} // dyn MemDB
    // fn write_batch(&self, _: Box<dyn Any>) {} // dyn MemBatch
    // debug
    fn for_each(&self, _: &mut dyn FnMut(Vec<u8>, Vec<u8>)->bool) {}
}



