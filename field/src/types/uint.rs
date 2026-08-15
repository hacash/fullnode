use std::fmt;
use std::ops::Deref;
use std::sync::OnceLock;

use sys::{Ret, normalf, errf};

use crate::codec::{Decode, Encode};

const fn uint_max(bytes: usize, full: usize) -> u128 {
    if bytes == full {
        u128::MAX
    } else {
        (1u128 << (bytes * 8)) - 1
    }
}

macro_rules! fixed_uint {
    ($name:ident, $prim:ty, $n:expr) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($prim);

        impl $name {
            pub const SIZE: usize = $n;
            pub const MAX: $prim = uint_max($n, core::mem::size_of::<$prim>()) as $prim;

            pub const fn from(v: $prim) -> Self {
                if v > Self::MAX {
                    panic!(concat!(stringify!($name), " overflow"));
                }
                Self(v)
            }

            pub fn from_checked(v: $prim) -> Option<Self> {
                (v <= Self::MAX).then_some(Self(v))
            }

            pub fn from_usize(v: usize) -> Ret<Self> {
                if (v as u128) > (Self::MAX as u128) {
                    return errf!(
                        "{} value {} exceeds max {}",
                        stringify!($name),
                        v,
                        Self::MAX
                    );
                }
                Ok(Self(v as $prim))
            }

            pub fn uint(&self) -> $prim {
                self.0
            }

            pub fn zero_ref() -> &'static Self {
                static Z: OnceLock<$name> = OnceLock::new();
                Z.get_or_init(|| $name::from(0))
            }
        }

        impl From<$prim> for $name {
            fn from(v: $prim) -> Self {
                Self::from(v)
            }
        }

        impl Deref for $name {
            type Target = $prim;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Encode for $name {
            fn size(&self) -> usize {
                $n
            }
            fn encode_to(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(
                    &self.0.to_be_bytes()[(core::mem::size_of::<$prim>() - $n)..],
                );
            }
        }

        impl Decode for $name {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                if buf.len() < $n {
                    return normalf!("buffer too short for {}", stringify!($name));
                }
                let mut full = [0u8; core::mem::size_of::<$prim>()];
                full[(core::mem::size_of::<$prim>() - $n)..].copy_from_slice(&buf[..$n]);
                let value = <$prim>::from_be_bytes(full);
                if value > Self::MAX {
                    return normalf!(
                        "{} parse value {} exceeds max {}",
                        stringify!($name),
                        value,
                        Self::MAX
                    );
                }
                Ok((Self(value), $n))
            }
        }
    };
}

fixed_uint!(Uint1, u8, 1);
fixed_uint!(Uint2, u16, 2);
fixed_uint!(Uint3, u32, 3);
fixed_uint!(Uint4, u32, 4);
fixed_uint!(Uint5, u64, 5);
fixed_uint!(Uint6, u64, 6);
fixed_uint!(Uint7, u64, 7);
fixed_uint!(Uint8, u64, 8);
fixed_uint!(Uint10, u128, 10);
fixed_uint!(Uint12, u128, 12);
fixed_uint!(Uint16, u128, 16);

pub type BlockHeight = Uint5;
