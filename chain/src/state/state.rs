
pub struct StateInst {
    disk: Arc<dyn DiskDB>,
    mem: MemKV,
    parent: Weak<dyn State>,
}


impl StateInst {

    fn build(d: Arc<dyn DiskDB>, p: Weak<dyn State>) -> Self where Self: Sized {
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
    
    fn fork_sub(&self, p: Weak<dyn State>) -> Box<dyn State> {
        Box::new(Self{
            disk: self.disk.clone(),
            mem: MemKV::new(),
            parent: p,
        })
    }

    fn merge_sub(&mut self, sta: Box<dyn State>) {
        self.mem.memry.extend(sta.as_mem().clone())
    }

    fn as_mem(&self) -> &MemMap {
        &self.mem.memry
    }
    
    fn write_to_disk(&self) {
        // debug_println!("write_to_disk !!!!!!");
        self.disk.write(&self.mem); // must
    }

    
    fn get(&self, k: Vec<u8>) -> Option<Vec<u8>> {
        // search memory db
        if let Some(v) = self.mem.get(&k) {
            return v
        }
        // search parent
        if let Some(parent) = self.parent.upgrade() {
            return parent.get(k)
        }
        // load from disk
        self.disk.read(&k)
    }

    
    fn set(&mut self, k: Vec<u8>, v: Vec<u8>) {
        self.mem.put(k, v)
    }

    
    fn del(&mut self, k: Vec<u8>) {
        self.mem.del(k) // add del mark
    }

}


