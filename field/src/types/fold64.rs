use std::fmt;

use sys::{Ret, errf, normalf};

use crate::codec::{Decode, Encode};

const FOLD64_STAGE1_BOUND: u64 = 32;
const FOLD64_STAGE2_BOUND: u64 = FOLD64_STAGE1_BOUND * 256;
const FOLD64_STAGE3_BOUND: u64 = FOLD64_STAGE2_BOUND * 256;
const FOLD64_STAGE4_BOUND: u64 = FOLD64_STAGE3_BOUND * 256;
const FOLD64_STAGE5_BOUND: u64 = FOLD64_STAGE4_BOUND * 256;
const FOLD64_STAGE6_BOUND: u64 = FOLD64_STAGE5_BOUND * 256;
const FOLD64_STAGE7_BOUND: u64 = FOLD64_STAGE6_BOUND * 256;
const FOLD64_STAGE8_BOUND: u64 = FOLD64_STAGE7_BOUND * 256;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fold64 {
    value: u64,
}

impl Fold64 {
    pub const MAX: u64 = FOLD64_STAGE8_BOUND - 1;

    pub fn from(v: u64) -> Ret<Self> {
        if v > Self::MAX {
            return errf!("Fold64 value {} exceeds max {}", v, Self::MAX);
        }
        Ok(Self { value: v })
    }

    pub fn uint(&self) -> u64 {
        self.value
    }
}

impl Encode for Fold64 {
    fn size(&self) -> usize {
        let v = self.value;
        if v < FOLD64_STAGE1_BOUND {
            return 1;
        }
        let bits = 64 - v.leading_zeros();
        let extra = bits.saturating_sub(5) as usize;
        1 + (extra + 7) / 8
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        let total_bytes = self.size() as u8;
        let head = (total_bytes - 1) << 5;
        let data = self.value.to_be_bytes();
        let be_start = 8 - total_bytes as usize;
        let start = out.len();
        out.extend_from_slice(&data[be_start..]);
        out[start] = (out[start] & 0b0001_1111) | head;
    }
}

impl Decode for Fold64 {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if buf.is_empty() {
            return normalf!("buffer too short for Fold64");
        }
        let bt = buf[0];
        let n = (bt >> 5) as usize;
        if buf.len() < 1 + n {
            return normalf!("Fold64 parse length {} < {}", buf.len(), 1 + n);
        }
        let mut value = (bt & 0b0001_1111) as u64;
        for i in 0..n {
            value = (value << 8) | buf[1 + i] as u64;
        }
        let fold = Self::from(value)?;
        if fold.size() != 1 + n {
            return normalf!(
                "Fold64 non-canonical size {} expected {}",
                1 + n,
                fold.size()
            );
        }
        Ok((fold, 1 + n))
    }
}

impl fmt::Display for Fold64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl Fold64 {
    pub fn checked(self) -> Ret<Self> {
        Self::from(self.value)
    }

    pub fn is_zero(&self) -> bool {
        self.value == 0
    }
}
