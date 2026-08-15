use crate::{Ret, normalf};

pub fn bytes_from_hex(stuff: &[u8], len: usize) -> Ret<Vec<u8>> {
    let got = stuff.len();
    let expect = len * 2;
    if got != expect {
        return normalf!(
            "hex size invalid: expected {} chars but got {}",
            expect,
            got
        );
    }
    hex::decode(stuff)
        .map(|b| b[..len].to_vec())
        .map_err(|e| crate::Error::normal(e.to_string()))
}

pub trait ToHex {
    fn to_hex(&self) -> String;
}

impl ToHex for [u8] {
    fn to_hex(&self) -> String {
        hex::encode(self)
    }
}

impl<const N: usize> ToHex for [u8; N] {
    fn to_hex(&self) -> String {
        hex::encode(self)
    }
}
