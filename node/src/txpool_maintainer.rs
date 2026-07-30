//!

use std::sync::Arc;

use base::{ChainListener, Engine, TxGroupId, TxPkg, TxPool};
use sys::Rerr;

const TXPOOL_STATUS_PRINT_BLOCK_INTERVAL: u64 = 15;

fn should_print_txpool_status(height: u64, origin: base::PkgOrigin) -> bool {
    !matches!(
        origin,
        base::PkgOrigin::Rebuild | base::PkgOrigin::Replay | base::PkgOrigin::Sync
    ) && height % TXPOOL_STATUS_PRINT_BLOCK_INTERVAL == 0
}

pub struct TxPoolMaintainer {
    engine: Arc<dyn Engine>,
    txpool: Arc<dyn TxPool>,
}

impl TxPoolMaintainer {
    pub fn new(engine: Arc<dyn Engine>, txpool: Arc<dyn TxPool>) -> Self {
        Self { engine, txpool }
    }

    fn clean_invalid_group(&self, group: TxGroupId, height: u64) -> Rerr {
        let mut txs = Vec::new();
        self.txpool.iter(group, &mut |tx| {
            txs.push(tx.clone());
            true
        })?;
        if txs.is_empty() {
            return Ok(());
        }

        // Evaluate the pool in one cumulative child state. This preserves
        // valid dependent transactions and lets the engine stop at an
        // uncertain Type3+ failure, matching dev's fork_sub_state loop.
        let failed = self.engine.try_execute_batch(
            txs.iter().map(TxPkg::tx_ref).collect(),
            height.saturating_add(1),
        );
        if !failed.is_empty() {
            self.txpool.remove(group, &failed)?;
        }
        Ok(())
    }
}

impl ChainListener for TxPoolMaintainer {
    fn on_block_accepted(&self, height: u64, origin: base::PkgOrigin) {
        if matches!(origin, base::PkgOrigin::Rebuild | base::PkgOrigin::Replay) {
            return;
        }
        if should_print_txpool_status(height, origin) {
            println!("{}.", self.txpool.print());
        }
        for spec in self.engine.tx_policy().tx_pool_groups() {
            if spec
                .revalidate_interval
                .is_some_and(|interval| interval > 0 && height % interval == 0)
            {
                let _ = self.clean_invalid_group(spec.id, height);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use base::PkgOrigin;

    use super::should_print_txpool_status;

    #[test]
    fn status_print_cadence_matches_fullnodedev() {
        assert!(!should_print_txpool_status(14, PkgOrigin::Broadcast));
        assert!(should_print_txpool_status(15, PkgOrigin::Broadcast));
        assert!(!should_print_txpool_status(16, PkgOrigin::Broadcast));
        assert!(should_print_txpool_status(30, PkgOrigin::Mining));
        assert!(!should_print_txpool_status(15, PkgOrigin::Sync));
        assert!(!should_print_txpool_status(15, PkgOrigin::Replay));
        assert!(!should_print_txpool_status(15, PkgOrigin::Rebuild));
    }
}
