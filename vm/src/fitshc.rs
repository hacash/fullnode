//! Minimal FitSH entry: IR→bytecode helpers are available now; the full FitSH source
//! compiler is staged under `fitsh_port/` (see README there).

pub use crate::ir::{
    convert_ir_to_bytecode, convert_ir_to_runtime_bytecode, runtime_irs_to_exec_bytecodes,
};
