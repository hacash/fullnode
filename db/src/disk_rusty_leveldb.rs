use std::sync::Mutex;

include!{"batch_rusty_leveldb.rs"} 



pub struct DiskKV {
    ldb: Mutex<rusty_leveldb::DB>,
}


impl DiskKV {

    pub fn open(dir: &Path) -> DiskKV {
        let mut opt = rusty_leveldb::Options::default();
        opt.create_if_missing = true;
        Self { ldb: Mutex::new(rusty_leveldb::DB::open(dir, opt).unwrap()) }
    }
}


impl DiskDB for DiskKV {

    #[inline(always)]
    fn remove(&self, k: &[u8]) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.delete(k).unwrap();
        ldb.flush().unwrap();
    }

    #[inline(always)]
    fn save(&self, k: &[u8], v: &[u8]) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.put(k, v).unwrap();
        ldb.flush().unwrap();
    }

    #[inline(always)]
    fn load(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.lock().unwrap().get(k)
    }

    #[inline(always)]
    fn save_batch(&self, batch: Writebatch) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.write(batch.deref(), true).unwrap(); // must
        ldb.flush().unwrap();
    }
}



