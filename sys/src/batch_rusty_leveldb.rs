


use rusty_leveldb::WriteBatch;


pub struct Writebatch {
    obj: WriteBatch
}

impl Writebatch {

    /// Create a new writebatch
    pub fn new() -> Writebatch {
        Writebatch { obj: WriteBatch::default() }
    }
    
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.obj.count() as usize
    }

    // Batch a put operation
    
    #[inline(always)]
    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        self.obj.put(k, v)
    }

    // Batch a delete operation
    
    #[inline(always)]
    pub fn delete(&mut self, k: &[u8]) {
        self.obj.delete(k)
    }

    #[inline(always)]
    pub fn deref(self) -> WriteBatch {
        self.obj
    }
}



