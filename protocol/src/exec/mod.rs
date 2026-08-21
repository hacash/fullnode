//! Execution: ContextInst, gas billing, tex settlement, action/tx execute bodies.
//! Compiled only when the crate's `execute` feature is on.

pub(crate) mod action;
pub mod context;
pub(crate) mod gas;
pub(crate) mod tex;
pub(crate) mod tx;
