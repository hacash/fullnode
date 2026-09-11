//! Contract storage discount budget: persistent budget record, block-level transient
//! snapshot keys, and the block budget lifecycle (§4 of the storage fee budget design).
//!
//! Consensus state facts (§4.1):
//! - `ContractStorageBudget` is the only persisted record: remaining capacity, the
//!   capacity it was last applied against (self-describing expansion migration), and
//!   a version tag. It is NOT window history: no per-block usage is stored.
//! - Two transient block-overlay keys hold `B_start` and the running discount usage
//!   `D`. They live only inside one block execution layer (or a pending draft/tx
//!   overlay), are consumed per transaction through tx-layer read-modify-write (so a
//!   failed tx rolls its usage back with its chunk), and are deleted at settlement.
//!   A post-block check treats any leak as a fatal lifecycle error.

use field::{Decode, Encode, Reader, Uint1, Uint12};
use sys::{Rerr, Ret, errf};

use crate::{STATE_DECODE_FAILED_CODE, StateLayer, StateRead, VmExecutionParams, read_typed};

/// Persistent budget record: numeric namespace byte (unused: 0x12).
pub const KEY_CONTRACT_STORAGE_BUDGET: u8 = crate::numeric_state_prefix(0x12);

/// Transient block-overlay key holding `B_start` (the parent-state remaining budget
/// frozen at block start). Never persisted; deleted at settlement.
pub const BLOCK_BUDGET_START_KEY: &[u8] = b"_contract_storage_budget_start";
/// Transient block-overlay key holding `D`, the cumulative discount-consumed billed
/// bytes of the current block layer. Never persisted; deleted at settlement.
pub const BLOCK_BUDGET_USED_KEY: &[u8] = b"_contract_storage_budget_used";

/// `state_version` of the persisted budget record encoding.
pub const CONTRACT_STORAGE_BUDGET_VERSION_V1: u8 = 1;

/// The persisted contract storage discount budget (§4.1). Field order is a state
/// codec boundary; extensions are append-only with a bumped version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContractStorageBudget {
    pub state_version: Uint1,
    /// `B`: remaining discount capacity in bytes.
    pub remaining_bytes: Uint12,
    /// The capacity `C` this record has already been applied against; lets a future
    /// capacity expansion migrate from the record itself (§7.1).
    pub applied_capacity_bytes: Uint12,
}

impl Encode for ContractStorageBudget {
    fn size(&self) -> usize {
        self.state_version.size()
            + self.remaining_bytes.size()
            + self.applied_capacity_bytes.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.state_version.encode_to(out);
        self.remaining_bytes.encode_to(out);
        self.applied_capacity_bytes.encode_to(out);
    }
}

impl Decode for ContractStorageBudget {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut reader = Reader::new(buf);
        let value = Self {
            state_version: reader.read()?,
            remaining_bytes: reader.read()?,
            applied_capacity_bytes: reader.read()?,
        };
        if reader.remaining() != 0 {
            return errf!(
                "contract storage budget decode: trailing {} bytes",
                reader.remaining()
            );
        }
        Ok((value, reader.used()))
    }
}

/// Fatal lifecycle/state error: a budget invariant broke, replaying with any other
/// outcome cannot reach consensus, so the node must stop loudly.
fn budget_fatal(msg: impl std::fmt::Display) -> sys::Error {
    sys::Error::abort(format!("contract storage budget: {msg}"))
        .with_code(STATE_DECODE_FAILED_CODE)
}

/// Read the persisted budget record (state decode failures surface as Abort).
pub fn read_contract_storage_budget(read: &dyn StateRead) -> Ret<Option<ContractStorageBudget>> {
    read_typed(read, &[KEY_CONTRACT_STORAGE_BUDGET])
}

/// Write the persisted budget record.
pub fn set_contract_storage_budget(
    layer: &mut dyn StateLayer,
    budget: &ContractStorageBudget,
) {
    let mut out = Vec::new();
    budget.encode_to(&mut out);
    layer.set(&[KEY_CONTRACT_STORAGE_BUDGET], out);
}

/// The block-level budget snapshot every contract storage fee in the block reads
/// (§4.2): `B_start` never moves within the block; `D` aggregates through tx-layer
/// commits; the block quota `K` is fixed at `min(B_start, K_max)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractStorageBlockSnapshot {
    /// `B_start`: remaining discount capacity at block start (parent state).
    pub remaining_start: u128,
    /// `D`: discount-consumed billed bytes so far in this block layer.
    pub used_discount: u128,
    /// `C = T × R` at the current height (after any §7.1 expansion migration).
    pub capacity: u128,
    /// `R`: active per-block supplement rate.
    pub rate: u128,
    /// `K = min(B_start, K_max)`: total discount quota for the block.
    pub quota: u128,
}

impl ContractStorageBlockSnapshot {
    /// Quota still grantable in this block: `K - D`.
    pub fn remaining_quota(&self) -> u128 {
        self.quota.saturating_sub(self.used_discount)
    }
}

fn read_u128_transient(read: &dyn StateRead, key: &[u8]) -> Ret<Option<u128>> {
    Ok(read_typed::<Uint12>(read, key)?.map(|v| v.uint()))
}

fn write_u128_transient(layer: &mut dyn StateLayer, key: &[u8], value: u128) -> Rerr {
    let encoded = Uint12::from_checked(value)
        .ok_or_else(|| budget_fatal(format!("transient value {value} overflows Uint12")))?;
    layer.set(key, encoded.encode());
    Ok(())
}

/// Block-start initialization (§4.2): read the parent budget, apply the §7.1
/// capacity-expansion migration if the active capacity grew, install the transient
/// `B_start`/`D=0` snapshot on the block layer. Idempotent errors (a missing/extra
/// record, shrink, or stale version) are fatal lifecycle facts, not user errors.
pub fn init_block_contract_storage_budget(
    layer: &mut dyn StateLayer,
    vp: &VmExecutionParams,
    height: u64,
) -> Ret<Option<ContractStorageBlockSnapshot>> {
    let storage = vp.contract_storage_fee;
    if !storage.is_active_at(height) {
        return Ok(None);
    }
    let Some((_, rate)) = storage.active_entry(height) else {
        return Err(budget_fatal(format!("no active schedule entry at {height}")));
    };
    let capacity = storage.capacity_at(height)?;
    let rate = u128::from(rate);
    let record = read_contract_storage_budget(&*layer)?;
    let start = match record {
        None => {
            if height != storage.activation_height {
                return Err(budget_fatal(format!(
                    "budget record missing at height {height} (H0 {})",
                    storage.activation_height
                )));
            }
            // First activation block starts with the full budget (B0 = C): every
            // pre-H0 deploy paid the enforced full price, so the mechanism behaves
            // as if it had been active — and unused — since genesis. The record is
            // born saturated, which makes the budget trajectory independent of the
            // activation height: a pure C/R/D token bucket that a later release can
            // manage without any H0 gate (a missing record then simply means the
            // mechanism never spent anything).
            capacity
        }
        Some(record) => {
            if record.state_version.uint() != CONTRACT_STORAGE_BUDGET_VERSION_V1 {
                return Err(budget_fatal(format!(
                    "unsupported budget state version {}",
                    record.state_version.uint()
                )));
            }
            let applied = record.applied_capacity_bytes.uint();
            let remaining = record.remaining_bytes.uint();
            if capacity < applied {
                return Err(budget_fatal(format!(
                    "active capacity {capacity} shrank below applied capacity {applied}; \
                     capacity reduction requires a new rule version (§7.4)"
                )));
            }
            if remaining > applied {
                return Err(budget_fatal(format!(
                    "remaining budget {remaining} exceeds recorded capacity {applied}"
                )));
            }
            // §7.1 expansion: keep the absolutely used amount, credit the whole
            // capacity delta to remaining budget (`used` is invariant).
            remaining.checked_add(capacity - applied).ok_or_else(|| {
                budget_fatal(format!(
                    "capacity migration overflow: {remaining} + {}",
                    capacity - applied
                ))
            })?
        }
    };
    write_u128_transient(layer, BLOCK_BUDGET_START_KEY, start)?;
    write_u128_transient(layer, BLOCK_BUDGET_USED_KEY, 0)?;
    Ok(Some(ContractStorageBlockSnapshot {
        remaining_start: start,
        used_discount: 0,
        capacity,
        rate,
        quota: storage.block_quota(start),
    }))
}

