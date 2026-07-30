use super::{ItrErr, ItrErrCode, VmrtErr, VmrtRes};
use crate::value::ValueTy;

macro_rules! abst_call_type_define {
    ( $( $k:ident : $v:expr , [ $( $atk:ident ),* ] )+ ) => {
        #[allow(non_camel_case_types)]
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
        pub enum AbstCall {
        $(
            $k = $v,
        )+
        }
        enum_try_from_u8_by_variant!(
            AbstCall,
            ItrErrCode::AbstTypeError,
            "AbstCall type {} not found",
            [$( $k ),+]
        );
        impl AbstCall {
            pub fn check(n: u8) -> VmrtErr {
                Self::try_from_u8(n).map(|_| ())
            }

            pub const fn uint(self) -> u8 {
                self as u8
            }

            pub const fn can_register_defer(self) -> bool {
                self.uint() > Self::Deferred.uint()
            }

            pub fn from_name(name: &str) -> VmrtRes<Self> {
                Ok(match name {
                    $(
                    stringify!($k) => Self::$k,
                    )+
                    _ => {
                        return itr_err_fmt!(
                            ItrErrCode::AbstTypeError,
                            "AbstCall name {} not found",
                            name
                        )
                    }
                })
            }

            pub fn param_types(&self) -> Vec<ValueTy> {
                match self {
                    $(
                    Self::$k => vec![ $( ValueTy::$atk ),* ],
                    )+
                }
            }
        }
    }
}

abst_call_type_define! {
    Construct    : 0u8 , [ Bytes ]
    Change       : 1   , [ ]
    Append       : 2   , [ ]
    Deferred     : 50  , [ ]

    PermitHAC    : 55  , [ Address, Bytes ]
    PermitSAT    : 56  , [ Address, U64 ]
    PermitHACD   : 57  , [ Address, U32, Bytes ]
    PermitAsset  : 58  , [ Address, U64, U64 ]

    PayableHAC   : 65  , [ Address, Bytes ]
    PayableSAT   : 66  , [ Address, U64 ]
    PayableHACD  : 67  , [ Address, U32, Bytes ]
    PayableAsset : 68  , [ Address, U64, U64 ]
}
