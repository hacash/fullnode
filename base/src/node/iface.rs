use std::sync::Arc;

use sys::{Rerr, Ret, Waiter};

use crate::chain::{BlkPkg, TxPkg};
use crate::chain::{ChainListener, Engine};
use crate::node::{TxAdmissionStatus, TxPool, TxRejectReason, TxSubmitResult};

pub trait Node: Send + Sync {
    fn start(&self, waiter: Waiter) -> Rerr;

    fn admit_transaction(
        &self,
        _tx: &TxPkg,
        _is_async: bool,
        _only_pool: bool,
    ) -> Ret<TxSubmitResult> {
        Ok(TxSubmitResult::rejected(
            _tx.hash(),
            TxRejectReason::Policy("transaction admission not supported".to_owned()),
        ))
    }

    fn submit_transaction(&self, tx: &TxPkg, is_async: bool, only_pool: bool) -> Rerr {
        let result = self.admit_transaction(tx, is_async, only_pool)?;
        match result.status {
            TxAdmissionStatus::AcceptedPool
            | TxAdmissionStatus::AcceptedBroadcast
            | TxAdmissionStatus::Duplicate
            | TxAdmissionStatus::Replaced
            | TxAdmissionStatus::Ignored => Ok(()),
            TxAdmissionStatus::Rejected => sys::errf!("tx rejected: {:?}", result.reason),
        }
    }
    fn submit_block(&self, _blk: &BlkPkg, _is_async: bool) -> Rerr {
        Ok(())
    }

    fn engine(&self) -> Arc<dyn Engine>;
    fn txpool(&self) -> Arc<dyn TxPool>;

    fn add_chain_listener(&self, _listener: Arc<dyn ChainListener>) -> Rerr {
        sys::errf!("chain listener registration not supported")
    }

    fn all_peer_prints(&self) -> Vec<String> {
        vec![]
    }
    fn stop(&self) {}
}