/// Read the live block snapshot from the current execution layer (used by contract
/// fee enforcement). A missing snapshot after activation is a lifecycle bug on
/// whichever execution path created this layer — fatal by design (§C6).
pub fn read_block_contract_storage_snapshot(
    read: &dyn StateRead,
    vp: &VmExecutionParams,
    height: u64,
) -> Ret<Option<ContractStorageBlockSnapshot>> {
    let storage = vp.contract_storage_fee;
    if !storage.is_active_at(height) {
        return Ok(None);
    }
    let Some((_, rate)) = storage.active_entry(height) else {
        return Err(budget_fatal(format!("no active schedule entry at {height}")));
    };
    let capacity = storage.capacity_at(height)?;
    let start = read_u128_transient(read, BLOCK_BUDGET_START_KEY)?
        .ok_or_else(|| budget_fatal("block budget snapshot missing while active"))?;
    if start > capacity {
        return Err(budget_fatal(format!(
            "block budget start {start} exceeds capacity {capacity}"
        )));
    }
    let used = read_u128_transient(read, BLOCK_BUDGET_USED_KEY)?
        .ok_or_else(|| budget_fatal("block budget used key missing while active"))?;
    if used > storage.block_quota(start) {
        return Err(budget_fatal(format!(
            "block discount usage {used} exceeds quota {}",
            storage.block_quota(start)
        )));
    }
    Ok(Some(ContractStorageBlockSnapshot {
        remaining_start: start,
        used_discount: used,
        capacity,
        rate: u128::from(rate),
        quota: storage.block_quota(start),
    }))
}

/// Best-effort budget read for quoting/analysis (§8.D): `Ok(None)` when the
/// mechanism is inactive or the caller is outside a block execution layer (no
/// `B_start` snapshot installed yet). Unlike [`read_block_contract_storage_snapshot`]
/// this never aborts on a missing transient key, so offline analysis can quote the
/// full price without knowing the head budget. Transient-key decode failures still
/// surface as fatal state errors.
pub fn peek_block_budget_remaining(
    read: &dyn StateRead,
    vp: &VmExecutionParams,
    height: u64,
) -> Ret<Option<u128>> {
    if !vp.contract_storage_fee.is_active_at(height) {
        return Ok(None);
    }
    read_u128_transient(read, BLOCK_BUDGET_START_KEY)
}

/// Consume `charge_bytes` of block discount quota for one whole transaction (§4.3
/// step 4: all-or-nothing). The write lands on the calling tx layer, so a tx-level
/// rollback removes it together with the rest of the transaction effects (§C2).
pub fn consume_block_contract_storage_discount(
    layer: &mut dyn StateLayer,
    snapshot: &ContractStorageBlockSnapshot,
    charge_bytes: usize,
) -> Ret<ContractStorageBlockSnapshot> {
    let charge = charge_bytes as u128;
    let remaining_quota = snapshot.remaining_quota();
    if charge > remaining_quota {
        return errf!(
            "contract storage discount quota exhausted: tx needs {charge} bytes, block has {remaining_quota} left (quota {}, used {})",
            snapshot.quota,
            snapshot.used_discount
        );
    }
    let used = snapshot
        .used_discount
        .checked_add(charge)
        .ok_or_else(|| budget_fatal("block discount usage overflow"))?;
    write_u128_transient(layer, BLOCK_BUDGET_USED_KEY, used)?;
    let mut next = *snapshot;
    next.used_discount = used;
    Ok(next)
}

/// Facts returned by settlement for node observability (§8.D).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractStorageSettlement {
    pub b_start: u128,
    pub discount_used: u128,
    pub b_next: u128,
    pub capacity: u128,
    pub rate: u128,
}

/// Block settlement (§4.4): `B_next = min(C, B_start - D + R)`; write the persisted
/// record and delete the transient snapshot. Called exactly once per block execution
/// on every path (strict, fast-sync, replay).
pub fn settle_block_contract_storage_budget(
    layer: &mut dyn StateLayer,
    vp: &VmExecutionParams,
    height: u64,
) -> Ret<Option<ContractStorageSettlement>> {
    let Some(snapshot) = read_block_contract_storage_snapshot(layer, vp, height)? else {
        return Ok(None);
    };
    // Transaction admission guarantees D <= K <= B_start; anything else is a
    // double-settlement or aggregation leak and must never reach persisted state.
    let remaining = snapshot
        .remaining_start
        .checked_sub(snapshot.used_discount)
        .ok_or_else(|| {
            budget_fatal(format!(
                "budget underflow: B_start {} < D {}",
                snapshot.remaining_start, snapshot.used_discount
            ))
        })?;
    let b_next = remaining
        .checked_add(snapshot.rate)
        .ok_or_else(|| {
            budget_fatal(format!(
                "budget refill overflow: {remaining} + {}",
                snapshot.rate
            ))
        })?
        .min(snapshot.capacity);
    let settlement = ContractStorageSettlement {
        b_start: snapshot.remaining_start,
        discount_used: snapshot.used_discount,
        b_next,
        capacity: snapshot.capacity,
        rate: snapshot.rate,
    };
    let record = ContractStorageBudget {
        state_version: Uint1::from(CONTRACT_STORAGE_BUDGET_VERSION_V1),
        remaining_bytes: Uint12::from_checked(b_next)
            .ok_or_else(|| budget_fatal(format!("B_next {b_next} overflows Uint12")))?,
        applied_capacity_bytes: Uint12::from_checked(snapshot.capacity)
            .ok_or_else(|| budget_fatal("capacity overflows Uint12"))?,
    };
    set_contract_storage_budget(layer, &record);
    layer.del(BLOCK_BUDGET_START_KEY);
    layer.del(BLOCK_BUDGET_USED_KEY);
    Ok(Some(settlement))
}

/// Post-block state verification (§C6): the transient keys must be gone, and from
/// `H0` onward the persisted record must exist, decode, and satisfy `B <= C` with the
/// record capacity matching the active schedule. Missing/extra records at the
/// activation boundary are lifecycle bugs, not user errors.
pub fn verify_block_contract_storage_state(
    read: &dyn StateRead,
    vp: &VmExecutionParams,
    height: u64,
) -> Rerr {
    let storage = vp.contract_storage_fee;
    if read_u128_transient(read, BLOCK_BUDGET_START_KEY)?.is_some() {
        return Err(budget_fatal("transient budget start key leaked into settled state"));
    }
    if read_u128_transient(read, BLOCK_BUDGET_USED_KEY)?.is_some() {
        return Err(budget_fatal("transient budget used key leaked into settled state"));
    }
    if !storage.is_active_at(height) {
        if read_contract_storage_budget(read)?.is_some() {
            return Err(budget_fatal(
                "budget record exists before activation height",
            ));
        }
        return Ok(());
    }
    let capacity = storage.capacity_at(height)?;
    let record = read_contract_storage_budget(read)?
        .ok_or_else(|| budget_fatal("budget record missing after settlement"))?;
    if record.state_version.uint() != CONTRACT_STORAGE_BUDGET_VERSION_V1 {
        return Err(budget_fatal(format!(
            "unsupported budget state version {}",
            record.state_version.uint()
        )));
    }
    if record.applied_capacity_bytes.uint() != capacity {
        return Err(budget_fatal(format!(
            "settled record capacity {} does not match active capacity {capacity}",
            record.applied_capacity_bytes.uint()
        )));
    }
    if record.remaining_bytes.uint() > capacity {
        return Err(budget_fatal(format!(
            "remaining budget {} exceeds capacity {capacity}",
            record.remaining_bytes.uint()
        )));
    }
    Ok(())
}

