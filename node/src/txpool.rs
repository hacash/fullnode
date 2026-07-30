use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use base::{
    TxGroupId, TxOrdering, TxPkg, TxPool, TxPoolGroupSpec, TxPoolInsertOutcome, TxPoolInsertReject,
};
use field::Hash;
use sys::Rerr;

struct TxGroup {
    max_size: usize,
    txs: Vec<TxPkg>,
    ordering: TxOrdering,
}

impl TxGroup {
    fn new(max_size: usize, ordering: TxOrdering) -> Self {
        Self {
            max_size,
            txs: Vec::new(),
            ordering,
        }
    }

    fn better_than(&self, left: &TxPkg, right: &TxPkg) -> bool {
        match self.ordering {
            TxOrdering::FeePurity => left.fee_purity() > right.fee_purity(),
            TxOrdering::Fee => left.tx().fee() > right.tx().fee(),
            TxOrdering::Fifo => false,
        }
    }

    fn not_better_than(&self, left: &TxPkg, right: &TxPkg) -> bool {
        !self.better_than(left, right)
    }

    fn find_index(&self, hash: &Hash) -> Option<usize> {
        self.txs.iter().position(|tx| tx.hash() == *hash)
    }

    fn insert(&mut self, tx: TxPkg) -> sys::Ret<TxPoolInsertOutcome> {
        if let Some(idx) = self.find_index(&tx.hash()) {
            if self.not_better_than(&tx, &self.txs[idx]) {
                return Ok(TxPoolInsertOutcome::NotStored(
                    TxPoolInsertReject::UnderpricedReplacement,
                ));
            }
            self.txs.remove(idx);
        }
        if self.max_size > 0 && self.txs.len() >= self.max_size {
            if let Some(tail) = self.txs.last() {
                if self.not_better_than(&tx, tail) {
                    return Ok(TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity));
                }
            }
        }
        let pos = match self.ordering {
            TxOrdering::Fifo => self.txs.len(),
            _ => self
                .txs
                .iter()
                .position(|have| self.better_than(&tx, have))
                .unwrap_or(self.txs.len()),
        };
        self.txs.insert(pos, tx);
        if self.max_size > 0 && self.txs.len() > self.max_size {
            self.txs.pop();
        }
        Ok(TxPoolInsertOutcome::Stored)
    }
}

pub struct MemTxPool {
    lowest_fee_purity: u64,
    groups: BTreeMap<TxGroupId, Mutex<TxGroup>>,
}

impl MemTxPool {
    pub fn new() -> Self {
        Self::with_groups(
            0,
            vec![TxPoolGroupSpec::new(
                TxGroupId::DEFAULT,
                "default",
                TxOrdering::FeePurity,
            )],
        )
    }

    pub fn with_groups(lowest_fee_purity: u64, specs: Vec<TxPoolGroupSpec>) -> Self {
        assert!(!specs.is_empty(), "invalid txpool groups");
        let spec_count = specs.len();
        let groups: BTreeMap<_, _> = specs
            .into_iter()
            .map(|spec| {
                (
                    spec.id,
                    Mutex::new(TxGroup::new(spec.default_capacity, spec.ordering)),
                )
            })
            .collect();
        assert_eq!(groups.len(), spec_count, "duplicate txpool group id");
        Self {
            lowest_fee_purity,
            groups,
        }
    }

    fn group(&self, group: TxGroupId) -> sys::Ret<&Mutex<TxGroup>> {
        self.groups
            .get(&group)
            .ok_or_else(|| sys::Error::fault("tx pool group overflow"))
    }
}

impl TxPool for MemTxPool {
    fn min_fee_purity(&self) -> u64 {
        self.lowest_fee_purity
    }

