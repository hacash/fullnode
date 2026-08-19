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
// The execution engine (frame/interpreter/machine/native/state/setup/api) is
// always compiled; SDK/wasm builds never construct the execute view, so this
// code is dead-code-eliminated from the wasm artifact (see the Cargo.toml
// features comments).
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