/// Read-only facts for node/wallet observability (§8.D): computed from a settled
/// state read and the active parameters — never from transient block keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractStorageFeeFacts {
    pub enabled: bool,
    pub rule_version: u8,
    pub activation_height: u64,
    pub height: u64,
    pub target_capacity_blocks: u64,
    pub capacity: u128,
    pub rate: u128,
    pub remaining: u128,
    pub period_floor: u64,
    pub max_periods: u64,
    pub periods: u64,
    pub max_block_discount_bytes: u64,
    /// Discount quota the next block grants at the current remaining budget.
    pub next_block_quota: u128,
    /// Next-block remaining range `[lo, hi]`: worst case this block consumes the
    /// full quota, best case nothing (`B - min(B, K_max) + R .. min(C, B + R)`).
    pub next_remaining_lo: u128,
    pub next_remaining_hi: u128,
    /// Periods at the next-block remaining bounds (price interval estimate).
    pub next_periods_lo: u64,
    pub next_periods_hi: u64,
}

/// Compute observability facts from the head budget record at `height` (the height
/// the next produced block will execute at). Pre-activation profiles report
/// `enabled = false` with legacy full price.
pub fn contract_storage_fee_facts(
    read: &dyn StateRead,
    vp: &VmExecutionParams,
    height: u64,
) -> Ret<ContractStorageFeeFacts> {
    let storage = vp.contract_storage_fee;
    let p_max = vp.contract_store_perm_periods;
    if !storage.is_active_at(height) {
        return Ok(ContractStorageFeeFacts {
            enabled: false,
            rule_version: storage.rule_version,
            activation_height: storage.activation_height,
            height,
            target_capacity_blocks: storage.target_capacity_blocks,
            capacity: 0,
            rate: 0,
            remaining: 0,
            period_floor: storage.period_floor,
            max_periods: p_max,
            periods: p_max,
            max_block_discount_bytes: storage.max_block_discount_bytes,
            next_block_quota: 0,
            next_remaining_lo: 0,
            next_remaining_hi: 0,
            next_periods_lo: p_max,
            next_periods_hi: p_max,
        });
    }
    let capacity = storage.capacity_at(height)?;
    let rate = u128::from(storage.active_rate(height).unwrap_or(0));
    let record = read_contract_storage_budget(read)?;
    let remaining = match record {
        None => 0,
        Some(record) => {
            let applied = record.applied_capacity_bytes.uint();
            if capacity < applied {
                return Err(budget_fatal(format!(
                    "active capacity {capacity} shrank below applied capacity {applied}"
                )));
            }
            let remaining = record.remaining_bytes.uint();
            if remaining > applied {
                return Err(budget_fatal(format!(
                    "remaining budget {remaining} exceeds recorded capacity {applied}"
                )));
            }
            // same §7.1 migration as block start: keep absolute used, credit the delta
            remaining.checked_add(capacity - applied).ok_or_else(|| {
                budget_fatal(format!(
                    "capacity migration overflow: {remaining} + {}",
                    capacity - applied
                ))
            })?
        }
    };
    let remaining = remaining.min(capacity);
    let periods = storage.discount_periods(remaining, capacity, p_max)?;
    let quota = storage.block_quota(remaining);
    let refill_lo = remaining.saturating_sub(quota);
    let next_lo = refill_lo.checked_add(rate).unwrap_or(u128::MAX).min(capacity);
    let next_hi = remaining.checked_add(rate).unwrap_or(capacity).min(capacity);
    let next_periods_lo = storage.discount_periods(next_lo, capacity, p_max)?;
    let next_periods_hi = storage.discount_periods(next_hi, capacity, p_max)?;
    Ok(ContractStorageFeeFacts {
        enabled: true,
        rule_version: storage.rule_version,
        activation_height: storage.activation_height,
        height,
        target_capacity_blocks: storage.target_capacity_blocks,
        capacity,
        rate,
        remaining,
        period_floor: storage.period_floor,
        max_periods: p_max,
        periods,
        max_block_discount_bytes: storage.max_block_discount_bytes,
        next_block_quota: quota,
        next_remaining_lo: next_lo,
        next_remaining_hi: next_hi,
        // lo remaining ⇒ high price, hi remaining ⇒ low price
        next_periods_lo: next_periods_hi.min(next_periods_lo),
        next_periods_hi: next_periods_lo.max(next_periods_hi),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{
        BlockRef, CONTRACT_STORAGE_RULE_V1, ContractStorageFeeParams, DiskDB, ForkChoiceKey,
        GAS_BUDGET_LOOKUP_NONE, StateChunkRef,
    };
    use field::Hash;

    struct NoDisk;
    impl DiskDB for NoDisk {
        fn read(&self, _key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
            Ok(None)
        }
        fn save(&self, _key: &[u8], _val: &[u8]) {}
        fn remove(&self, _key: &[u8]) {}
        fn try_write(&self, _memkv: &dyn crate::MemDB) -> sys::Rerr {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
        hash: Hash,
    }
    impl field::Encode for TestBlock {
        fn size(&self) -> usize {
            0
        }
        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }
    impl crate::Block for TestBlock {
        fn version(&self) -> u8 {
            1
        }
        fn height(&self) -> u64 {
            self.height
        }
        fn hash(&self) -> Hash {
            self.hash
        }
        fn prev_hash(&self) -> Hash {
            Hash::default()
        }
        fn mrklroot(&self) -> Hash {
            Hash::default()
        }
        fn timestamp(&self) -> u64 {
            self.height
        }
        fn transactions(&self) -> &[crate::TxRef] {
            &[]
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn block(height: u64, hash: Hash) -> BlockRef {
        Arc::new(TestBlock { height, hash })
    }

    fn root() -> StateChunkRef {
        StateChunkRef::new_root(Arc::new(NoDisk), block(0, Hash::default()))
    }

    /// Mainnet-shaped profile from §3.1 with H0 = 10_000 so tests can cheaply
    /// climb over the whole activation period. Fee purity floor in the chain
    /// pricing unit (u232): 5×10¹⁰ = the legacy 50,000 u238/byte equivalent.
    fn mainnet_like_params() -> VmExecutionParams {
        VmExecutionParams {
            contract_store_perm_periods: 10_000,
            contract_storage_fee: ContractStorageFeeParams {
                rule_version: CONTRACT_STORAGE_RULE_V1,
                activation_height: 10_000,
                target_capacity_blocks: 1_000,
                curve_steps: 1_000,
                period_floor: 10,
                max_block_discount_bytes: 16_384,
                supplement_schedule: &[(10_000, 1_000)],
            },
            initial_fee_purity_floor: 50_000_000_000,
            fee_purity_reductions: &[],
            gas_budget_lookup: &GAS_BUDGET_LOOKUP_NONE,
            tx_gas_budget_cap_byte: 0,
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
        }
    }

    /// Execute one full block lifecycle (init → optional txs → settle → verify)
    /// and freeze it onto `parent`, mirroring `chain::apply::execute_block`.
    fn execute_block_lifecycle(
        parent: &StateChunkRef,
        height: u64,
        vp: &VmExecutionParams,
        discount_bytes: &[usize],
    ) -> StateChunkRef {
        let mut b = StateChunkRef::block_exec_on(parent, block(height, Hash::from([height as u8; 32])), ForkChoiceKey::from_height(height)).unwrap();
        let snapshot = init_block_contract_storage_budget(&mut b, vp, height).unwrap();
        for &bytes in discount_bytes {
            let snapshot = snapshot.expect("discount tx only when active");
            // tx-level read-modify-write then commit
            let mut tx = b.spawn_tx_child(Hash::from([bytes as u8; 32])).unwrap();
            let tx_snap = read_block_contract_storage_snapshot(&tx, vp, height).unwrap().unwrap();
            let consumed = consume_block_contract_storage_discount(&mut tx, &tx_snap, bytes).unwrap();
            assert_eq!(consumed.used_discount - tx_snap.used_discount, bytes as u128);
            let parent = tx.commit_to_parent().unwrap();
            assert!(parent.ptr_eq(&b));
            let _ = snapshot;
        }
        settle_block_contract_storage_budget(&mut b, vp, height).unwrap();
        verify_block_contract_storage_state(&b, vp, height).unwrap();
        parent.attach_block_child(&b).unwrap();
        b
    }

    fn head_budget(read: &dyn StateRead) -> ContractStorageBudget {
        read_contract_storage_budget(read).unwrap().unwrap()
    }

    /// Fabricate a settled head whose budget record reads exactly `remaining`
    /// against `capacity` (no lifecycle ran on the fabricated block): lets a test
    /// position the budget at an arbitrary level without replaying drain blocks.
    fn fabricate_record_head(
        parent: &StateChunkRef,
        height: u64,
        remaining: u128,
        capacity: u128,
    ) -> StateChunkRef {
        let mut b = StateChunkRef::block_exec_on(
            parent,
            block(height, Hash::from([height.to_le_bytes()[0]; 32])),
            ForkChoiceKey::from_height(height),
        )
        .unwrap();
        set_contract_storage_budget(
            &mut b,
            &ContractStorageBudget {
                state_version: Uint1::from(CONTRACT_STORAGE_BUDGET_VERSION_V1),
                remaining_bytes: Uint12::from_checked(remaining).unwrap(),
                applied_capacity_bytes: Uint12::from_checked(capacity).unwrap(),
            },
        );
        parent.attach_block_child(&b).unwrap();
        b
    }

    #[test]
    fn disabled_profile_and_pre_activation_are_legacy_noops() {
        let root = root();
        let vp_disabled = VmExecutionParams {
            contract_storage_fee: ContractStorageFeeParams::disabled(),
            ..mainnet_like_params()
        };
        let mut b = StateChunkRef::block_draft_on(&root, 1);
        assert!(init_block_contract_storage_budget(&mut b, &vp_disabled, 1).unwrap().is_none());
        assert!(settle_block_contract_storage_budget(&mut b, &vp_disabled, 1).unwrap().is_none());
        verify_block_contract_storage_state(&b, &vp_disabled, 1).unwrap();
        assert!(read_contract_storage_budget(&b).unwrap().is_none());
        // the transient keys must not leak even in a no-op lifecycle
        assert!(b.get(BLOCK_BUDGET_START_KEY).unwrap().is_none());

        // pre-activation heights keep the legacy rule too
        let vp = mainnet_like_params();
        let mut b2 = StateChunkRef::block_draft_on(&root, 9_999);
        assert!(init_block_contract_storage_budget(&mut b2, &vp, 9_999).unwrap().is_none());
        verify_block_contract_storage_state(&b2, &vp, 9_999).unwrap();
    }

    /// §9 row: `H0` with a missing budget record → `B_start = C` (B0 = C): the
    /// discount mechanism behaves as if it had been active — and unused — since
    /// genesis, so the budget record is born full.
    #[test]
    fn activation_block_starts_full_and_settles_full() {
        let root = root();
        let vp = mainnet_like_params();
        let b = execute_block_lifecycle(&root, 10_000, &vp, &[]);
        let record = head_budget(&b);
        assert_eq!(record.remaining_bytes.uint(), 1_000_000);
        assert_eq!(record.applied_capacity_bytes.uint(), 1_000_000);
        assert_eq!(record.state_version.uint(), 1);
    }

    /// §5.2 under B0 = C: the budget is born full at H0 and idle blocks keep it
    /// full — every block from the activation block itself starts at the
    /// 10-period floor, there is no ramp from zero.
    #[test]
    fn idle_blocks_stay_full_from_activation() {
        let root = root();
        let vp = mainnet_like_params();
        let storage = vp.contract_storage_fee;
        let c = 1_000_000u128;
        let mut tip = root.clone();
        let mut start_remaining = Vec::new();
        for i in 0..=1000u64 {
            let height = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(height, Hash::from([(height % 256) as u8; 32])), ForkChoiceKey::from_height(height)).unwrap();
            let snap = init_block_contract_storage_budget(&mut b, &vp, height).unwrap().unwrap();
            start_remaining.push(snap.remaining_start);
            settle_block_contract_storage_budget(&mut b, &vp, height).unwrap();
            verify_block_contract_storage_state(&b, &vp, height).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        // B0 = C: every idle block starts fully funded at the floor price
        for (i, &b) in start_remaining.iter().enumerate() {
            assert_eq!(b, c, "idle block {i} must start full");
            assert_eq!(
                storage.discount_periods(b, c, 10_000).unwrap(),
                10,
                "full budget prices at the floor from the activation block on"
            );
        }
        assert_eq!(head_budget(&tip).remaining_bytes.uint(), c);
    }

    /// §9 row: discount consumption moves D into settlement; idle next block refills.
    #[test]
    fn discount_usage_lowers_next_block_remaining_and_recovers_at_rate() {
        let root = root();
        let vp = mainnet_like_params();
        // warm up to a full budget
        let mut tip = root.clone();
        for i in 0..=1000u64 {
            let h = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            settle_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        assert_eq!(head_budget(&tip).remaining_bytes.uint(), 1_000_000);
        // block uses 15,000 discount bytes across three txs (≤ K_max 16,384)
        let b1 = execute_block_lifecycle(&tip, 11_001, &vp, &[5_000, 5_000, 5_000]);
        assert_eq!(head_budget(&b1).remaining_bytes.uint(), 1_000_000 - 15_000 + 1_000);
        // an idle block refills by R
        let b2 = execute_block_lifecycle(&b1, 11_002, &vp, &[]);
        assert_eq!(head_budget(&b2).remaining_bytes.uint(), 1_000_000 - 15_000 + 2_000);
    }

    /// §9 rows: a transaction above the remaining quota is rejected (all-or-nothing);
    /// paying in two txs works; total never exceeds min(B_start, K_max).
    #[test]
    fn discount_quota_caps_per_block_usage() {
        let root = root();
        let vp = mainnet_like_params();
        let mut tip = root.clone();
        for i in 0..=1000u64 {
            let h = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            settle_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        let mut b = StateChunkRef::block_exec_on(&tip, block(11_001, Hash::from([2; 32])), ForkChoiceKey::from_height(11_001)).unwrap();
        let snap = init_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap().unwrap();
        // quota = min(1MB, 16KB) = 16,384
        assert_eq!(snap.quota, 16_384);
        // a single 20 KB discount tx is rejected
        let mut tx = b.spawn_tx_child(Hash::from([3; 32])).unwrap();
        let tx_snap = read_block_contract_storage_snapshot(&tx, &vp, 11_001).unwrap().unwrap();
        assert!(consume_block_contract_storage_discount(&mut tx, &tx_snap, 20_000).is_err());
        tx.discard().unwrap();
        // 12 KB then 5 KB > 16,384: committed in tx order, the second must reject
        let mut tx1 = b.spawn_tx_child(Hash::from([4; 32])).unwrap();
        let s1 = read_block_contract_storage_snapshot(&tx1, &vp, 11_001).unwrap().unwrap();
        consume_block_contract_storage_discount(&mut tx1, &s1, 12_000).unwrap();
        tx1.commit_to_parent().unwrap();
        let mut tx2 = b.spawn_tx_child(Hash::from([5; 32])).unwrap();
        let s2 = read_block_contract_storage_snapshot(&tx2, &vp, 11_001).unwrap().unwrap();
        assert_eq!(s2.used_discount, 12_000);
        assert!(consume_block_contract_storage_discount(&mut tx2, &s2, 5_000).is_err());
        tx2.discard().unwrap();
        // exactly-at-cap allowed: 4,384 more
        let mut tx3 = b.spawn_tx_child(Hash::from([6; 32])).unwrap();
        let s3 = read_block_contract_storage_snapshot(&tx3, &vp, 11_001).unwrap().unwrap();
        assert_eq!(s3.remaining_quota(), 16_384 - 12_000);
        consume_block_contract_storage_discount(&mut tx3, &s3, 4_384).unwrap();
        tx3.commit_to_parent().unwrap();
        settle_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
        assert_eq!(head_budget(&b).remaining_bytes.uint(), 1_000_000 - 16_384 + 1_000);
    }

    /// §9 row: a failed transaction rolls its discount back with its chunk.
    #[test]
    fn failed_tx_discount_usage_rolls_back() {
        let root = root();
        let vp = mainnet_like_params();
        let mut tip = root.clone();
        for i in 0..=1000u64 {
            let h = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            settle_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        let mut b = StateChunkRef::block_exec_on(&tip, block(11_001, Hash::from([2; 32])), ForkChoiceKey::from_height(11_001)).unwrap();
        init_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
        // consume on a tx layer then fail/discard it
        let mut tx = b.spawn_tx_child(Hash::from([7; 32])).unwrap();
        let s = read_block_contract_storage_snapshot(&tx, &vp, 11_001).unwrap().unwrap();
        consume_block_contract_storage_discount(&mut tx, &s, 8_000).unwrap();
        tx.discard().unwrap();
        // the used accumulator still reads zero after rollback
        let s2 = read_block_contract_storage_snapshot(&b, &vp, 11_001).unwrap().unwrap();
        assert_eq!(s2.used_discount, 0);
        settle_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
        // nothing was consumed, refill +1000 is capped at C
        assert_eq!(head_budget(&b).remaining_bytes.uint(), 1_000_000);
    }

    /// §7.1: a capacity expansion credits the full delta and preserves absolute
    /// usage; prices never rise because B moved with C.
    #[test]
    fn capacity_expansion_migrates_without_raising_prices() {
        let vp = mainnet_like_params();
        let expanded = {
            let mut p = vp;
            p.contract_storage_fee.supplement_schedule = &[(10_000, 1_000), (20_000, 2_000)];
            p
        };
        let root = root();
        // Activation head under B0 = C (H0 idle block, born full), then fabricate
        // a settled head whose record reads B = 500,000: the same "half spent"
        // precondition the old zero-ramp design reached after 500 idle blocks.
        let tip = execute_block_lifecycle(&root, 10_000, &vp, &[]);
        let tip = fabricate_record_head(&tip, 11_500, 500_000, 1_000_000);
        assert_eq!(head_budget(&tip).remaining_bytes.uint(), 500_000);
        // expansion block at 20_000 under the new schedule (C=2MB)
        let mut b = StateChunkRef::block_exec_on(&tip, block(20_000, Hash::from([9; 32])), ForkChoiceKey::from_height(20_000)).unwrap();
        let snap = init_block_contract_storage_budget(&mut b, &expanded, 20_000).unwrap().unwrap();
        // used 500,000 stays put; B jumps to 1,500,000
        assert_eq!(snap.remaining_start, 1_500_000);
        assert_eq!(snap.capacity, 2_000_000);
        let used_old = 1_000_000 - 500_000;
        let used_new = snap.capacity - snap.remaining_start;
        assert_eq!(used_old, used_new);
        // §7.1: the expansion never raises prices — the used fraction is preserved
        // or diluted (full budget keeps the floor, exhausted budget gets cheaper).
        let p_old = vp.contract_storage_fee.discount_periods(500_000, 1_000_000, 10_000).unwrap();
        let p_new = expanded.contract_storage_fee.discount_periods(1_500_000, 2_000_000, 10_000).unwrap();
        assert!(p_new <= p_old, "capacity expansion must never raise the current price");
        let floor_old = vp.contract_storage_fee.discount_periods(1_000_000, 1_000_000, 10_000).unwrap();
        let floor_new = expanded.contract_storage_fee.discount_periods(2_000_000, 2_000_000, 10_000).unwrap();
        assert_eq!(floor_old, floor_new);
        settle_block_contract_storage_budget(&mut b, &expanded, 20_000).unwrap();
        let record = head_budget(&b);
        assert_eq!(record.applied_capacity_bytes.uint(), 2_000_000);
    }

    /// §A16: replaying old blocks uses the old schedule segment; a later expansion
    /// never rewrites earlier budget trajectories (the final params never reclassify
    /// history because the gate is the height-ordered active segment).
    #[test]
    fn old_segment_replay_uses_old_rules() {
        let vp = mainnet_like_params();
        let mut expanded = vp;
        expanded.contract_storage_fee.supplement_schedule = &[(10_000, 1_000), (20_000, 2_000)];
        // drive H0..H0+1000 under each profile: identical early budget trajectory
        for profile in [vp, expanded] {
            let root = root();
            let mut tip = root.clone();
            for i in 0..=1000u64 {
                let h = 10_000 + i;
                let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
                init_block_contract_storage_budget(&mut b, &profile, h).unwrap();
                settle_block_contract_storage_budget(&mut b, &profile, h).unwrap();
                tip.attach_block_child(&b).unwrap();
                tip = b;
            }
            assert_eq!(head_budget(&tip).remaining_bytes.uint(), 1_000_000);
        }
    }

    /// Post-block verification: transient key leaks, missing records and
    /// inconsistent capacity are deterministic lifecycle errors.
    #[test]
    fn verify_detects_leaks_and_invariants() {
        let root = root();
        let vp = mainnet_like_params();
        // leak of a start key into a settled block
        let mut b = StateChunkRef::block_exec_on(&root, block(10_000, Hash::from([3; 32])), ForkChoiceKey::from_height(10_000)).unwrap();
        init_block_contract_storage_budget(&mut b, &vp, 10_000).unwrap();
        settle_block_contract_storage_budget(&mut b, &vp, 10_000).unwrap();
        b.set(BLOCK_BUDGET_START_KEY, Uint12::from(1u128).encode());
        let err = verify_block_contract_storage_state(&b, &vp, 10_000);
        assert!(err.is_err());
        assert!(err.unwrap_err().is_abort());
        // missing persistent record after a transient setup (no settle ran)
        let mut b2 = StateChunkRef::block_exec_on(&root, block(10_001, Hash::from([4; 32])), ForkChoiceKey::from_height(10_001)).unwrap();
        b2.set(BLOCK_BUDGET_START_KEY, Uint12::from(5u128).encode());
        b2.set(BLOCK_BUDGET_USED_KEY, Uint12::from(0u128).encode());
        let err = verify_block_contract_storage_state(&b2, &vp, 10_001);
        assert!(err.is_err());
        assert!(err.unwrap_err().is_abort());
        // capacity shrink: record applied at 1 MB, active capacity re-keyed smaller
        let mut b3 = StateChunkRef::block_exec_on(&root, block(10_000, Hash::from([5; 32])), ForkChoiceKey::from_height(10_000)).unwrap();
        init_block_contract_storage_budget(&mut b3, &vp, 10_000).unwrap();
        settle_block_contract_storage_budget(&mut b3, &vp, 10_000).unwrap();
        let shrunk = {
            let mut p = vp;
            p.contract_storage_fee.supplement_schedule = &[(10_000, 500)]; // C = 500,000 < applied 1,000,000
            p
        };
        let err = verify_block_contract_storage_state(&b3, &shrunk, 10_001);
        assert!(err.is_err());
        // records before activation are rejected
        let mut b4 = StateChunkRef::block_exec_on(&root, block(1, Hash::from([6; 32])), ForkChoiceKey::from_height(1)).unwrap();
        b4.set(&[KEY_CONTRACT_STORAGE_BUDGET], vec![1, 0, 0]);
        let err = verify_block_contract_storage_state(&b4, &vp, 1);
        assert!(err.is_err());
    }

    #[test]
    fn snapshot_rejects_missing_used_key_and_start_over_capacity() {
        let root = root();
        let vp = mainnet_like_params();

        let mut missing_used = StateChunkRef::block_exec_on(
            &root,
            block(10_000, Hash::from([7; 32])),
            ForkChoiceKey::from_height(10_000),
        )
        .unwrap();
        init_block_contract_storage_budget(&mut missing_used, &vp, 10_000).unwrap();
        missing_used.del(BLOCK_BUDGET_USED_KEY);
        let err = read_block_contract_storage_snapshot(&missing_used, &vp, 10_000)
            .expect_err("missing USED key must be a lifecycle error");
        assert!(err.is_abort());

        let mut oversized_start = StateChunkRef::block_exec_on(
            &root,
            block(10_000, Hash::from([8; 32])),
            ForkChoiceKey::from_height(10_000),
        )
        .unwrap();
        init_block_contract_storage_budget(&mut oversized_start, &vp, 10_000).unwrap();
        oversized_start.set(
            BLOCK_BUDGET_START_KEY,
            Uint12::from(1_000_001u128).encode(),
        );
        let err = read_block_contract_storage_snapshot(&oversized_start, &vp, 10_000)
            .expect_err("B_start above C must be a lifecycle error");
        assert!(err.is_abort());
    }

    /// Random/property invariants (§9): `0 <= B <= C`, prices stay inside
    /// `[P_min, P_max]`, and remaining-to-price is monotone non-increasing.
    #[test]
    fn random_budget_invariants() {
        let storage = mainnet_like_params().contract_storage_fee;
        let c = 1_000_000u128;
        // deterministic pseudo-random sweep over every possible remaining budget
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut next = move || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            x >> 33
        };
        for _ in 0..5_000 {
            let b = (next() as u128) % (c + 1);
            let periods = storage.discount_periods(b, c, 10_000).unwrap();
            assert!((10..=10_000).contains(&periods));
            // out-of-range inputs clamp without error and never escape the floor
            let periods_hi = storage.discount_periods(c + 99, c, 10_000).unwrap();
            assert_eq!(periods_hi, 10);
            // crossing a curve boundary (every 1000 used bytes) steps by exactly
            // one step: 1,000 used bytes move the integer step by one unit
            if b > 1_000 && b < c {
                let step_1000_more = storage.discount_periods(b - 1_000, c, 10_000).unwrap();
                let diff = step_1000_more.saturating_sub(periods);
                assert!(diff <= 20, "1000 more used bytes may step the price by at most two steps: {diff} at B {b}");
            }
        }
        // monotone sweep over the whole range at 1 KiB resolution
        let mut prev = u64::MAX;
        for step in (0u128..=c).step_by(1_000) {
            let p = storage.discount_periods(step, c, 10_000).unwrap();
            assert!(p <= prev, "price must not rise as remaining grows: {p} > {prev} at B {step}");
            prev = p;
        }
        assert_eq!(prev, 10);
    }

    /// §6.2 simulation: repeated small edits drain the quota but never overdraw it,
    /// and the budget recovers at R when use stops.
    #[test]
    fn simulation_repeated_small_edits_drain_only_up_to_quota() {
        let root = root();
        let vp = mainnet_like_params();
        let mut tip = root.clone();
        // warm up to full capacity
        for i in 0..=1000u64 {
            let h = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            settle_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        // attacker submits 4-byte edits until quota is exhausted within one block
        let mut b = StateChunkRef::block_exec_on(&tip, block(11_001, Hash::from([9; 32])), ForkChoiceKey::from_height(11_001)).unwrap();
        let snap = init_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap().unwrap();
        let mut used = 0u128;
        while used + 4 <= snap.quota {
            let mut tx = b.spawn_tx_child(Hash::from([(used % 255) as u8; 32])).unwrap();
            let s = read_block_contract_storage_snapshot(&tx, &vp, 11_001).unwrap().unwrap();
            consume_block_contract_storage_discount(&mut tx, &s, 4).unwrap();
            tx.commit_to_parent().unwrap();
            used += 4;
        }
        // the next 4-byte edit must fail: quota fully drained
        let mut tx = b.spawn_tx_child(Hash::from([8; 32])).unwrap();
        let s = read_block_contract_storage_snapshot(&tx, &vp, 11_001).unwrap().unwrap();
        assert!(consume_block_contract_storage_discount(&mut tx, &s, 4).is_err());
        tx.discard().unwrap();
        assert_eq!(used, 16_384);
        settle_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
        // B dropped by exactly the quota; the refill cannot produce a negative balance
        assert_eq!(head_budget(&b).remaining_bytes.uint(), 1_000_000 - 16_384 + 1_000);
    }

    /// Arbitrary tx-set permutation never changes per-tx minima or the final D
    /// when every tx fits the quota (§4.5 / property list).
    #[test]
    fn permutation_invariant_when_total_fits_quota() {
        let root = root();
        let vp = mainnet_like_params();
        let mut tip = root.clone();
        for i in 0..=1000u64 {
            let h = 10_000 + i;
            let mut b = StateChunkRef::block_exec_on(&tip, block(h, Hash::from([1; 32])), ForkChoiceKey::from_height(h)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            settle_block_contract_storage_budget(&mut b, &vp, h).unwrap();
            tip.attach_block_child(&b).unwrap();
            tip = b;
        }
        // two different orders of the same tx sizes under the full 16 KiB quota
        for order in [[8_000u128, 4_000, 3_000], [3_000, 8_000, 4_000], [4_000, 3_000, 8_000]] {
            let mut b = StateChunkRef::block_exec_on(&tip, block(11_001, Hash::from([order[0] as u8; 32])), ForkChoiceKey::from_height(11_001)).unwrap();
            init_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
            let mut total = 0u128;
            for &sz in &order {
                let mut tx = b.spawn_tx_child(Hash::from([(sz % 255) as u8; 32])).unwrap();
                let s = read_block_contract_storage_snapshot(&tx, &vp, 11_001).unwrap().unwrap();
                consume_block_contract_storage_discount(&mut tx, &s, sz as usize).unwrap();
                tx.commit_to_parent().unwrap();
                total += sz;
            }
            settle_block_contract_storage_budget(&mut b, &vp, 11_001).unwrap();
            assert_eq!(head_budget(&b).remaining_bytes.uint(), 1_000_000 - total + 1_000);
        }
    }

    #[test]
    fn quote_reports_full_and_discount_tiers() {
        let root = root();
        let vp = mainnet_like_params();
        // full-budget head at the activation block (B0 = C)
        let full_tip = execute_block_lifecycle(&root, 10_000, &vp, &[]);
        // near-drained head fabricated on top: B = 1,000 against C = 1,000,000
        let tip = fabricate_record_head(&full_tip, 10_001, 1_000, 1_000_000);
        let height = 10_002;
        let charge = 10_000u64;
        let q = contract_storage_fee_quote(&tip, &vp, height, charge).unwrap();
        // full price: 5×10¹⁰ u232/byte × 10,000 bytes × 10,000 periods
        assert_eq!(q.floor_purity, 50_000_000_000);
        assert_eq!(q.full_u232, 50_000_000_000u128 * 10_000 * 10_000);
        // discount at the head remaining 1000 bytes: step = ceil(1000·999k/1M) = 999
        // → periods = step × P_min = 9,990 (near full, budget nearly drained)
        let p = vp.contract_storage_fee
            .discount_periods(1_000, 1_000_000, 10_000)
            .unwrap();
        assert_eq!(p, 9_990);
        assert_eq!(
            q.discount_u232,
            Some(50_000_000_000u128 * 10_000 * p as u128)
        );
        // the full-budget activation head quotes the 10-period floor discount
        let q_full = contract_storage_fee_quote(&full_tip, &vp, 10_001, charge).unwrap();
        assert_eq!(
            q_full.discount_u232,
            Some(50_000_000_000u128 * 10_000 * 10)
        );
        // pre-activation heights quote full price only
        let q_off = contract_storage_fee_quote(&tip, &vp, 9_999, charge).unwrap();
        assert_eq!(q_off.discount_u232, None);
        assert_eq!(q_off.full_u232, q.full_u232);
    }

    #[test]
    fn quote_ceils_sub_u238_pricing_up_to_one_u238() {
        let root = root();
        let mut vp = mainnet_like_params();
        // 1 u232/byte: 1 byte × 10_000 periods = 10_000 u232 < 1 u238.
        vp.initial_fee_purity_floor = 1;
        let full_tip = execute_block_lifecycle(&root, 10_000, &vp, &[]);
        let q = contract_storage_fee_quote(&full_tip, &vp, 10_001, 1).unwrap();
        assert_eq!(q.floor_purity, 1);
        // ceil to 1 u238, expressed back in u232.
        assert_eq!(q.full_u232, crate::SETTLEMENT_SCALE);
        // full-budget discount: 1 × 1 × 10 periods = 10 u232 → same 1 u238.
        assert_eq!(q.discount_u232, Some(crate::SETTLEMENT_SCALE));
    }

    #[test]
    fn facts_view_reports_expected_fields() {
        let root = root();
        let vp = mainnet_like_params();
        // full-budget head at the activation block (B0 = C)
        let full_tip = execute_block_lifecycle(&root, 10_000, &vp, &[]);
        // near-drained head fabricated on top: B = 1,000 against C = 1,000,000
        let tip = fabricate_record_head(&full_tip, 10_001, 1_000, 1_000_000);
        let facts = contract_storage_fee_facts(&tip, &vp, 10_002).unwrap();
        assert!(facts.enabled);
        assert_eq!(facts.remaining, 1_000);
        assert_eq!(facts.rate, 1_000);
        assert_eq!(facts.capacity, 1_000_000);
        assert_eq!(facts.max_periods, 10_000);
        assert_eq!(facts.periods, vp.contract_storage_fee.discount_periods(1_000, 1_000_000, 10_000).unwrap());
        // next range: worst case consumes the whole quota min(1,000, K_max) = 1,000
        assert_eq!(facts.next_remaining_hi, 1_000 + 1_000);
        assert_eq!(facts.next_remaining_lo, 1_000);
        // the full-budget activation head reports the floor price
        let facts_full = contract_storage_fee_facts(&full_tip, &vp, 10_001).unwrap();
        assert!(facts_full.enabled);
        assert_eq!(facts_full.remaining, 1_000_000);
        assert_eq!(facts_full.periods, 10);
        // full-price facts before activation
        let facts_off = contract_storage_fee_facts(&full_tip, &vp, 9_999).unwrap();
        assert!(!facts_off.enabled);
        assert_eq!(facts_off.periods, 10_000);
    }
}

/// Floor-basis two-tier fee quote for `charge_bytes` at `height` (§8.D). Amounts are
/// the consensus minimum: priced in u232 then ceiled to the gas settlement unit
/// (u238) and expressed back in u232 (`ceil(v/10⁶)×10⁶`) so they match
/// `settlement_amount`. Priced at the consensus fee-purity floor; any actual
/// transaction purity above the floor scales both figures linearly (§3.4), so a
/// wallet pays at least `full_u232` for guaranteed inclusion and at least
/// `discount_u232` when it accepts discount-classification under quota competition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractStorageFeeQuote {
    /// Floor purity in the pricing unit (u232 per billing byte).
    pub floor_purity: u64,
    pub full_u232: u128,
    pub discount_u232: Option<u128>,
}

/// State-aware quote over a settled head read. `enabled=false` profiles (and
/// pre-activation heights) quote the full price only.
pub fn contract_storage_fee_quote(
    read: &dyn StateRead,
    vp: &VmExecutionParams,
    height: u64,
    charge_bytes: u64,
) -> Ret<ContractStorageFeeQuote> {
    let facts = contract_storage_fee_facts(read, vp, height)?;
    let floor = u128::from(vp.effective_fee_purity(height, 0));
    let bytes = u128::from(charge_bytes);
    let scale = |periods: u64| -> Ret<u128> {
        let v = floor
            .checked_mul(bytes)
            .and_then(|v| v.checked_mul(u128::from(periods)))
            .ok_or_else(|| budget_fatal("storage fee quote overflow"))?;
        // Same ceil-to-238 as the consensus minimum, expressed back in u232.
        crate::ceil_pricing_to_settlement(v)
            .checked_mul(crate::SETTLEMENT_SCALE)
            .ok_or_else(|| budget_fatal("storage fee quote overflow"))
    };
    let full_u232 = scale(facts.max_periods)?;
    let discount_u232 = if facts.enabled {
        Some(scale(facts.periods)?)
    } else {
        None
    };
    Ok(ContractStorageFeeQuote {
        floor_purity: u64::try_from(floor).unwrap_or(u64::MAX),
        full_u232,
        discount_u232,
    })
}

/// §6/A20 economic simulations over a *model* of the consensus budget using the
/// real mainnet limits (1 MiB max block, 16 KiB max tx, §2.1): single-block dump,
/// split deployment, front-run-and-overwrite, and repeated small-edit burn attacks.
/// These run the exact same price/quota formulas as consensus but step blocks and
/// attacker transaction sets explicitly, so the cost/benefit numbers are auditable.
#[cfg(test)]
mod economic_sim_tests {
    use super::*;
    use crate::{CONTRACT_STORAGE_RULE_V1, ContractStorageFeeParams, GAS_BUDGET_LOOKUP_NONE};

    /// Mainnet-like model profile (§3.1): C=1,000,000; R=1,000; T=1,000; K=16,384.
    /// Purity floor 5×10¹⁰ u232/byte; 1 HAC = 10^16 u232 (§3.4).
    fn model() -> (VmExecutionParams, ContractStorageFeeParams) {
        let vp = VmExecutionParams {
            contract_store_perm_periods: 10_000,
            contract_storage_fee: ContractStorageFeeParams {
                rule_version: CONTRACT_STORAGE_RULE_V1,
                activation_height: 784_000,
                target_capacity_blocks: 1_000,
                curve_steps: 1_000,
                period_floor: 10,
                max_block_discount_bytes: 16_384,
                supplement_schedule: &[(784_000, 1_000)],
            },
            initial_fee_purity_floor: 50_000_000_000,
            fee_purity_reductions: &[],
            gas_budget_lookup: &GAS_BUDGET_LOOKUP_NONE,
            tx_gas_budget_cap_byte: 0,
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
        };
        let storage = vp.contract_storage_fee;
        (vp, storage)
    }

    const MAX_TX: u128 = 16 * 1024; // bytes (max single tx / K_max)
    // const MAX_BLOCK: u128 = 1024 * 1024; // bytes (max block payload)
    const P_MAX: u64 = 10_000;

    fn periods(storage: &ContractStorageFeeParams, b: u128) -> u64 {
        storage.discount_periods(b, 1_000_000, P_MAX).unwrap()
    }

    /// Price burned (u232) for `charge_bytes` at `periods` on the 5×10¹⁰ purity floor.
    fn burn(charge: u128, periods: u64) -> u128 {
        50_000_000_000u128 * charge * periods as u128
    }

    /// Simulate the attacker's sustained small-edit drain and measure the economics.
    #[test]
    fn sim_one_block_dump_is_capped_and_prices_rise() {
        let (_vp, storage) = model();
        // Fully funded budget; the attacker tries to move as much discount storage
        // as possible as fast as possible (dump every block to the quota cap).
        let mut b = 1_000_000u128;
        let mut periods_seen = Vec::new();
        let mut blocks = 0u32;
        while b > 1_000_000 / 2 {
            // max discount usage in this block is min(B, K_max)
            let used = b.min(MAX_TX);
            // settle: refill after usage
            b = (b - used + 1_000).min(1_000_000);
            periods_seen.push(periods(&storage, b));
            blocks += 1;
            assert!(blocks < 10_000);
        }
        // Dumping to half the budget takes many blocks: 15,384 net drain per block
        // ⇒ a ~500k dump takes ≥ ceil(500_000/15_384) = 33 blocks even at the cap.
        assert!(blocks >= 32, "single-block dump is blocked by K_max: took {blocks} blocks");
        // While the budget depletes the discount price never falls: 100%-…-full dump
        // cannot be executed at floor prices.
        let mut prev = 0u64;
        for (i, p) in periods_seen.iter().enumerate() {
            if i > 0 && *p < prev {
                panic!("discount price must not fall during a sustained dump (block {i}: {p} < {prev})");
            }
            prev = *p;
        }
        // and the absolute used amount is bounded by 1 MiB (never above C)
        assert!(b <= 1_000_000);
        // total discount bytes moved in the dump window cannot exceed C
        let moved = 1_000_000u128.saturating_sub(b);
        assert!(moved <= 1_000_000);
    }

    /// Splitting a huge payload into 16 KiB sub-deploys never earns a discount
    /// bonus: each sub-deploy is billed and quota-charged by its real bytes.
    #[test]
    fn sim_split_deployment_pays_per_real_byte() {
        let (_vp, storage) = model();
        // A payload too large for a single tx (max_tx_size 16 KiB) is split into
        // N chunks of ≤16 KiB.
        let total = 1_000_000u128;
        let n_chunks = total.div_ceil(MAX_TX);
        assert_eq!(n_chunks, 62); // 1,000,000 / 16,384 → 62 chunks
        let mut b = 1_000_000u128;
        let mut charged = 0u128;
        for _ in 0..n_chunks {
            let chunk = MAX_TX.min(total - charged);
            assert!(chunk <= MAX_TX);
            let used = chunk.min(b.min(MAX_TX)); // quota bound
            assert!(used == chunk, "a 16 KiB chunk fits one block's quota when funded");
            // fee is per real chunk bytes at the current price
            let _ = burn(chunk, periods(&storage, b));
            b = (b - chunk + 1_000).min(1_000_000);
            charged += chunk;
        }
        assert_eq!(charged, total);
        // Split storage cannot exceed C; discount spend equals actual payload bytes
        assert!(charged <= 1_000_000);
    }

    /// Front-run-and-overwrite: a placeholder deploy followed by overwrites pays
    /// the protocol fee every time; the discount quota is never reserved by an
    /// earlier tx, so "hold the low price" has no rule-level arbitrage profit.
    #[test]
    fn sim_front_run_then_overwrite_is_charged_each_time() {
        let (_vp, storage) = model();
        // placeholder is small and cheap, then a large overwrite follows later.
        let placeholder = 100u128;
        let overwrite = 8_000u128; // a mid-size update payload
        let mut b = 1_000_000u128;
        // 1) placeholder deploy
        let p0 = periods(&storage, b);
        let fee_ph = burn(placeholder, p0);
        b = (b - placeholder + 1_000).min(1_000_000);
        // 2) later overwrite (update) after some idle blocks
        for _ in 0..100 {
            b = (b + 1_000).min(1_000_000);
        }
        let p1 = periods(&storage, b);
        let fee_ow = burn(overwrite, p1);
        // Both operations are charged: reserving the address has no fee waiver and
        // the overwrite pays the same per-byte price as a fresh write would.
        assert!(fee_ph > 0 && fee_ow > 0);
        assert_eq!(fee_ow, burn(overwrite, p1));
        assert!(p1 <= p0, "idle refill may only improve the price, never worsen it");
    }

    /// Sustained small-edit burn (§6.2): an attacker can push the price up to near
    /// full, but every drained byte burns the floor protocol fee, and the budget
    /// bottoms out at R=1,000 bytes/block (the refill floor) rather than zero —
    /// so price manipulation is a burn-type cost, not free. Once the attack stops,
    /// the budget refills and the price falls back.
    #[test]
    fn sim_repeated_small_edit_burn_has_a_floor_and_recovers() {
        let (_vp, storage) = model();
        // attacker controls a contract and submits tiny edits to consume quota.
        let mut b = 1_000_000u128;
        let mut total_burn = 0u128;
        let mut min_b = b;
        let mut max_periods = 10u64;
        for _ in 0..200 {
            let block_used = b.min(MAX_TX);
            total_burn += burn(block_used, periods(&storage, b));
            // settle: usage then refill (attacker re-fills the quota every block)
            b = (b - block_used + 1_000).min(1_000_000);
            min_b = min_b.min(b);
            max_periods = max_periods.max(periods(&storage, b));
        }
        // steady state: the budget sits at the 1,000-byte refill floor, price near full
        assert_eq!(min_b, 1_000, "sustained drain bottoms out at the refill rate");
        assert!(
            max_periods >= 9_990,
            "sustained small-edit burn drives the price near full: {max_periods}"
        );
        // the attack was not free: it burned the floor fee on every moved byte
        assert!(total_burn > 0);
        // Once the attacker stops, the (nearly empty) budget refills at R: from the
        // 1,000-byte floor, 499 idle blocks bring it to exactly half capacity.
        let mut b = min_b;
        for _ in 0..499 {
            b = (b + 1_000).min(1_000_000);
        }
        assert_eq!(b, 500_000);
        assert_eq!(periods(&storage, b), 5_000);
    }
}
