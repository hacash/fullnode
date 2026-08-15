use sys::{Ret, normalf};

use crate::codec::{Decode, Encode};
use crate::types::fixed::Hash;
use crate::types::list::{ListW1, ListW2};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sign {
    pub publickey: [u8; Self::PUBLICKEY_SIZE],
    pub signature: [u8; Self::SIGNATURE_SIZE],
}

impl Default for Sign {
    fn default() -> Self {
        Self {
            publickey: [0u8; Self::PUBLICKEY_SIZE],
            signature: [0u8; Self::SIGNATURE_SIZE],
        }
    }
}

impl Sign {
    pub const PUBLICKEY_SIZE: usize = 33;
    pub const SIGNATURE_SIZE: usize = 64;
    pub const SIZE: usize = Self::PUBLICKEY_SIZE + Self::SIGNATURE_SIZE;

    pub fn create_by(acc: &sys::Account, hash: &Hash) -> Self {
        Self {
            publickey: acc.public_key().serialize_compressed(),
            signature: acc.do_sign(&hash.0),
        }
    }
}

impl Encode for Sign {
    fn size(&self) -> usize {
        Self::SIZE
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.publickey);
        out.extend_from_slice(&self.signature);
    }
}

impl Decode for Sign {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if buf.len() < Self::SIZE {
            return normalf!("buffer too short for Sign");
        }
        let mut publickey = [0u8; Self::PUBLICKEY_SIZE];
        let mut signature = [0u8; Self::SIGNATURE_SIZE];
        publickey.copy_from_slice(&buf[..Self::PUBLICKEY_SIZE]);
        signature.copy_from_slice(&buf[Self::PUBLICKEY_SIZE..Self::SIZE]);
        Ok((
            Self {
                publickey,
                signature,
            },
            Self::SIZE,
        ))
    }
}

pub type SignW1 = ListW1<Sign>;
pub type SignW2 = ListW2<Sign>;
