//! Generic chain HTTP API services (status / chain / pool / account). Three layers:
//! `server` (transport only) <- `api` (generic, depends only on `base`+`field`+`sys`) <- `mint::api`/`vm::api` (domain routes); `app` assembles all three.

mod account;
mod chain;
mod contract_storage;
mod pool;
mod status;
mod util;

pub use account::AccountApi;
pub use chain::ChainApi;
pub use contract_storage::ContractStorageApi;
pub use pool::PoolApi;
pub use status::StatusApi;
