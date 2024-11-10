use base64::prelude::*;

/*
pub fn bytes_from_base64(stuff: &[u8], len: usize) -> Ret<Vec<u8>> {
    panic!("");
    Ok(vec![])
}
*/


//////////////////////////


pub trait ToBase64 {
    fn base64(&self) -> String;
}


impl ToBase64 for Vec<u8> {

    fn base64(&self) -> String {
        BASE64_STANDARD.encode(self)
    }

}