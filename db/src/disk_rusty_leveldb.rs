use std::sync::Mutex;

include!{"batch_rusty_leveldb.rs"} 

use rusty_leveldb::LdbIterator;


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

    fn remove(&self, k: &[u8]) {
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

    fn write(&self, memkv: &dyn MemDB) {
        let wb = Membatch::from_memkv(memkv);
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.write(wb.into_batch().obj, true).unwrap(); // must
        ldb.flush().unwrap();
    }

    /*
    fn write_batch(&self, batch: Box<dyn Any>) {
        let wb = batch.downcast::<Membatch>().unwrap().into_batch();
        let mut ldb =  self.ldb.lock().unwrap();
        ldb.write(wb.obj, true).unwrap(); // must
        ldb.flush().unwrap();
    }
    */

    fn for_each(&self, each: &mut dyn FnMut(Vec<u8>, Vec<u8>)->bool) {
        let mut ldb =  self.ldb.lock().unwrap();
        let mut ldbiter = ldb.new_iter().unwrap();
        while let Some((k, v)) = ldbiter.next() {
            if !each(k, v) {
                break // end
            }
        }
    }


}



