macro_rules! codec_struct {
    ($name:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct $name {
            $(pub $field: $ty),+
        }

        impl Encode for $name {
            fn size(&self) -> usize {
                0 $(+ self.$field.size())+
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                $(self.$field.encode_to(out);)+
            }
        }

        impl Decode for $name {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut r = Reader::new(buf);
                $(let $field: $ty = r.read()?;)+
                Ok((Self { $($field),+ }, r.used()))
            }
        }
    };
}
#[allow(unused_imports)]
pub(crate) use codec_struct;

mod addr_or_list;
mod addr_or_ptr;
mod address;
mod amount;
mod asset;
mod balance;
mod bool;
mod bytes_w;
mod channel;
mod diamond;
mod diamond_sto;
mod fixed;
mod fold64;
mod json_impls;
mod list;
mod satoshi;
mod sign;
mod timestamp;
mod uint;

pub use addr_or_list::*;
pub use addr_or_ptr::*;
pub use address::*;
pub use amount::*;
pub use asset::*;
pub use balance::*;
pub use bool::*;
pub use bytes_w::*;
pub use channel::*;
pub use diamond::*;
pub use diamond_sto::*;
pub use fixed::*;
pub use fold64::*;
pub use list::*;
pub use satoshi::*;
pub use sign::*;
pub use timestamp::*;
pub use uint::*;
