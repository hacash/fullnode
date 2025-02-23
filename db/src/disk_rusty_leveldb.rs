use std::sync::Mutex;

include!{"batch_rusty_leveldb.rs"} 



pub struct DiskKV {
    ldb: Mutex<rusty_leveldb::DB>,
}


impl DiskKV {

    pub fn open(dir: &Path) -> Self {
        let mut opt = rusty_leveldb::Options::default();
        opt.create_if_missing = true;
        Self { ldb: Mutex::new(rusty_leveldb::DB::open(dir, opt).unwrap()) }
    }
    
}


impl DiskDB for DiskKV {

    fn drop(&self, k: &[u8]) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.delete(k).unwrap();
        ldb.flush().unwrap();
    }

    fn save(&self, k: &[u8], v: &[u8]) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.put(k, v).unwrap();
        ldb.flush().unwrap();
    }

    fn read(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.lock().unwrap().get(k)
    }

    fn write(&self, memkv: &MemKV) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.write(memkv.to_writebatch().deref(), true).unwrap(); // must
        ldb.flush().unwrap();
    }

    fn write_batch(&self, batch: MemBatch) {
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.write(batch.into_writebatch().deref(), true).unwrap(); // must
        ldb.flush().unwrap();
    }


}



