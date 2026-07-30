use base64::prelude::*;

pub trait ToBase64 {
    fn to_base64(&self) -> String;
}

impl ToBase64 for [u8] {
    fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(self)
    }
}

impl<const N: usize> ToBase64 for [u8; N] {
    fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(self)
    }
}

pub fn to_readable_or_base64(s: &[u8]) -> String {
    match crate::bytes_try_to_readable_string(s) {
        Some(s) => s,
        None => BASE64_STANDARD.encode(s),
    }
}
