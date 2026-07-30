use sys::{Ret, decodef};

use crate::codec::{Decode, Encode};
use crate::types::fixed::Fixed1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Bool(Fixed1);

impl Bool {
    pub fn new(v: bool) -> Self {
        Self(Fixed1::from([if v { 1 } else { 0 }]))
    }

    pub fn is_true(&self) -> bool {
        self.0[0] != 0
    }
}

impl Encode for Bool {
    fn size(&self) -> usize {
        1
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for Bool {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (v, n) = Fixed1::decode(buf)?;
        if v[0] > 1 {
            return decodef!("Bool value {} invalid", v[0]);
        }
        Ok((Self(v), n))
    }
}
