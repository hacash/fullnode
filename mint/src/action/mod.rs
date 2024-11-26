use sys::*;
use protocol::interface::*;

// 

pub fn empty_try_create(_kind: u16, _buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
    Ok(None)
}



