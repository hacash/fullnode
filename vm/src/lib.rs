//! Hacash VM wire types, contract actions, execution engine, and tooling. Codec-only consumers
//! compile without the `execute` feature; fullnodes install the native VM via `register_exec`.

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
#[cfg(feature = "execute")]
pub mod api;
#[macro_use]
#[allow(dead_code)] // Shared wire types retain execution helpers in codec-only builds.
pub(crate) mod rt;
#[allow(dead_code)] // Contract editing helpers are consumed only by selected VM entry paths.
pub(crate) mod contract;
#[cfg(feature = "execute")]
pub mod fitshc;
// Execution engine: compiled only with `execute`. Codec stays on
// `action` / `contract` / `rt` (wire types) / `value` (`ContractAddress`).
#[cfg(feature = "execute")]
#[allow(dead_code)] // Internal frame API also backs the optional compiler/tooling surface.
pub(crate) mod frame;
#[cfg(feature = "execute")]
#[allow(dead_code)] // Instruction helpers intentionally expose the complete VM opcode surface.
pub(crate) mod interpreter;
#[cfg(feature = "execute")]
#[allow(dead_code)] // IR builders are re-exported selectively through `fitshc`.
pub(crate) mod ir;
#[cfg(feature = "execute")]
#[allow(dead_code)] // The VM service owns lifecycle methods not all used by the fullnode path.
pub(crate) mod machine;
#[cfg(feature = "execute")]
#[allow(dead_code)] // Native opcode catalog includes compiler-visible operations.
pub(crate) mod native;
#[cfg(feature = "execute")]
mod setup;
#[cfg(feature = "execute")]
#[allow(dead_code)] // Space containers expose reset/introspection hooks for VM tooling.
pub(crate) mod space;
#[cfg(feature = "execute")]
#[allow(dead_code)] // State debug and lifecycle hooks are not all used in production assembly.
pub(crate) mod state;
#[allow(dead_code)] // Value conversion helpers are execution-only in codec-only builds.
pub(crate) mod value;
mod wire;

#[cfg(feature = "execute")]
pub use machine::peek_vm_runtime_limits;
#[cfg(feature = "execute")]
pub use setup::register_exec;
#[cfg(feature = "execute")]
pub use state::{StorageDebug, VMState, VMStateRead, VmLog};
pub use value::ContractAddress;
pub use wire::{ACTION_CODECS, STRUCT_SCHEMAS, register_wire};

pub const MAX_FUNC_PARAM_LEN: usize = 15;
