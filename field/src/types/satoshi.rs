use sys::{Ret, errf};

use crate::codec::{Decode, Encode};
use crate::types::fold64::Fold64;
use crate::types::uint::Uint8;

pub type Satoshi = Uint8;
pub const SATOSHI_MAX: u64 = 21_000_000 * 100_000_000;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SatoshiAuto(Fold64);

impl SatoshiAuto {
    pub const MAX: u64 = SATOSHI_MAX;

    fn from_fold64(value: Fold64) -> Ret<Self> {
        if value.uint() > Self::MAX {
            return errf!("satoshi value {} exceeds max {}", value.uint(), Self::MAX);
        }
        Ok(Self(value))
    }

    pub fn uint(&self) -> u64 {
        self.0.uint()
    }

    pub fn from_satoshi(satoshi: &Satoshi) -> Ret<Self> {
        Self::from_fold64(Fold64::from(satoshi.uint())?)
    }

    pub fn check_satoshi(satoshi: &Satoshi) -> Ret<()> {
        if satoshi.uint() > Self::MAX {
            return errf!("satoshi value {} exceeds max {}", satoshi.uint(), Self::MAX);
        }
        Ok(())
    }

    pub fn to_satoshi(&self) -> Satoshi {
        Satoshi::from(self.0.uint())
    }
}

impl Encode for SatoshiAuto {
    fn size(&self) -> usize {
        self.0.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.0.encode_to(out);
    }
}

impl Decode for SatoshiAuto {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (value, used) = Fold64::decode(buf)?;
        Ok((Self::from_fold64(value)?, used))
    }
}
