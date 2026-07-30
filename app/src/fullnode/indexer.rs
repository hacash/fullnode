//! External indexer integration for the standard full node.
//!
//! # Wiring (only place that knows a concrete indexer)
//!
//! ```ignore
//! let scaner: Arc<dyn Scaner> = Arc::new(NilScaner); // or hascan::open(...)
//! let node = app::Fullnode::open(
//!     std::path::Path::new("hacash.config.ini"),
//!     Some(scaner),
//! )?;
//! node.run()?;
//! ```
//!
//! External indexers implement [`base::Scaner`] in their own crate and pass the
//! instance to [`super::Fullnode::open`]. Engine stays unaware.

use std::sync::Arc;

use base::{ApiService, BlockHistory, ChainListener, CoreStateRead, Engine, Scaner, ScanerView};
use field::{Address, Balance, Hash};
use sys::{Rerr, Waiter};

/// Result of attaching a scaner: keep the handle to `start`.
pub(super) struct AttachedIndexer {
    scaner: Arc<dyn Scaner>,
    view: Arc<dyn ScanerView>,
}

impl AttachedIndexer {
    pub(super) fn name(&self) -> &str {
        self.scaner.name()
    }

    pub(super) fn start(&self, waiter: Waiter) -> Rerr {
        self.scaner.start(waiter)?;
        self.scaner.sync(self.view.clone())?;
        Ok(())
    }
}

/// Keeps the concrete engine behind the explorer's minimal read-only view.
struct EngineScanerView {
    engine: Arc<dyn Engine>,
}

impl ScanerView for EngineScanerView {
    fn block_history(&self) -> Arc<dyn BlockHistory> {
        self.engine.block_history()
    }

    fn balance_at(&self, block_hash: &Hash, address: &Address) -> Option<Balance> {
        self.balances_at(block_hash, &[*address])?
            .into_iter()
            .next()
            .flatten()
    }

    fn balances_at(
        &self,
        block_hash: &Hash,
        addresses: &[Address],
    ) -> Option<Vec<Option<Balance>>> {
        const MAX_SNAPSHOT_ATTEMPTS: usize = 3;
        for _ in 0..MAX_SNAPSHOT_ATTEMPTS {
            let Some(session) = self.engine.state_at_session(block_hash) else {
                std::thread::yield_now();
                continue;
            };
            let state = CoreStateRead::wrap(session.view());
            let balances = addresses
                .iter()
                .map(|address| state.balance(address))
                .collect();
            if self.engine.validate_state_view(&session.tip_hash()) {
                return Some(balances);
            }
            std::thread::yield_now();
        }
        None
    }
}

/// Forwards chain events to [`Scaner`] without the engine holding a scaner.
struct ScanerListener {
    scaner: Arc<dyn Scaner>,
    view: Arc<dyn ScanerView>,
}

impl ChainListener for ScanerListener {
    fn on_stable_block(&self, height: u64, _hash: Hash, origin: base::PkgOrigin) {
        if matches!(origin, base::PkgOrigin::Rebuild | base::PkgOrigin::Replay) {
            return;
        }
        let Some(block) = self.view.block_history().block_at_height(height) else {
            return;
        };
        self.scaner.on_block(block, self.view.clone());
    }
}

/// Attach a concrete indexer.
///
/// - registers a [`ChainListener`]
/// - extends `services` with `scaner.api_services()`
/// - asks the scaner to enqueue a checkpoint-based catch-up
///
pub(super) fn attach(
    engine: Arc<dyn Engine>,
    scaner: Arc<dyn Scaner>,
    services: &mut Vec<Arc<dyn ApiService>>,
) -> RetAttached {
    services.extend(scaner.api_services());
    let view: Arc<dyn ScanerView> = Arc::new(EngineScanerView {
        engine: engine.clone(),
    });
    engine.add_chain_listener(Arc::new(ScanerListener {
        scaner: scaner.clone(),
        view: view.clone(),
    }))?;
    Ok(AttachedIndexer { scaner, view })
}

type RetAttached = sys::Ret<AttachedIndexer>;
