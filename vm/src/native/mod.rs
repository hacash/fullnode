use field::interface::*;
use field::*;

use super::rt::*;
use super::rt::ItrErrCode::*;
use super::value::*;

include!("hash.rs");
include!("types.rs");
include!("amount.rs");
include!("address.rs");



use ValueTy::*;

/*
    Native call define
*/
native_call_define!{  // idx, gas,   ValueType
    sha2               = 1,   32,    Bytes  
    sha3               = 2,   32,    Bytes 
    ripemd160          = 3,   20,    Bytes
    /* */
    hac_to_mei         = 21,   8,    U64 
    hac_to_zhu         = 22,   8,    U128 
    // hac_to_shuo         = 23,   8,    U128
    mei_to_hac         = 25,   8,    Bytes
    zhu_to_hac         = 26,   8,    Bytes
    // shuo_to_suo         = 27,   8,    Bytes
    address_ptr        = 30,   4,    U8
    context_address    = 31,   6,    Address  
    
}
