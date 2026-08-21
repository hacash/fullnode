//! Optional block-index / scaner extension. The engine never holds a scaner — indexing
//! hooks into `ChainListener` + `on_stable_block`; assembly lives only in `app`.

use std::sync::Arc;

use field::{Address, Balance, Hash};
use sys::{Rerr, Ret, Waiter};

use crate::BlockRef;
use crate::api::ApiService;
use crate::chain::BlockHistory;

/// Read-only chain data exposed to a block explorer. Deliberately excludes transaction
/// execution, peers, storage handles, and generic state-KV access.
pub trait ScanerView: Send + Sync {
    fn block_history(&self) -> Arc<dyn BlockHistory>;
    fn balance_at(&self, block_hash: &Hash, address: &Address) -> Ret<Option<Balance>>;

    /// Read several balances from one validated state snapshot. Outer `Option` = snapshot
    /// availability, inner `Option` = per-address balance entry. Prefer over repeated `balance_at` calls.
    fn balances_at(
        &self,
        _block_hash: &Hash,
        _addresses: &[Address],
    ) -> Ret<Option<Vec<Option<Balance>>>> {
        Ok(None)
    }
}

/// Block-index / scan extension point. Prefer state via [`ChainView`] / store over raw
/// `DiskDB` handles (keeps `chain` free of indexer types).
pub trait Scaner: Send + Sync {
    fn name(&self) -> &str;

    /// Enqueue historical blocks after the indexer's local checkpoint.
    fn sync(&self, _view: Arc<dyn ScanerView>) -> Rerr {
        Ok(())
    }

    /// A stable block is available for indexing. Implementations must return promptly and do
    /// DB work in their own queue; failures must not reject the block (errors are recorded).
    fn on_block(&self, _block: BlockRef, _view: Arc<dyn ScanerView>) -> Rerr {
        Ok(())
    }

    /// Extra HTTP routes merged by `app` into the main `HttpServer` service list.
    fn api_services(&self) -> Vec<Arc<dyn ApiService>> {
        vec![]
    }

    /// Background work. Default no-op. May spawn threads or schedule on a shared tokio runtime;
    /// must honour `waiter` for shutdown (no separate `serve` OS thread).
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
