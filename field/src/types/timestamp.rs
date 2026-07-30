use sys::{Ret, errf};

use crate::codec::{Decode, Encode};
use crate::types::uint::Uint5;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(Uint5);

impl Timestamp {
    pub const SIZE: usize = Uint5::SIZE;
    pub const MAX: u64 = Uint5::MAX;

    pub fn from_checked(value: u64) -> Ret<Self> {
        if value > Self::MAX {
            return errf!("Timestamp value {} exceeds max {}", value, Self::MAX);
        }
        Ok(Self(Uint5::from(value)))
    }

    pub fn value(&self) -> u64 {
        self.0.uint()
    }

    pub fn zero_ref() -> &'static Timestamp {
        static Z: Timestamp = Timestamp(Uint5::from(0));
        &Z
    }
}

impl From<u64> for Timestamp {
    fn from(value: u64) -> Self {
        Self(Uint5::from(value))
    }
}

impl Encode for Timestamp {
    fn size(&self) -> usize {
        self.0.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for Timestamp {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (value, used) = Uint5::decode(buf)?;
        Ok((Self(value), used))
    }
}
