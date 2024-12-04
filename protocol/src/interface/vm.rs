
pub trait VMI {
    fn active(&self) -> bool { false }
    fn call(&self, _: u8, _: u8, _: &[u8]) -> Ret<Vec<u8>> { unimplemented!() }
}