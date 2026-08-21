//! Standard Hacash codecs, state helpers and Registry setup. Depends only on
//! sys → field → base; fullnode roots install its wire/execution catalogs, the SDK selects a wallet-reachable subset.

pub(crate) mod codec;
#[cfg(feature = "execute")]
pub(crate) mod exec;
pub(crate) mod facts;
pub(crate) mod level;
pub(crate) mod params;
#[cfg(feature = "execute")]
pub(crate) mod setup;
mod wire;

// ---- public nested path aliases (external crates keep using these) ----
pub use codec::action as action_std;
pub use codec::block as block_std;
pub use codec::tx as tx_std;

// ---- crate-root re-exports ----
pub use facts::{ScheduleFacts, schedule_facts};
pub use level::{TopologyFacts, topology_facts};
#[cfg(feature = "execute")]
pub use params::execution_params;
#[cfg(feature = "execute")]
pub use setup::register_exec;
pub use wire::{ACTION_CODECS, STRUCT_SCHEMAS, TX_CODECS, register_wire};