    fn group_ids(&self) -> Vec<TxGroupId> {
        self.groups.keys().copied().collect()
    }
    fn count(&self, group: TxGroupId) -> usize {
        self.groups
            .get(&group)
            .map_or(0, |g| g.lock().unwrap().txs.len())
    }
    fn first(&self, group: TxGroupId) -> Option<TxPkg> {
        self.groups
            .get(&group)
            .and_then(|g| g.lock().unwrap().txs.first().cloned())
    }
    fn iter(&self, group: TxGroupId, f: &mut dyn FnMut(&TxPkg) -> bool) -> Rerr {
        let group = self.group(group)?.lock().unwrap();
        for tx in &group.txs {
            if !f(tx) {
                break;
            }
        }
        Ok(())
    }
    fn insert(&self, group: TxGroupId, tx: TxPkg) -> sys::Ret<TxPoolInsertOutcome> {
        if tx.fee_purity() < self.lowest_fee_purity {
            return sys::errf!("tx fee purity {} too low to add txpool", tx.fee_purity());
        }
        self.group(group)?.lock().unwrap().insert(tx)
    }
    fn find(&self, hash: &[u8]) -> Option<TxPkg> {
        for group in self.groups.values() {
            let group = group.lock().unwrap();
            if let Some(p) = group.txs.iter().find(|p| p.hash().as_bytes() == hash) {
                return Some(p.clone());
            }
        }
        None
    }
    fn take(&self, group: TxGroupId, max: usize) -> Vec<TxPkg> {
        self.groups
            .get(&group)
            .map(|g| g.lock().unwrap().txs.iter().take(max).cloned().collect())
            .unwrap_or_default()
    }
    fn remove(&self, group: TxGroupId, hashes: &[Hash]) -> Rerr {
        let mut group = self.group(group)?.lock().unwrap();
        group.txs.retain(|p| !hashes.contains(&p.hash()));
        Ok(())
    }
    fn clear(&self, group: TxGroupId) -> Rerr {
        self.group(group)?.lock().unwrap().txs.clear();
        Ok(())
    }
    fn retain(&self, group: TxGroupId, keep: &mut dyn FnMut(&TxPkg) -> bool) -> Rerr {
        self.group(group)?.lock().unwrap().txs.retain(|tx| keep(tx));
        Ok(())
    }
    fn drain(&self, hashes: &[Hash]) -> Vec<TxPkg> {
        let mut out = Vec::new();
        let mut target: HashSet<Hash> = HashSet::from_iter(hashes.iter().copied());
        for group in self.groups.values() {
            let mut group = group.lock().unwrap();
            group.txs.retain(|p| {
                if target.remove(&p.hash()) {
                    out.push(p.clone());
                    false
                } else {
                    true
                }
            });
        }
        out
    }
    fn print(&self) -> String {
        let groups = self
            .groups
            .iter()
            .filter_map(|(id, group)| {
                group
                    .try_lock()
                    .ok()
                    .map(|group| format!("{}({})", id, group.txs.len()))
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("[TxPool] tx count: {}", groups)
    }
}

#[cfg(test)]
mod tests {
    use super::MemTxPool;
    use base::{
        Context, PkgOrigin, PkgSource, Transaction, TxGroupId, TxOrdering, TxPkg, TxPool,
        TxPoolGroupSpec, TxPoolInsertOutcome, TxPoolInsertReject,
    };
    use field::{Address, Amount, Encode, Hash};
    use std::any::Any;
    use std::sync::Arc;

    #[derive(Debug)]
    struct TestTx(Hash, u64);

    impl Encode for TestTx {
        fn size(&self) -> usize {
            32
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(self.0.as_bytes());
        }
    }

    impl Transaction for TestTx {
        fn ty(&self) -> u8 {
            2
        }

        fn hash(&self) -> Hash {
            self.0
        }

        fn main(&self) -> Address {
            Address::default()
        }

        fn fee(&self) -> &Amount {
            Amount::zero_ref()
        }

        fn fee_purity(&self) -> u64 {
            self.1
        }

        fn verify_signature(&self) -> sys::Rerr {
            Ok(())
        }

        fn execute(&self, _ctx: &mut dyn Context) -> sys::Rerr {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn test_pkg(hash_byte: u8, fee_purity: u64) -> TxPkg {
        let hash = Hash::from([hash_byte; 32]);
        TxPkg::from_transaction(
            Arc::new(TestTx(hash, fee_purity)),
            PkgSource::new(PkgOrigin::Broadcast),
        )
    }

    #[test]
    fn supports_three_non_contiguous_groups() {
        let specs = [
            (0, TxOrdering::FeePurity),
            (7, TxOrdering::Fee),
            (42, TxOrdering::Fifo),
        ]
        .into_iter()
        .map(|(id, ordering)| TxPoolGroupSpec::new(TxGroupId::new(id), id.to_string(), ordering))
        .collect();
        let pool = MemTxPool::with_groups(0, specs);
        assert_eq!(
            pool.group_ids(),
            vec![TxGroupId::new(0), TxGroupId::new(7), TxGroupId::new(42)]
        );
        assert!(pool.clear(TxGroupId::new(1)).is_err());
    }

    #[test]
    fn full_pool_reports_not_stored_without_mutating_contents() {
        let mut spec = TxPoolGroupSpec::new(TxGroupId::DEFAULT, "normal", TxOrdering::FeePurity);
        spec.default_capacity = 1;
        let pool = MemTxPool::with_groups(0, vec![spec]);

        assert_eq!(
            pool.insert(TxGroupId::DEFAULT, test_pkg(1, 100)).unwrap(),
            TxPoolInsertOutcome::Stored
        );
        assert_eq!(
            pool.insert(TxGroupId::DEFAULT, test_pkg(2, 50)).unwrap(),
            TxPoolInsertOutcome::NotStored(TxPoolInsertReject::Capacity)
        );
        assert_eq!(pool.count(TxGroupId::DEFAULT), 1);
        assert!(pool.find(Hash::from([1; 32]).as_bytes()).is_some());
    }

    #[test]
    fn underpriced_replacement_reports_not_stored() {
        let pool = MemTxPool::with_groups(
            0,
            vec![TxPoolGroupSpec::new(
                TxGroupId::DEFAULT,
                "normal",
                TxOrdering::FeePurity,
            )],
        );

        assert_eq!(
            pool.insert(TxGroupId::DEFAULT, test_pkg(1, 100)).unwrap(),
            TxPoolInsertOutcome::Stored
        );
        assert_eq!(
            pool.insert(TxGroupId::DEFAULT, test_pkg(1, 50)).unwrap(),
            TxPoolInsertOutcome::NotStored(TxPoolInsertReject::UnderpricedReplacement)
        );
        assert_eq!(pool.count(TxGroupId::DEFAULT), 1);
        assert_eq!(pool.first(TxGroupId::DEFAULT).unwrap().fee_purity(), 100);
    }
}
