
pub mod interface;

// use std::fmt;
// use std::fmt::Display;
// use std::cmp::Ordering::{Less,Greater};
// use std::cmp::{Ordering, PartialOrd, Ord};
// use std::ops::{Deref, Add, Sub, Mul, Div, AddAssign, SubAssign, MulAssign, DivAssign, Index, IndexMut};
// use std::convert::TryInto;

use std::fmt::Display;
use std::cmp::Ordering;
use std::ops::{
    Deref, Add, Sub, Mul, Div, 
    AddAssign, SubAssign, MulAssign, DivAssign
};

use concat_idents::concat_idents;

use sys::*;


include!("impl.rs");

include!("macro_compute.rs");
include!("macro_uint.rs");

include!("empty.rs");
include!("uint.rs");
include!("fold64.rs");



