use field::Hash;

use crate::chain::PkgOrigin;

/// Notifications are observational only. They run after the relevant chain
/// transition has completed and cannot reject or roll back a block.
pub trait ChainListener: Send + Sync {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) {}

    fn on_stable_block(&self, _height: u64, _hash: Hash, _origin: PkgOrigin) {}
}
