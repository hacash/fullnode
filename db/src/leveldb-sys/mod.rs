use std::ptr;
use std::ffi::{ c_void, c_char, CString };
use leveldb_sys::*;
use libc::size_t;


include!("error.rs");
include!("bytes.rs");
// include!("batch.rs");
include!("db.rs");

