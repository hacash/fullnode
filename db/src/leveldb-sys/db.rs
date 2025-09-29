

struct RawDB {
    ptr: *mut leveldb_t,
}

impl Drop for RawDB {
    fn drop(&mut self) {
        unsafe {
            leveldb_close(self.ptr);
        }
    }
}

unsafe impl Send for RawDB {}
unsafe impl Sync for RawDB {}


pub struct LevelDB {
    database: RawDB,
    // ldb: LevelDatabase<LDBKey>,
}


impl LevelDB {

    pub fn open(dir: &Path) -> LevelDB {
        // let mut opts = Options::new();
        // opts.create_if_missing = true;
        // let ldb = LevelDatabase::open(dir, opts).unwrap();
        let mut error = ptr::null_mut();
        let database = unsafe {
            let c_options = leveldb_options_create();
            leveldb_options_set_create_if_missing(c_options, 1u8);
            let c_dbpath = CString::new(dir.to_str().unwrap()).unwrap();
            let db = leveldb_open(c_options as *const leveldb_options_t,
                c_dbpath.as_bytes_with_nul().as_ptr() as *const c_char,
                                  &mut error);
            leveldb_options_destroy(c_options);
            db
        };
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
        // create
        LevelDB{
            database: RawDB { ptr: database },
        }
    }

    // get if find, bool is not check base
    
    pub fn get_at(&self, k: &[u8]) -> Option<RawBytes> {
        let mut error = ptr::null_mut();
        let mut length: size_t = 0;
        let result = unsafe {
            let c_readoptions = leveldb_readoptions_create();
            let res = leveldb_get(self.database.ptr,
                c_readoptions,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                &mut length,
                &mut error);
            leveldb_readoptions_destroy(c_readoptions);
            res
        };
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
        if result.is_null() {
            return None // not find
        }
        Some(unsafe {
            RawBytes::from_raw_unchecked(result as *mut u8, length)
        })
    }
    
    
    pub fn get(&self, k: &[u8]) -> Option<Vec<u8>> {
        if let Some(v) = self.get_at(k) {
            return Some(v.into())
        }
        None
    }

    // set
    
    pub fn put(&self, k: &[u8], value: &[u8]) {
        let mut error = ptr::null_mut();
        unsafe {
            let c_writeoptions = leveldb_writeoptions_create();
            leveldb_put(self.database.ptr,
                c_writeoptions,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                value.as_ptr() as *mut c_char,
                value.len() as size_t,
                &mut error);
            leveldb_writeoptions_destroy(c_writeoptions);
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
    }

    // del
    
    pub fn rm(&self, k: &[u8]) {
        let mut error = ptr::null_mut();
        unsafe {
            let c_writeoptions = leveldb_writeoptions_create();
            leveldb_delete(self.database.ptr,
                c_writeoptions,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                &mut error);
            leveldb_writeoptions_destroy(c_writeoptions);
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
    }
    
    // write batch
    
    pub fn write(&self, batch: &Writebatch) {
        let mut error = ptr::null_mut();
        unsafe {
            let c_writeoptions = leveldb_writeoptions_create();
            leveldb_write(self.database.ptr,
                          c_writeoptions,
                          batch.ptr,
                          &mut error);
            leveldb_writeoptions_destroy(c_writeoptions);
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
    }


}
