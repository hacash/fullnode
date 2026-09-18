use std::collections::BTreeMap;

use base::{MemDB, MemKV, StateLayer, StateRead};
use sys::Ret;

/// Isolated in-memory state layer for unit and VM tests.
#[derive(Clone, Default, Debug)]
pub struct FlatMemState {
    values: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl FlatMemState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_mem(mem: MemKV) -> Self {
        let mut state = Self::new();
        mem.for_each(&mut |key, value| match value {
            Some(value) => {
                state.values.insert(key.to_vec(), value.to_vec());
            }
            None => {
                state.values.remove(key);
            }
        });
        state
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn entries(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
    pub fn snapshot(&self) -> Self {
        self.clone()
    }
    pub fn restore(&mut self, snapshot: Self) {
        *self = snapshot;
    }
    pub fn into_mem(self) -> MemKV {
        let mut mem = MemKV::new();
        for (key, value) in self.values {
            mem.put(key, value);
        }
        mem
    }
}

impl StateRead for FlatMemState {
    fn get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        Ok(self.values.get(key).cloned())
    }
}

impl StateLayer for FlatMemState {
    fn set(&mut self, key: &[u8], value: Vec<u8>) {
        self.values.insert(key.to_vec(), value);
    }
    fn del(&mut self, key: &[u8]) {
        self.values.remove(key);
    }
}

/// Compatibility alias: the 1.0 state model owns forks in `StateChunk`; an
/// independent test state remains useful for direct unit tests.
pub type ForkableMemState = FlatMemState;
