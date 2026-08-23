//! FitSH source compiler frontend: `.fitsh` source → contract tree + deploy
//! options + source maps. Ported from fullnodedev `vm/src/fitshc` (synced
//! at fullnodedev 4de8acc); the IR→bytecode helpers below keep the pre-port
//! `vm::fitshc` tooling surface (IR payload compilation without source).

pub mod compile_body;
pub mod compiler;
pub mod parse_deploy;
pub mod parse_func;
pub mod parse_top;
pub mod state;

pub use compile_body::{CompiledCode, compile_body};
pub use compiler::{compile, compile_with_warnings};

pub use crate::ir::{
    convert_ir_to_bytecode, convert_ir_to_runtime_bytecode, runtime_irs_to_exec_bytecodes,
};
