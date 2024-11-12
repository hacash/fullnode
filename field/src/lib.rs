
pub mod interface;

// use std::fmt;
// use std::fmt::Display;
// use std::cmp::Ordering::{Less,Greater};
// use std::cmp::{Ordering, PartialOrd, Ord};
// use std::ops::{Deref, Add, Sub, Mul, Div, AddAssign, SubAssign, MulAssign, DivAssign, Index, IndexMut};
// use std::convert::TryInto;

use std::fmt::{Display, Formatter, Result};
use std::cmp::Ordering;
use std::ops::{
    Deref, Index, IndexMut, Add, Sub, Mul, Div, 
    AddAssign, SubAssign, MulAssign, DivAssign
};

use concat_idents::concat_idents;
use base64::prelude::*;

use sys::*;


include!{"impl.rs"}
include!{"empty.rs"}

// number
include!{"number/macro_compute.rs"}
include!{"number/macro_uint.rs"}
include!{"number/uint.rs"}
include!{"number/fold64.rs"}

// bytes
include!{"bytes/fixed.rs"}
include!{"bytes/datas.rs"}

// core
include!{"core/define.rs"}
include!{"core/address.rs"}

