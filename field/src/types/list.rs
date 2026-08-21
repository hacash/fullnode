use std::ops::Deref;
use sys::{Ret, errf};

use crate::codec::{Decode, Encode, ParsePrefix, Reader};
use crate::types::address::Address;
use crate::types::uint::{Uint1, Uint2, Uint4};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListW1<T>(pub Vec<T>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListW2<T>(pub Vec<T>);

macro_rules! list_view {
    ($($name:ident),+ $(,)?) => {
        $(
            impl<T> Deref for $name<T> {
                type Target = [T];

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            impl<'a, T> IntoIterator for &'a $name<T> {
                type Item = &'a T;
                type IntoIter = std::slice::Iter<'a, T>;

                fn into_iter(self) -> Self::IntoIter {
                    self.0.iter()
                }
            }
        )+
    };
}

list_view!(ListW1, ListW2);

macro_rules! list_w {
    ($name:ident, $len_ty:ty) => {
        impl<T> $name<T> {
            pub fn from(v: Vec<T>) -> Ret<Self> {
                <$len_ty>::from_usize(v.len())?;
                Ok(Self(v))
            }
            pub fn as_vec(&self) -> &Vec<T> {
                &self.0
            }
            pub fn as_list(&self) -> &Vec<T> {
                &self.0
            }
            pub fn as_mut(&mut self) -> &mut Vec<T> {
                &mut self.0
            }
            pub fn into_vec(self) -> Vec<T> {
                self.0
            }
            pub fn length(&self) -> usize {
                self.0.len()
            }
            pub fn push(&mut self, v: T) -> Ret<()> {
                <$len_ty>::from_usize(self.0.len() + 1)?;
                self.0.push(v);
                Ok(())
            }
            pub fn drop(&mut self, i: usize) -> Ret<T> {
                if i >= self.0.len() {
                    return errf!("list index overflow");
                }
                Ok(self.0.remove(i))
            }
        }

        impl<T: Encode> Encode for $name<T> {
            fn size(&self) -> usize {
                <$len_ty>::SIZE + self.0.iter().map(|v| v.size()).sum::<usize>()
            }
            fn encode_to(&self, out: &mut Vec<u8>) {
                <$len_ty>::from_usize(self.0.len())
                    .expect(concat!(stringify!($name), " length overflow"))
                    .encode_to(out);
                for v in &self.0 {
                    v.encode_to(out);
                }
            }
        }

        impl<T: Decode> Decode for $name<T> {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut r = Reader::new(buf);
                let count: $len_ty = r.read()?;
                let mut vals = Vec::with_capacity(count.uint() as usize);
                for _ in 0..count.uint() {
                    vals.push(r.read()?);
                }
                Ok((Self(vals), r.used()))
            }
        }
    };
}

list_w!(ListW1, Uint1);
list_w!(ListW2, Uint2);

impl<T: Decode> ParsePrefix for ListW1<T> {
    fn create_with_prefix(prefix: &[u8], rest: &[u8]) -> Ret<(Self, usize)> {
        if prefix.len() != Uint1::SIZE {
            return errf!("ListW1 prefix must be {} byte", Uint1::SIZE);
        }
        let count = prefix[0] as usize;
        let mut r = Reader::new(rest);
        let mut vals = Vec::with_capacity(count);
        for _ in 0..count {
            vals.push(r.read()?);
        }
        Ok((Self(vals), prefix.len() + r.used()))
    }
}

pub type AddressW1 = ListW1<Address>;
pub type ChainIDList = ListW1<Uint4>;
