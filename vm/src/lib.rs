//! `vm` ——
//!
//! `Registry`
//! - `vm_assigner` Context  vm  VM
//!
//! crate  mint vm  `mint::action::DiaInscEdit`
//! mint ——  protocol

#![allow(dead_code)]
#![allow(unused_macros)]

#[macro_use]
extern crate sys;

#[macro_export]
macro_rules! s {
    ("") => {
        String::new()
    };
    ($v:expr) => {
        ($v).to_string()
    };
}

#[macro_export]
macro_rules! never {
    () => {
        panic!("never call this")
    };
}

#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}

pub mod action;
pub mod api;
#[macro_use]
pub(crate) mod rt;
pub(crate) mod contract;
pub mod fitshc;
pub(crate) mod frame;
pub(crate) mod interpreter;
pub(crate) mod ir;
pub(crate) mod machine;
pub(crate) mod native;
pub mod setup;
pub(crate) mod space;
pub(crate) mod state;
pub(crate) mod value;

pub use machine::peek_vm_runtime_limits;
pub use setup::register;
pub use state::{StorageDebug, VMState, VMStateRead, VmLog};
pub use value::ContractAddress;

pub const MAX_FUNC_PARAM_LEN: usize = 15;
