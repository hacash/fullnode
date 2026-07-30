use std::fmt;
use std::ops::Deref;

use sys::{Ret, decodef};

use crate::codec::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fixed<const N: usize>(pub [u8; N]);

impl<const N: usize> Default for Fixed<N> {
    fn default() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> Fixed<N> {
    pub const SIZE: usize = N;
    pub const DEFAULT: Self = Self([0u8; N]);

    pub const fn from(v: [u8; N]) -> Self {
        Self(v)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_array(&self) -> &[u8; N] {
        &self.0
    }

    pub fn into_array(self) -> [u8; N] {
        self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }
}

impl<const N: usize> From<[u8; N]> for Fixed<N> {
    fn from(v: [u8; N]) -> Self {
        Self(v)
    }
}

impl<const N: usize> AsRef<[u8]> for Fixed<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl<const N: usize> Deref for Fixed<N> {
    type Target = [u8; N];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> fmt::Display for Fixed<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

impl<const N: usize> Encode for Fixed<N> {
    fn size(&self) -> usize {
        N
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.0);
    }
}

impl<const N: usize> Decode for Fixed<N> {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if buf.len() < N {
            return decodef!("buffer too short for Fixed<{}>", N);
        }
        let mut a = [0u8; N];
        a.copy_from_slice(&buf[..N]);
        Ok((Self(a), N))
    }
}

pub type Fixed1 = Fixed<1>;
pub type Fixed2 = Fixed<2>;
pub type Fixed3 = Fixed<3>;
pub type Fixed4 = Fixed<4>;
pub type Fixed5 = Fixed<5>;
pub type Fixed6 = Fixed<6>;
pub type Fixed7 = Fixed<7>;
pub type Fixed8 = Fixed<8>;
pub type Fixed9 = Fixed<9>;
pub type Fixed10 = Fixed<10>;
pub type Fixed12 = Fixed<12>;
pub type Fixed15 = Fixed<15>;
pub type Fixed16 = Fixed<16>;
pub type Fixed18 = Fixed<18>;
pub type Fixed20 = Fixed<20>;
pub type Fixed21 = Fixed<21>;
pub type Fixed32 = Fixed<32>;
pub type Fixed33 = Fixed<33>;
pub type Fixed64 = Fixed<64>;

pub type Hash = Fixed<32>;
pub type HashHalf = Fixed<16>;
pub type HashNonce = Fixed<8>;
pub type HashCheck = Fixed<4>;
pub type HashMark = Fixed<2>;
