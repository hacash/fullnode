use leveldb::{
    database::{
        db::Database,
        batch::{Batch, WriteBatch},
    },
    options::{Options, ReadOptions, WriteOptions}
};


pub struct DiskKV {
    ldb: Database,
}


impl DiskKV {

    pub fn open(dir: &Path) -> DiskKV {
        let mut opt = Options::new();
        opt.create_if_missing = true;
        let Ok(ldb) = Database::open(dir, &opt) else {
            let dir = match dir.to_str() {
                Some(s) => s,
                _ => "unknown"
            };
            panic!("cannot open Level database by dir {}", dir)
        };
        // yes
        Self { ldb }
    }
}


impl DiskDB for DiskKV {

    fn remove(&self, k: &[u8]) {
        let opt = WriteOptions::new();
        self.ldb.delete_u8(&opt, k).unwrap(); // must 
    }

    fn save(&self, k: &[u8], v: &[u8]) {
        let opt = WriteOptions::new();
        self.ldb.put_u8(&opt, k, v).unwrap(); // must 
    }

    fn load(&self, k: &[u8]) -> Option<Vec<u8>> {
        let opt = ReadOptions::new();
        self.ldb.get_u8(&opt, k).unwrap() // must 
    }

    fn save_batch(&self, kvs: Vec<(Vec<u8>, Vec<u8>)>) {
        let batch = WriteBatch::new();
        for (k, v) in kvs.iter() {
            batch.put_u8(k, v);
        }
        self.ldb.write(&WriteOptions::new(), &batch).unwrap(); // must
    }
}



