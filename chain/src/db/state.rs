
pub struct StateInst {
    disk: Arc<DiskKV>,
    mem: MemKV,
    parent: Weak<dyn State>,
}


impl StateInst {

    pub fn build(d: Arc<DiskKV>, p: Weak<dyn State>) -> Self where Self: Sized {
        Self {
            disk: d,
            parent: p,
            mem: MemKV::new()
        }
    }
    


}


impl State for StateInst {

    fn disk(&self) -> Arc<dyn DiskDB> {
        self.disk.clone()
    }
    
    fn fork_sub(&self, p: Arc<dyn State>) -> Box<dyn State> {
        Box::new(Self{
            disk: self.disk.clone(),
            mem: MemKV::new(),
            parent: Arc::downgrade(&p),
        })
    }

    
    fn write_to_disk(&self) {
        // debug_println!("write_to_disk !!!!!!");
        let mut batch = leveldb::Writebatch::new();
        for (k, v) in self.mem.memry.iter() {
            match v {
                MemItem::Delete => batch.delete(k),
                MemItem::Value(v) => batch.put(k, &v),
            };
        }
        self.disk.ldb.write(&batch); // must
    }

    
    fn get(&self, k: Vec<u8>) -> Option<Vec<u8>> {
        // search memory db
        if let Some(v) = self.mem.get(&k) {
            return match v {
                MemItem::Delete => None, // be delete by mark
                MemItem::Value(v) => Some(v), // yes be put in
            }
        }
        // search parent
        if let Some(parent) = self.parent.upgrade() {
            return parent.get(k)
        }
        // load from disk
        self.disk.load(&k)
    }

    
    fn set(&mut self, k: Vec<u8>, v: Vec<u8>) {
        self.mem.put(k, v)
    }

    
    fn del(&mut self, k: Vec<u8>) {
        self.mem.del(k) // add del mark
    }

}


