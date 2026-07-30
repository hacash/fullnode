//! Optional block-index / scaner extension.
//!
//! # Design (vs fullnodedev)
//!
//! | fullnodedev | fullnodenext |
//! |-------------|--------------|
//! | `ChainEngine` holds `Arc<dyn Scaner>` | Engine never holds a scaner |
//! | `scaner.roll(block, state, disk)` from insert path | `ChainListener` + `on_stable_block` |
//! | `start` / `serve` each own an OS thread | `start(Waiter)` on shared shutdown; HTTP via `api_services` |
//! | `GLOBAL_API_SERVICES` OnceLock | App merges `api_services()` into `HttpServer::open` |
//!
//! Assembly lives only in `app` (or an external indexer crate that `app` wires).
//! Concrete indexers (hascan, …) implement [`Scaner`] and optionally [`ApiService`].
//!
//! Configuration and concrete assembly belong to the application crate.

use std::sync::Arc;

use field::{Address, Balance, Hash};
use sys::{Rerr, Waiter};

use crate::BlockRef;
use crate::api::ApiService;
use crate::chain::BlockHistory;

/// Read-only chain data exposed to a block explorer.
///
/// This deliberately excludes transaction execution, peers, storage handles,
/// and generic state-KV access.
pub trait ScanerView: Send + Sync {
    fn block_history(&self) -> Arc<dyn BlockHistory>;
    fn balance_at(&self, block_hash: &Hash, address: &Address) -> Option<Balance>;

    /// Read several balances from one validated state snapshot.
    ///
    /// The outer `Option` reports whether the requested snapshot is available;
    /// each inner `Option` reports whether the corresponding address has a
    /// balance entry. Indexers should prefer this over repeated `balance_at`
    /// calls so a root move cannot mix results from different snapshots.
    fn balances_at(
        &self,
        _block_hash: &Hash,
        _addresses: &[Address],
    ) -> Option<Vec<Option<Balance>>> {
        None
    }
}

/// Block-index / scan extension point.
///
/// Prefer loading state via [`ChainView`] / store rather than receiving raw
/// `DiskDB` handles (keeps `chain` free of indexer types).
pub trait Scaner: Send + Sync {
    fn name(&self) -> &str;

    /// Enqueue historical blocks after the indexer's local checkpoint.
    fn sync(&self, _view: Arc<dyn ScanerView>) -> Rerr {
        Ok(())
    }

    /// A stable block is available for indexing.
    ///
    /// Implementations must return promptly and do their database work in
    /// their own queue/worker. Indexer failures must not reject a block.
    fn on_block(&self, _block: BlockRef, _view: Arc<dyn ScanerView>) {}

    /// Extra HTTP routes merged by `app` into the main `HttpServer` service list.
    fn api_services(&self) -> Vec<Arc<dyn ApiService>> {
        vec![]
    }

    /// Background work. Default no-op.
    ///
    /// Implementations may spawn threads or schedule on a shared tokio runtime;
    /// they must honour `waiter` for shutdown (no separate `serve` OS thread).
    fn start(&self, _waiter: Waiter) -> Rerr {
        Ok(())
    }
}

/// No-op scaner used when no indexer is linked.
#[derive(Default)]
pub struct NilScaner;

impl Scaner for NilScaner {
    fn name(&self) -> &str {
        "nil"
    }
}
