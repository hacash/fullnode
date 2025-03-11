

include!{"batch_sled.rs"}


pub struct DiskKV {
    ldb: sled::Db,
}


impl DiskKV {

    pub fn open(dir: &Path) -> Self {
        Self { ldb: sled::open(dir).unwrap() }
    }

}


impl DiskDB for DiskKV {

    fn drop(&self, k: &[u8]) {
        self.ldb.remove(k).unwrap();
        self.ldb.flush().unwrap();
    }

    fn save(&self, k: &[u8], v: &[u8]) {
        self.ldb.insert(k, v).unwrap();
        self.ldb.flush().unwrap();
    }

    fn read(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.get(k).unwrap().map(|a|a.to_vec())
    }

    fn write(&self, memkv: &MemKV) {
        self.ldb.apply_batch(memkv.to_writebatch().deref()).unwrap(); // must
        self.ldb.flush().unwrap();
    }

    fn write_batch(&self, batch: MemBatch) {
        self.ldb.apply_batch(batch.into_writebatch().deref()).unwrap(); // must
        self.ldb.flush().unwrap();
    }

    fn for_each(&self, each: &mut dyn FnMut(Vec<u8>, Vec<u8>)->bool) {
        let mut ldbiter = self.ldb.iter();
        while let Some(Ok(it)) = ldbiter.next() {
            if !each(it.0.to_vec(), it.1.to_vec()) {
                break // end
            }
        }
    }

}



