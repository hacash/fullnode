
pub trait P2sh : Send + Sync {
    fn code_stuff(&self) -> &[u8];
}
