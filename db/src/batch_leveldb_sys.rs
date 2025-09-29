

pub struct Writebatch {
    length: usize,
    pub ptr: *mut leveldb_writebatch_t,
}

impl Drop for Writebatch {
    fn drop(&mut self) {
        unsafe {
            leveldb_writebatch_destroy(self.ptr);
        }
    }
}


impl Writebatch {
    
    pub fn new() -> Writebatch {
        let ptr = unsafe { leveldb_writebatch_create() };
        Writebatch { ptr, length: 0 }
    }
    
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn put(&mut self, k: &[u8], value: &[u8]) {
        self.length += 1;
        unsafe {
            leveldb_writebatch_put(self.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                value.as_ptr() as *mut c_char,
                value.len() as size_t);
        }
    }

    pub fn delete(&mut self, k: &[u8]) {
        unsafe {
            leveldb_writebatch_delete(self.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t);
        }
    }

    pub fn deref(self) -> Self {
        self
    }

}



