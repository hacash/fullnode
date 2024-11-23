
pub struct DiskKV {
    ldb: leveldb::LevelDB,
}


impl DiskKV {

    pub fn open(dir: &Path) -> DiskKV {
        Self { ldb: leveldb::LevelDB::open(dir) }
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

    
    fn save_batch(&self, batch: &leveldb::Writebatch) {
        self.ldb.write(&batch); // must
    }
}



