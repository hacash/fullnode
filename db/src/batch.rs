

/**************************************************** */


pub struct Membatch {
    batch: Writebatch
}

impl MemBatch for Membatch {

    fn from_memkv(kv: &dyn MemDB) -> Self {
        let mut batch = Self::new();
        kv.for_each(&mut |k, v|{
            match v {
                None => batch.del(k.as_ref()),
                Some(v) => batch.put(k, &v),
            };
        });
        batch
    }

    fn new() -> Self {
        Self {
            batch: Writebatch::new()
        }
    }

    fn del(&mut self, k: &[u8]) {
        // self.batch.delete(&k);
        self.batch.delete(k);
    }
    
    fn put(&mut self, k: &[u8], v: &[u8]) {
        // self.batch.put(&k, &v);
        self.batch.put(k, v);
    }
}


impl Membatch {

    fn into_batch(self) -> Writebatch {
        self.batch
    }

}