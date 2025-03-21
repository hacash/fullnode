use std::collections::*;

pub type MemMap = HashMap<Vec<u8>, Option<Vec<u8>>>;



#[derive(Default)]
pub struct MemKV {
    pub memry: MemMap
}

impl MemKV {

    pub fn new() -> MemKV {
        Self {
            memry: HashMap::default()
        }
    }

    pub fn del(&mut self, k: Vec<u8>) {
        // self.batch.delete(&k);
        self.memry.insert(k, None);
    }
    
    pub fn put(&mut self, k: Vec<u8>, v: Vec<u8>) {
        // self.batch.put(&k, &v);
        self.memry.insert(k, Some(v));
    }
    
    pub fn get(&self, k: &Vec<u8>) -> Option<Option<Vec<u8>>> {
        match self.memry.get(k) {
            None => None,
            Some(item) => Some(item.clone()),
        }
    }

    pub fn to_writebatch(&self) -> Writebatch {
        let mut batch = Writebatch::new();
        for (k, v) in self.memry.iter() {
            match v {
                None => batch.delete(k),
                Some(v) => batch.put(k, &v),
            };
        }
        batch
    }

}


/**************************************************** */


pub struct MemBatch {
    batch: Writebatch
}

impl MemBatch {

    pub fn new() -> MemBatch {
        Self {
            batch: Writebatch::new()
        }
    }

    pub fn del(&mut self, k: &[u8]) {
        // self.batch.delete(&k);
        self.batch.delete(k);
    }
    
    pub fn put(&mut self, k: &[u8], v: &[u8]) {
        // self.batch.put(&k, &v);
        self.batch.put(k, v);
    }

    pub fn as_writebatch(&self) -> &Writebatch {
        &self.batch
    }

    pub fn into_writebatch(self) -> Writebatch {
        self.batch
    }

}