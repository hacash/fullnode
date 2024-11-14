
use sys::*;
use field::{self, *, interface::*};

use super::interface::*;
use super::action::*;


include!{"define.rs"}
include!{"macro.rs"}
include!{"create.rs"}

include!{"coinbase.rs"}


/*
* define
*/


transaction_define!{ TransactionType1, 1u8 }
transaction_define!{ TransactionType2, 2u8 }
transaction_define!{ TransactionType3, 3u8 }

