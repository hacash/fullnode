
pub trait VM {
    fn new() -> Self where Self: Sized { unimplemented!() }
    fn main_call(&mut self, _: &Vec<u8>) -> Ret<bool> { unimplemented!() }
}




