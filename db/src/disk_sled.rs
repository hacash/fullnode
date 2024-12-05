

include!{"batch_sled.rs"}


pub struct DiskKV {
    ldb: sled::Db,
}


impl DiskKV {

    pub fn open(dir: &Path) -> DiskKV {
        Self { ldb: sled::open(dir).unwrap() }
    }
}


impl DiskDB for DiskKV {

    #[inline(always)]
    fn remove(&self, k: &[u8]) {
        self.ldb.remove(k).unwrap();
        self.ldb.flush().unwrap();
    }

    #[inline(always)]
    fn save(&self, k: &[u8], v: &[u8]) {
        self.ldb.insert(k, v).unwrap();
        self.ldb.flush().unwrap();
    }

    #[inline(always)]
    fn load(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.get(k).unwrap().map(|a|a.to_vec())
    }

    #[inline(always)]
    fn save_batch(&self, batch: Writebatch) {
        self.ldb.apply_batch(batch.deref()).unwrap(); // must
        self.ldb.flush().unwrap();
    }
}



