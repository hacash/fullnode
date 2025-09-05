
pub trait VM {
    fn usable(&self) -> bool { false }
    fn call(&mut self, _: &mut dyn Context, _: &mut dyn State, _: u8, _: u8, _: &[u8], _: Vec<u8>) 
        -> Ret<(i64, Vec<u8>)> { never!() }
}


pub struct VMNil {}
impl VM for VMNil {}

impl VMNil {
    pub fn new() -> Self {
        VMNil{}
    }

    pub fn empty() -> Box<dyn VM> {
        Box::new(VMNil::new())
    }
}



