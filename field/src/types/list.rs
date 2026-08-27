use std::ops::Deref;
use sys::{Ret, errf};

use crate::codec::{Decode, Encode, Reader};
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

pub type AddressW1 = ListW1<Address>;
pub type ChainIDList = ListW1<Uint4>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_w1_count_bounds() {
        let empty = ListW1::<Uint1>::from(vec![]).unwrap();
        assert_eq!(empty.encode(), vec![0]);
        assert_eq!(empty.size(), empty.encode().len());

        let one = ListW1::from(vec![Uint1::from(7)]).unwrap();
        assert_eq!(one.size(), one.encode().len());
        let (decoded, used) = ListW1::<Uint1>::decode(&one.encode()).unwrap();
        assert_eq!(used, one.encode().len());
        assert_eq!(decoded.as_list(), one.as_list());

        let max = ListW1::from(vec![Uint1::from(1); u8::MAX as usize]).unwrap();
        assert_eq!(max.length(), u8::MAX as usize);
        assert_eq!(max.size(), max.encode().len());
        let (decoded, used) = ListW1::<Uint1>::decode(&max.encode()).unwrap();
        assert_eq!(used, max.encode().len());
        assert_eq!(decoded.length(), u8::MAX as usize);

        assert!(ListW1::<Uint1>::from(vec![Uint1::from(1); u8::MAX as usize + 1]).is_err());
        let mut list = ListW1::from(vec![Uint1::from(1); u8::MAX as usize]).unwrap();
        assert!(list.push(Uint1::from(1)).is_err());
    }

    #[test]
    fn list_w2_count_bounds() {
        let empty = ListW2::<Uint1>::from(vec![]).unwrap();
        assert_eq!(empty.encode(), vec![0, 0]);
        assert_eq!(empty.size(), empty.encode().len());

        let one = ListW2::from(vec![Uint1::from(7)]).unwrap();
        assert_eq!(one.size(), one.encode().len());
        let (decoded, used) = ListW2::<Uint1>::decode(&one.encode()).unwrap();
        assert_eq!(used, one.encode().len());
        assert_eq!(decoded.as_list(), one.as_list());

        let max = ListW2::from(vec![Uint1::from(1); u16::MAX as usize]).unwrap();
        assert_eq!(max.length(), u16::MAX as usize);
        assert_eq!(max.size(), max.encode().len());
        assert_eq!(&max.encode()[..2], &[0xff, 0xff]);

        assert!(ListW2::<Uint1>::from(vec![Uint1::from(1); u16::MAX as usize + 1]).is_err());
        let mut list = ListW2::from(vec![Uint1::from(1); u16::MAX as usize]).unwrap();
        assert!(list.push(Uint1::from(1)).is_err());
    }
}
