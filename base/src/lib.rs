//! `base` —— trait crate for Context / Action / Transaction / Block / State /
//! Vm / Engine / ForkTree / Store / Node / Server.
//!
//! # Module map
//!
//! - `runtime/`    execution context, action dispatch, gas and scopes
//! - `ledger/`     shared Hacash ledger schema and state transitions
//! - `registry/`   codec registration, context factories and VM metadata
//! - `state/`      StateRead / StateLayer / StateChunk
//! - `store/`      BlockStore / DiskDB / Store
//! - `sync/`       pipeline source, handle, progress and stream contracts
//! - `chain/`      packages, apply modes, consensus and chain runtime contracts
//! - `node/`       TxPool / Peer / Node
//! - `api/`        ApiRoute / ApiService / Server / ApiExecCtx
//! - `scaner/`     optional Scaner / NilScaner (indexer extension; not held by Engine)
//!
//! Domain paths and flat `base::Foo` re-exports are both supported.

pub mod api;
pub mod chain;
pub mod iface;
pub mod ledger;
pub mod node;
pub mod registry;
pub mod runtime;
pub mod scaner;
pub mod state;
pub mod store;
pub mod sync;

pub use action_codec_derive::ActionCodec;
pub use api::*;
pub use chain::*;
pub use iface::*;
pub use ledger::*;
pub use node::*;
pub use registry::*;
pub use runtime::*;
pub use scaner::*;
pub use state::*;
pub use store::*;
pub use sync::*;
