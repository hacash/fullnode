//! Cross-crate behavior contracts.
//!
//! Implementations and domain data remain in their owning modules; this module
//! is the single home for the traits that connect them.

pub(crate) mod action;
pub(crate) mod block;
pub(crate) mod context;
pub(crate) mod schema;
pub(crate) mod state;
pub(crate) mod store;
pub(crate) mod transaction;
pub(crate) mod vm;

pub use action::*;
pub use block::*;
pub use context::*;
pub use schema::*;
pub use transaction::*;
pub use vm::*;
