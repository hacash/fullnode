
pub trait VMI {
    fn active(&self) -> bool { false }
    fn call(&mut self, _: u8, _: u8, _: &[u8], _: Vec<u8>) -> Ret<Vec<u8>> { unimplemented!() }
}