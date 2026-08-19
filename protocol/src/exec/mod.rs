//! Execution: ContextInst, gas billing, tex settlement.
//!
//! Only `ContextInst` is execute-view-bound (it drives `ActionDispatcher`, which
//! dispatches through the `ActionRef` execute view), so it compiles only under
//! the `execute` feature. `gas` and `tex` are pure settlement logic over the
//! unconditional `Context`/`CoreState` surface: they are always compiled and
//! dead-code-eliminated from the SDK/wasm artifact.

#[cfg(feature = "execute")]
pub mod context;
pub(crate) mod gas;
pub(crate) mod tex;
