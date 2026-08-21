//! External indexer integration for the standard full node. Indexers implement
//! [`base::Scaner`] and pass the instance to [`super::Fullnode::open`] — the only concrete indexer.

use std::sync::Arc;

use base::{ApiService, BlockHistory, ChainListener, CoreStateRead, Engine, Scaner, ScanerView};
use field::{Address, Balance, Hash};
use sys::{Rerr, Ret, Waiter};

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

    fn balance_at(&self, block_hash: &Hash, address: &Address) -> Ret<Option<Balance>> {
        Ok(self
            .balances_at(block_hash, &[*address])?
            .and_then(|list| list.into_iter().next().flatten()))
    }

    fn balances_at(
        &self,
        block_hash: &Hash,
        addresses: &[Address],
    ) -> Ret<Option<Vec<Option<Balance>>>> {
        const MAX_SNAPSHOT_ATTEMPTS: usize = 3;
        for _ in 0..MAX_SNAPSHOT_ATTEMPTS {
            // `Ok(None)` (branch tip not in the tree) retries; a query failure
            // (`Err`) propagates instead of being flattened away (§5).
            let session = match self.engine.state_at_session(block_hash) {
                Ok(Some(session)) => session,
                Ok(None) => {
                    std::thread::yield_now();
                    continue;
                }
                Err(e) => return Err(e),
            };
            let state = CoreStateRead::wrap(session.view());
            let mut balances = Vec::with_capacity(addresses.len());
            for address in addresses {
                balances.push(state.balance(address)?);
            }
            if self.engine.validate_state_view(&session.tip_hash()) {
                return Ok(Some(balances));
            }
            std::thread::yield_now();
        }
        Ok(None)
    }
}

/// Forwards chain events to [`Scaner`] without the engine holding a scaner.
struct ScanerListener {
    scaner: Arc<dyn Scaner>,
    view: Arc<dyn ScanerView>,
}

impl ChainListener for ScanerListener {
    fn on_stable_block(&self, height: u64, _hash: Hash, origin: base::PkgOrigin) -> Rerr {
        if matches!(origin, base::PkgOrigin::Rebuild | base::PkgOrigin::Replay) {
            return Ok(());
        }
        // A read failure at a stable height is returned so the listener boundary
        // records it (and escalates an `Abort`); a confirmed-missing block skips this.
        let block = match self.view.block_history().block_at_height(height) {
            Ok(Some(block)) => block,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };
        self.scaner.on_block(block, self.view.clone())
    }
}

/// Attach a concrete indexer: register a [`ChainListener`], extend `services`
/// with `scaner.api_services()`, and enqueue a checkpoint-based catch-up.
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
