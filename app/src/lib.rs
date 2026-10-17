//! Process-level Hacash applications. [`fullnode`] owns the standard node
//! process; [`worker`] contains the independent mining clients.

mod apps;
pub mod fullnode;
mod launch;
pub mod registry;
pub mod version;
pub mod worker;

pub use fullnode::{Fullnode, run, run_with_scaner};
pub use registry::{Registry, standard_registry, standard_registry_with_params};
pub use version::{DB_VERSION, HACASH_NODE_BUILD_TIME, HACASH_NODE_VERSION};
