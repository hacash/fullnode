



pub struct MemKV {
    memry: MemMap
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

}