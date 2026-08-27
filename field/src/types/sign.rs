use sys::Account;

use crate::types::fixed::{Fixed, Hash};
use crate::types::list::{ListW1, ListW2};

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct Sign {
    pub publickey: Fixed<33>,
    pub signature: Fixed<64>,
}

impl Sign {
    pub const PUBLICKEY_SIZE: usize = 33;
    pub const SIGNATURE_SIZE: usize = 64;
    pub const SIZE: usize = Self::PUBLICKEY_SIZE + Self::SIGNATURE_SIZE;

    pub fn create_by(acc: &Account, hash: &Hash) -> Self {
        Self {
            publickey: Fixed::from(acc.public_key().serialize_compressed()),
            signature: Fixed::from(acc.do_sign(&hash.0)),
        }
    }
}

pub type SignW1 = ListW1<Sign>;
pub type SignW2 = ListW2<Sign>;
