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


pub fn to_readable_or_base64(s: &[u8]) -> String {
    match bytes_try_to_readable_string(s) {
        Some(s) => s,
        _ => BASE64_STANDARD.encode(s)
    }
}