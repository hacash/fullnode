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

// Exactly one of `codec-only` and `full` should be enabled (see the Cargo.toml
// features comments); if neither is, the dependent crate misconfigured its feature,
// so fail at compile time instead of silently compiling half a VM.
#[cfg(all(not(feature = "full"), not(feature = "codec-only")))]
compile_error!("vm requires either `full` or `codec-only` feature");

pub mod action;
#[cfg(feature = "full")]
pub mod api;
#[macro_use]
pub(crate) mod rt;
pub(crate) mod contract;
#[cfg(feature = "full")]
pub mod fitshc;
// The execution engine (frame/interpreter/machine/native/state/setup/api) compiles
// only under `full`: codec-only (SDK/wasm) pulls in no execution dependencies
// (see the Cargo.toml features comments).
#[cfg(feature = "full")]
pub(crate) mod frame;
#[cfg(feature = "full")]
pub(crate) mod interpreter;
#[cfg(feature = "full")]
pub(crate) mod ir;
#[cfg(feature = "full")]
pub(crate) mod machine;
#[cfg(feature = "full")]
pub(crate) mod native;
#[cfg(feature = "full")]
pub mod setup;
pub(crate) mod space;
#[cfg(feature = "full")]
pub(crate) mod state;
pub(crate) mod value;

#[cfg(feature = "full")]
pub use machine::peek_vm_runtime_limits;
#[cfg(feature = "full")]
pub use setup::register;
#[cfg(feature = "full")]
pub use state::{StorageDebug, VMState, VMStateRead, VmLog};
pub use value::ContractAddress;

pub const MAX_FUNC_PARAM_LEN: usize = 15;

/// Wire schema exports (collected by `codec-schema-gen`; purely static data, not
/// involved in execution). The contract structs register themselves via
/// `contract::struct_schemas()`; the two remaining entries are composite/leaf
/// types that keep hand-written impls.
pub mod codec_schema {
    pub fn struct_schemas() -> Vec<base::StructSchema> {
        let mut v = crate::contract::struct_schemas();
        v.push(<crate::rt::CodeStuff as base::StructSchemaProvider>::STRUCT_SCHEMA);
        v.push(<crate::rt::FuncArgvTypes as base::StructSchemaProvider>::STRUCT_SCHEMA);
        v
    }
}
