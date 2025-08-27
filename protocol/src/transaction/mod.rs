use std::collections::*;

use sys::*;
use field::{self, *, interface::*};

use super::*;
use super::interface::*;
use super::operate;
use super::action::*;
use super::state::*;
use super::component::*;


include!{"util.rs"}
include!{"macro.rs"}
include!{"coinbase.rs"}
include!{"create.rs"}
include!{"store.rs"}

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