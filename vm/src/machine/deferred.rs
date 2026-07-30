use std::collections::HashSet;

use crate::rt::{IntentScope, ItrErr, ItrErrCode, VmrtErr};
use crate::value::ContractAddress;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeferredEntry {
    pub addr: ContractAddress,
    pub intent_scope: IntentScope,
}

#[derive(Clone, Debug)]
pub struct DeferredRegistry {
    defer_auth: Option<ContractAddress>,
    entries: Vec<DeferredEntry>,
    seen: HashSet<DeferredEntry>,
}

impl Default for DeferredRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DeferredRegistry {
    pub fn new() -> Self {
        Self {
            defer_auth: None,
            entries: Vec::new(),
            seen: HashSet::new(),
        }
    }

    pub fn clear(&mut self) {
        self.defer_auth = None;
        self.entries.clear();
        self.seen.clear();
    }

    pub fn replace_defer_auth(&mut self, auth: Option<ContractAddress>) -> Option<ContractAddress> {
        std::mem::replace(&mut self.defer_auth, auth)
    }

    pub fn register_current(
        &mut self,
        caller: &ContractAddress,
        intent_scope: IntentScope,
    ) -> VmrtErr {
        if self.defer_auth.as_ref() != Some(caller) {
            return itr_err_fmt!(
                ItrErrCode::DeferredError,
                "defer can only be registered from Permit*/Payable* abst entries (defer_auth mismatch)"
            );
        }
        let entry = DeferredEntry {
            addr: *caller,
            intent_scope,
        };
        if !self.seen.insert(entry.clone()) {
            return itr_err_fmt!(
                ItrErrCode::DeferredError,
                "duplicate deferred cleanup hook registration"
            );
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn drain_lifo(&mut self) -> Vec<DeferredEntry> {
        self.entries.drain(..).rev().collect()
    }
}

pub type DeferCallbacks = DeferredRegistry;
