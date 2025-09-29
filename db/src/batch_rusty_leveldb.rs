


use rusty_leveldb::WriteBatch;


pub struct Writebatch {
    obj: WriteBatch
}

impl Writebatch {

    pub fn new() -> Writebatch {
        Writebatch { obj: WriteBatch::default() }
    }
    
    pub fn len(&self) -> usize {
        self.obj.count() as usize
    }

    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        self.obj.put(k, v)
    }

    pub fn delete(&mut self, k: &[u8]) {
        self.obj.delete(k)
    }

    pub fn deref(self) -> WriteBatch {
        self.obj
    }
    
}



