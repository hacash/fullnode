use field::Hash;
use sys::Rerr;

use crate::chain::PkgOrigin;

/// Notifications are observational only. They run after the relevant chain
/// transition has completed and cannot reject or roll back a block.
///
/// An ordinary `Err` only warns and does not affect the accepted block; an
/// `Abort` is escalated to engine fatal after all listeners have been notified
/// (§8.4 of the state-read error contract).
pub trait ChainListener: Send + Sync {
    fn on_block_accepted(&self, _height: u64, _origin: PkgOrigin) -> Rerr {
        Ok(())
    }

    fn on_stable_block(&self, _height: u64, _hash: Hash, _origin: PkgOrigin) -> Rerr {
        Ok(())
    }
}
