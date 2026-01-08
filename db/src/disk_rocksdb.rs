

include!{"batch_rocksdb.rs"}


pub struct DiskKV {
    rdb: rocksdb::DB,
}


impl DiskKV {

    pub fn open(dir: &Path) -> Self {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        Self { rdb: rocksdb::DB::open(&opts, dir).unwrap() }
    }

}


impl DiskDB for DiskKV {

    fn remove(&self, k: &[u8]) {
        self.rdb.delete(k).unwrap();
        // self.rdb.flush().unwrap();
    }

    fn save(&self, k: &[u8], v: &[u8]) {
        self.rdb.put(k, v).unwrap();
        // self.rdb.flush().unwrap();
    }

    fn read(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.rdb.get(k).unwrap().map(|a|a.to_vec())
    }

    fn write(&self, memkv: &dyn MemDB) {
        let wb = Membatch::from_memkv(memkv);
        self.rdb.write(wb.into_batch().obj).unwrap(); // must
        // self.rdb.flush().unwrap();
    }

    /*
    fn write_batch(&self, batch: Box<dyn Any>) {
        let wb = batch.downcast::<Membatch>().unwrap().into_batch();
        self.ldb.apply_batch(wb.obj).unwrap(); // must
        // self.ldb.flush().unwrap();
    }
    */

    fn for_each(&self, each: &mut dyn FnMut(Vec<u8>, Vec<u8>)->bool) {
        let mut rdbiter = self.rdb.iterator(rocksdb::IteratorMode::Start);
        while let Some(Ok(it)) = rdbiter.next() {
            if !each(it.0.to_vec(), it.1.to_vec()) {
                break // end
            }
        }
    }

}



