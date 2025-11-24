
include!{"batch_leveldb_sys.rs"}
include!{"leveldb-sys/mod.rs"}


/************************/


pub struct DiskKV {
    ldb: LevelDB,
}


impl DiskKV {

    pub fn open(dir: &Path) -> Self {
        Self { ldb: LevelDB::open(dir) }
    }
    
}


impl DiskDB for DiskKV {

    fn remove(&self, k: &[u8]) {
        self.ldb.rm(k)
    }

    fn save(&self, k: &[u8], v: &[u8]) {
        self.ldb.put(k, v)
    }

    fn read(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.get(k)
    }

    fn write(&self, memkv: &dyn MemDB) {
        let wb = Membatch::from_memkv(memkv);
        self.ldb.write(&wb.into_batch()); // must
    }

    /*
    fn write_batch(&self, batch: Box<dyn Any>) {
        let wb = batch.downcast::<Membatch>().unwrap().into_batch();
        self.ldb.write(&wb); // must
    }
    */

    
}



