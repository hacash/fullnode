
use sled::Batch;


pub struct Writebatch {
    obj: Batch
}

impl Writebatch {

    pub fn new() -> Writebatch {
        Writebatch { obj: Batch ::default() }
    }

    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        self.obj.insert(k, v)
    }

    pub fn delete(&mut self, k: &[u8]) {
        self.obj.remove(k)
    }

    pub fn deref(self) -> Batch {
        self.obj
    }
    
}




