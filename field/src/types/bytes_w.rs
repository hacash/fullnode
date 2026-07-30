use std::ops::Deref;

use sys::Ret;

use crate::codec::{Decode, Encode, Reader};
use crate::types::uint::{Uint1, Uint2, Uint4};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BytesW1 {
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BytesW2 {
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BytesW4 {
    bytes: Vec<u8>,
}

macro_rules! impl_bytes_w {
    ($name:ident, $len_ty:ty, $int_ty:ty) => {
        impl $name {
            pub fn from(buf: Vec<u8>) -> Ret<Self> {
                <$len_ty>::from_usize(buf.len())?;
                Ok(Self { bytes: buf })
            }

            pub fn as_vec(&self) -> &Vec<u8> {
                &self.bytes
            }

            pub fn to_vec(&self) -> Vec<u8> {
                self.bytes.clone()
            }

            pub fn into_vec(self) -> Vec<u8> {
                self.bytes
            }

            pub fn length(&self) -> usize {
                self.bytes.len()
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.bytes
            }
        }

        impl Deref for $name {
            type Target = Vec<u8>;
            fn deref(&self) -> &Self::Target {
                &self.bytes
            }
        }

        impl Encode for $name {
            fn size(&self) -> usize {
                <$len_ty>::SIZE + self.bytes.len()
            }
            fn encode_to(&self, out: &mut Vec<u8>) {
                <$len_ty>::from(self.bytes.len() as $int_ty).encode_to(out);
                out.extend_from_slice(&self.bytes);
            }
        }

        impl Decode for $name {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut r = Reader::new(buf);
                let count: $len_ty = r.read()?;
                let bytes = r.read_bytes(count.uint() as usize)?.to_vec();
                Ok((Self { bytes }, r.used()))
            }
        }
    };
}

impl_bytes_w!(BytesW1, Uint1, u8);
impl_bytes_w!(BytesW2, Uint2, u16);
impl_bytes_w!(BytesW4, Uint4, u32);

impl BytesW1 {
    pub fn to_readable_or_hex(&self) -> String {
        sys::bytes_to_readable_string_or_hex(self.as_ref())
    }
}
