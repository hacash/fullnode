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

// codec-only 与 full 二选一（见 Cargo.toml features 注释）；两者都不启用说明
// 依赖方配错了 feature，编译期报错而不是静默编译出半套 VM。
#[cfg(all(not(feature = "full"), not(feature = "codec-only")))]
compile_error!("vm requires either `full` or `codec-only` feature");

pub mod action;
pub mod api;
#[macro_use]
pub(crate) mod rt;
pub(crate) mod contract;
#[cfg(feature = "full")]
pub mod fitshc;
pub(crate) mod frame;
pub(crate) mod interpreter;
#[cfg(feature = "full")]
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
