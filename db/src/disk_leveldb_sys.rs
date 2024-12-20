
include!{"batch_leveldb_sys.rs"}
include!{"leveldb/mod.rs"}


/************************/


pub struct DiskKV {
    ldb: LevelDB,
}


impl DiskKV {

    pub fn open(dir: &Path) -> DiskKV {
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

    
    fn load(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ldb.get(k)
    }

    
    fn save_batch(&self, batch: Writebatch) {
        self.ldb.write(&batch); // must
    }
}



