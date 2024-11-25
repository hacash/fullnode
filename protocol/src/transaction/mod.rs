use std::collections::*;

use sys::*;
use field::{self, *, interface::*};

use super::*;
use super::operate;
use super::action::*;
use super::state::*;


include!{"define.rs"}
include!{"util.rs"}
include!{"macro.rs"}
include!{"coinbase.rs"}
include!{"create.rs"}

/*
* define
*/
transaction_define!{ TransactionType1, 1u8 }
transaction_define!{ TransactionType2, 2u8 }
transaction_define!{ TransactionType3, 3u8 }

/*
* register
*/
transaction_register!{
    TransactionCoinbase
    TransactionType1
    TransactionType2
    TransactionType3
}