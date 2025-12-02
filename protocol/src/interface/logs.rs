

pub trait Logs : Send + Sync {
    fn push(&mut self, _ :&dyn Serialize) {}
    fn load(&self, _: u64, _: usize) -> Option<Vec<u8>> { None }
    fn remove(&self, _: u64) {}
    //
    fn height(&self) -> u64 { 0 }
    fn write_to_disk(&self) {}
}