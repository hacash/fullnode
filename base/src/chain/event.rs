use field::Hash;
use sys::Rerr;

use crate::chain::PkgOrigin;

/// Observational-only notifications: run after the chain transition, cannot reject
/// a block. `Err` only warns; `Abort` is escalated to engine-fatal (§8.4).
pub trait ChainListener: Send + Sync {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) -> Rerr {
        Ok(())
    }

    fn on_stable_block(&self, _height: u64, _hash: Hash, _origin: PkgOrigin) -> Rerr {
        Ok(())
    }
}
