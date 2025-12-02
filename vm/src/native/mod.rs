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
    context_address    = 1,    6,    Address  
    /* */
    hac_to_mei         = 31,   8,    U64 
    hac_to_zhu         = 32,   8,    U128 
    // hac_to_shuo         = 33,   8,    U128
    mei_to_hac         = 35,   8,    Bytes
    zhu_to_hac         = 36,   8,    Bytes
    // shuo_to_suo         = 37,   8,    Bytes
    address_ptr        = 41,   4,    U8
    /* */
    sha2               = 101, 32,    Bytes  
    sha3               = 102, 32,    Bytes 
    ripemd160          = 103, 20,    Bytes
    
}
