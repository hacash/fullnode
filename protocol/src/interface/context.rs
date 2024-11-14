
pub trait Context {
    fn env(&self) -> &Env { unimplemented!() }
    fn state(&self) -> Arc<dyn State> { unimplemented!() }
    fn disk(&self) -> Arc<dyn DiskDB> { unimplemented!() }
    fn vm(&self) -> Arc<dyn VM> { unimplemented!() }
}

