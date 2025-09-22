use field::interface::*;
use field::*;

use super::rt::*;
use super::rt::ItrErrCode::*;
use super::value::*;

include!("hash.rs");
include!("amount.rs");
include!("types.rs");




/*
    Native call define
*/
native_call_define!{  // idx, gas,   vsize, ValueType
    sha2               = 1,   32,    32, ValueTy::Bytes // Bytes[32]
    sha3               = 2,   32,    32, ValueTy::Bytes // Bytes[32]
    ripemd160          = 3,   32,    32, ValueTy::Bytes // Bytes[32]
    amount_to_zhu      = 22,   8,    16, ValueTy::U128  // U128
    amount_to_mei      = 21,   8,     8, ValueTy::U64   // U64
}




