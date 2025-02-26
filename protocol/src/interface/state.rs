

pub trait State : Send + Sync {
    fn fork_sub(&self, _: Weak<dyn State>) -> Box<dyn State> { unimplemented!() }
    fn merge_sub(&mut self, _: Box<dyn State>) { unimplemented!() }
    fn to_mem(&self) -> MemMap { unimplemented!() }

    // fn set_parent(&mut self, _: Arc<dyn State>) { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }
    fn write_to_disk(&self) { unimplemented!() }

    fn get(&self,     _: Vec<u8>) -> Option<Vec<u8>> { unimplemented!() }
    fn set(&mut self, _: Vec<u8>, _: Vec<u8>) { unimplemented!() }
    fn del(&mut self, _: Vec<u8>) { unimplemented!() }
}



pub trait Store : Send + Sync {

    

}








