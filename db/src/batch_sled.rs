
use sled::Batch;


pub struct Writebatch {
    obj: Batch
}

impl Writebatch {

    /// Create a new writebatch
    pub fn new() -> Writebatch {
        Writebatch { obj: Batch ::default() }
    }

    // Batch a put operation
    
    #[inline(always)]
    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        self.obj.insert(k, v)
    }

    // Batch a delete operation
    
    #[inline(always)]
    pub fn delete(&mut self, k: &[u8]) {
        self.obj.remove(k)
    }

    #[inline(always)]
    pub fn deref(self) -> Batch {
        self.obj
    }
}




