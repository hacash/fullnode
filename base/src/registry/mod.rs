//! `Registry`: binary/json codec, block hasher / vm assigner / action hooks, VM host metadata.

use field::{
    Amount, Decode, Uint1, Uint2, Uint12, json_expect_quoted_decoded, json_expect_unquoted,
    json_object_entries,
};
use std::any::Any;
#[cfg(feature = "execute")]
use std::sync::Arc;
use sys::{Rerr, Ret, normalf};

#[cfg(feature = "execute")]
use crate::runtime::Env;
use crate::{ActionRef, BlockRef, TxRef};
#[cfg(feature = "execute")]
use crate::{Context, StateChunkRef, Vm};

pub const HASH_SIZE: usize = 32;

/// Chain fee/gas pricing unit: `fee_purity` (pricing sub-unit per billing byte)
/// and the fee-purity floor are expressed in this sub-unit — `UNIT_SHUO` = 232
/// = 10⁻¹⁶ HAC. Protocol settle amounts (gas burn, protocol-cost minima) are
/// ceiled to [`GAS_SETTLEMENT_UNIT`] before they are written to a balance.
pub const FEE_PRICING_UNIT: u8 = field::UNIT_SHUO;

/// Unit actually written to HAC balances and `*_238` accumulators: `UNIT_238`
/// = 238 = 10⁻¹⁰ HAC. Pricing may be finer ([`FEE_PRICING_UNIT`]); every
/// protocol-produced HAC amount is rounded up to a whole number of this unit
/// so balances never receive sub-238 dust.
pub const GAS_SETTLEMENT_UNIT: u8 = field::UNIT_238;

const _: () = assert!(GAS_SETTLEMENT_UNIT >= FEE_PRICING_UNIT);

/// `10^(GAS_SETTLEMENT_UNIT − FEE_PRICING_UNIT)` = 1_000_000: one u238 equals
/// this many u232 pricing sub-units.
pub const SETTLEMENT_SCALE: u128 =
    10u128.pow((GAS_SETTLEMENT_UNIT - FEE_PRICING_UNIT) as u32);

/// Ceil a pricing-unit (u232) count up to a whole number of settlement units
/// (u238): `ceil(v / SETTLEMENT_SCALE)`.
pub fn ceil_pricing_to_settlement(v_pricing: u128) -> u128 {
    v_pricing.div_ceil(SETTLEMENT_SCALE)
}

/// Protocol-produced HAC amount: `v_pricing` u232, rounded up to an integer
/// number of [`GAS_SETTLEMENT_UNIT`] (u238).
pub fn settlement_amount(v_pricing: u128) -> Amount {
    Amount::coin_u128(ceil_pricing_to_settlement(v_pricing), GAS_SETTLEMENT_UNIT)
}

pub type BlockHasherFn = fn(u64, &[u8]) -> [u8; HASH_SIZE];
#[cfg(feature = "execute")]
pub type VmAssignFn = fn(&dyn ExecutionServices, u64) -> Box<dyn Vm>;
pub type ActionCreateFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<(ActionRef, usize)>;
pub type TxCreateFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<(TxRef, usize)>;
#[cfg(feature = "execute")]
pub type BlockCreateFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<(BlockRef, usize)>;
pub type ActionJsonDecodeFn = fn(&dyn CodecRegistry, &str) -> Ret<ActionRef>;

/// Opaque chain profile selected by an application composition root. `base` owns no
/// concrete network type; a concrete chain exposes typed accessors in its parameter crate.
pub trait ExecutionProfile: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Send + Sync> ExecutionProfile for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Complete codec binding for one action kind, constructed by composition roots while
/// registering their profile; no global catalog is implied.
#[derive(Clone, Copy)]
pub struct ActionCodecBinding {
    pub schema: crate::ActionSchema,
    /// Consensus placement rule of the action (`ActScope::CALL_ONLY` actions are
    /// contract-internal syscalls that can never appear as ordinary transaction
    /// actions). Static, so selection surfaces (e.g. the SDK) can exclude them
    /// without kind arithmetic or decoding.
    pub scope: crate::ActScope,
    pub decode_wire: ActionCreateFn,
    pub decode_json: ActionJsonDecodeFn,
}

/// Transaction codec registration. Transactions are a separate wire namespace
/// and must not be hidden in an action table.
#[derive(Clone, Copy)]
pub struct TxCodecBinding {
    pub ty: u8,
    pub decode_wire: TxCreateFn,
}

/// Construct an action binding. JSON encoding lives on `Action: ToJSON`, not
/// in the binding. The one-argument form uses derived wire + regular JSON
/// (`Default + FromJSON`). The two-argument form supplies a custom wire
/// decoder and still uses regular JSON (HacdMint). The three-argument form
/// supplies custom wire and custom JSON (AST).
#[macro_export]
macro_rules! action_codec_binding {
    ($ty:ty) => {
        $crate::ActionCodecBinding {
            schema: <$ty as $crate::ActionSchemaProvider>::ACTION_SCHEMA,
            scope: <$ty as $crate::ActionScopeProvider>::SCOPE,
            decode_wire: $crate::create_regular_action::<$ty>,
            decode_json: $crate::decode_regular_action_json::<$ty>,
        }
    };
    ($ty:ty, $wire:path) => {
        $crate::ActionCodecBinding {
            schema: <$ty as $crate::ActionSchemaProvider>::ACTION_SCHEMA,
            scope: <$ty as $crate::ActionScopeProvider>::SCOPE,
            decode_wire: $wire,
            decode_json: $crate::decode_regular_action_json::<$ty>,
        }
    };
    ($ty:ty, $wire:path, $json:path) => {
        $crate::ActionCodecBinding {
            schema: <$ty as $crate::ActionSchemaProvider>::ACTION_SCHEMA,
            scope: <$ty as $crate::ActionScopeProvider>::SCOPE,
            decode_wire: $wire,
            decode_json: $json,
        }
    };
}

/// Shared validated storage used by both native and SDK codec containers.
/// One action row holds schema, scope, and both decoders (`ActionCodecBinding`);
/// binary decode, JSON decode, and `action_kinds()` read that single table.
/// Transactions stay a separate wire namespace. Tables are small (a few tx
/// types, tens of actions): linear `Vec` lookup instead of `HashMap` keeps
/// the hash-table machinery out of the wasm graph.
#[derive(Default)]
pub struct WireCodecTable {
    transactions: Vec<(u8, TxCreateFn)>,
    actions: Vec<ActionCodecBinding>,
}

impl WireCodecTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_tx(&mut self, binding: TxCodecBinding) -> Rerr {
        if self.transactions.iter().any(|(ty, _)| *ty == binding.ty) {
            return sys::errf!("transaction type {} already registered", binding.ty);
        }
        self.transactions.push((binding.ty, binding.decode_wire));
        Ok(())
    }

    pub fn add_action(&mut self, binding: ActionCodecBinding) -> Rerr {
        let kind = binding.schema.kind;
        if self.actions.iter().any(|entry| entry.schema.kind == kind) {
            return sys::errf!("action kind {} already registered", kind);
        }
        self.actions.push(binding);
        Ok(())
    }

    pub fn tx(&self, ty: u8) -> Option<TxCreateFn> {
        self.transactions
            .iter()
            .find(|(k, _)| *k == ty)
            .map(|(_, f)| *f)
    }

    pub fn action(&self, kind: u16) -> Option<ActionCreateFn> {
        self.actions
            .iter()
            .find(|entry| entry.schema.kind == kind)
            .map(|entry| entry.decode_wire)
    }

    pub fn action_json(&self, kind: u16) -> Option<ActionJsonDecodeFn> {
        self.actions
            .iter()
            .find(|entry| entry.schema.kind == kind)
            .map(|entry| entry.decode_json)
    }

    pub fn decode_action(&self, host: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(ActionRef, usize)> {
        let (kind, _) = Uint2::decode(buf)?;
        let kind = kind.uint();
        match self.action(kind) {
            Some(codec) => codec(host, buf),
            None => normalf!("action kind {} not registered", kind),
        }
    }

    pub fn decode_transaction(&self, host: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(TxRef, usize)> {
        let (ty, _) = Uint1::decode(buf)?;
        let ty = ty.uint();
        match self.tx(ty) {
            Some(codec) => codec(host, buf),
            None => normalf!("transaction type {} not registered", ty),
        }
    }

    pub fn decode_action_json(&self, host: &dyn CodecRegistry, json: &str) -> Ret<ActionRef> {
        let entries = json_object_entries(json)?;
        let kind_raw = entries
            .iter()
            .find(|(key, _)| *key == "kind")
            .map(|(_, value)| *value)
            .ok_or_else(|| sys::Error::normal("action JSON missing kind"))?;
        let kind = match self.action_kind_from_json(kind_raw)? {
            Some(kind) => kind,
            None => return normalf!("action kind/name {} not registered", kind_raw),
        };
        match self.action_json(kind) {
            Some(codec) => codec(host, json),
            None => normalf!("action kind {} not registered", kind),
        }
    }

    /// Resolve the `kind` value of an action JSON object: a numeric id or a
    /// registered action name (quoted or bare). `None` means the value is
    /// neither a number nor any registered name.
    fn action_kind_from_json(&self, kind_raw: &str) -> Ret<Option<u16>> {
        if let Ok(raw) = json_expect_unquoted(kind_raw) {
            if let Ok(kind) = raw.parse::<u16>() {
                return Ok(Some(kind));
            }
            return self.action_kind_by_name(raw.trim());
        }
        if let Ok(name) = json_expect_quoted_decoded(kind_raw) {
            return self.action_kind_by_name(name.trim());
        }
        Ok(None)
    }

    fn action_kind_by_name(&self, name: &str) -> Ret<Option<u16>> {
        Ok(self
            .actions
            .iter()
            .find(|binding| binding.schema.name == name)
            .map(|binding| binding.schema.kind))
    }

    pub fn tx_types(&self) -> Vec<u8> {
        let mut values: Vec<_> = self.transactions.iter().map(|(ty, _)| *ty).collect();
        values.sort_unstable();
        values
    }

    pub fn action_kinds(&self) -> Vec<u16> {
        let mut values: Vec<_> = self.actions.iter().map(|entry| entry.schema.kind).collect();
        values.sort_unstable();
        values
    }
}
#[cfg(feature = "execute")]
pub type ContextCreateFn =
    fn(Env, Arc<dyn ExecutionServices>, StateChunkRef, TxRef) -> Ret<Box<dyn Context>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg(feature = "execute")]
pub enum VmHostCallKind {
    Action,
    Env,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "execute")]
pub enum VmValueType {
    Nil,
    Bool,
    U8,
    U16,
    U64,
    Address,
    Bytes,
}

/// When a registered host action / env / view may be invoked from the VM. Enforced by
/// the interpreter (`ensure_act_allowed`), not only as metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "execute")]
pub enum VmHostAllowedPolicy {
    /// Any non-pure call site.
    Any,
    /// Main entry, Edit effect, call depth 0 only (e.g. transfer EXTACTION).
    TopOnly,
    /// Edit or View call sites (not Pure).
    ViewOnly,
}

#[derive(Clone, Debug)]
#[cfg(feature = "execute")]
pub struct VmHostActionDef {
    pub id: u8,
    pub name: &'static str,
    pub kind: VmHostCallKind,
    pub ret: VmValueType,
    /// Source-level function arity used by compilers and capability introspection.
    /// Runtime body decoding remains authoritative for the wire ABI.
    pub argc: usize,
    pub allowed_policy: VmHostAllowedPolicy,
}

#[cfg(feature = "execute")]
impl VmHostActionDef {
    /// ACTION host: the opcode id is the action kind itself (`id = kind`), so a
    /// kind above `0xff` is rejected instead of silently truncating into the u8 id.
    pub fn action_host(kind: u16, name: &'static str, argc: usize) -> Ret<Self> {
        if kind > 0xff {
            return sys::errf!(
                "VM ACTION host {} kind {:#06x} cannot fit the u8 opcode id",
                name,
                kind
            );
        }
        Ok(Self {
            id: kind as u8,
            name,
            kind: VmHostCallKind::Action,
            ret: VmValueType::Nil,
            argc,
            allowed_policy: VmHostAllowedPolicy::TopOnly,
        })
    }

    /// ACTENV host: kinds live in the 0x07xx opcode space; the id is the low byte.
    pub fn env_host(kind: u16, name: &'static str, ret: VmValueType) -> Ret<Self> {
        if kind >> 8 != 0x07 {
            return sys::errf!(
                "VM ACTENV host {} kind {:#06x} must be in the 0x07xx opcode space",
                name,
                kind
            );
        }
        Ok(Self {
            id: kind as u8,
            name,
            kind: VmHostCallKind::Env,
            ret,
            argc: 0,
            allowed_policy: VmHostAllowedPolicy::Any,
        })
    }

    /// ACTVIEW host: kinds live in the 0x06xx opcode space; the id is the low byte.
    pub fn view_host(kind: u16, name: &'static str, ret: VmValueType, argc: usize) -> Ret<Self> {
        if kind >> 8 != 0x06 {
            return sys::errf!(
                "VM ACTVIEW host {} kind {:#06x} must be in the 0x06xx opcode space",
                name,
                kind
            );
        }
        Ok(Self {
            id: kind as u8,
            name,
            kind: VmHostCallKind::View,
            ret,
            argc,
            allowed_policy: VmHostAllowedPolicy::ViewOnly,
        })
    }

    /// Validate fields constrained by the ACTION / ACTENV / ACTVIEW opcodes. Body consumption
    /// and stack output are opcode semantics, deliberately not configurable host-definition fields.
    pub fn validate_opcode_abi(&self) -> Rerr {
        match self.kind {
            VmHostCallKind::Action if self.ret != VmValueType::Nil => sys::errf!(
                "VM ACTION host {}/{} must have Nil return type",
                self.name,
                self.id
            ),
            VmHostCallKind::Env if self.argc != 0 => sys::errf!(
                "VM ACTENV host {}/{} must have zero source arguments",
                self.name,
                self.id
            ),
            _ => Ok(()),
        }
    }
}

/// Empty gas-budget vocabulary: every bytecode decodes to 0 (limits disabled).
/// Test/stub `VmExecutionParams` use this; production profiles inject the chain table.
pub const GAS_BUDGET_LOOKUP_NONE: [u32; 256] = [0; 256];

/// Rule identifier of the linear limited-capacity storage discount curve
/// (`contract-storage-fee-budget-design.md` §3). v1: step curve with an all-or-nothing
/// per-tx discount and no congestion penalty above full price.
pub const CONTRACT_STORAGE_RULE_V1: u8 = 1;

/// Height-gated contract storage fee discount parameters (§3.1, §7.1).
///
/// The only governance surface is the append-only `supplement_schedule` of
/// `(activation_height, R bytes/block)` pairs; capacity is derived as `C = T × R`.
/// No per-block history, EMA, or retune-effect snapshots are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContractStorageFeeParams {
    /// Curve/rule version identifier; only `CONTRACT_STORAGE_RULE_V1` exists.
    pub rule_version: u8,
    /// `H0`: first height where the discount rules apply (a multiple of `T`).
    pub activation_height: u64,
    /// `T`: target fill period in blocks (v1: 1000).
    pub target_capacity_blocks: u64,
    /// `N`: number of linear curve steps (v1: 1000).
    pub curve_steps: u64,
    /// `P_min`: discount floor in periods (v1: 10); full price is `P_min × N`.
    pub period_floor: u64,
    /// `K_max`: per-block cap on discount-consumed billed bytes (§3.1).
    pub max_block_discount_bytes: u64,
    /// Append-only `(activation_height, R)` supplement schedule. The first entry
    /// must sit at `H0`; heights strictly increase by multiples of `T`, `R` strictly
    /// increases (capacity expansion family only, §7.1).
    pub supplement_schedule: &'static [(u64, u64)],
}

impl ContractStorageFeeParams {
    /// Disabled parameters: no schedule entry means the discount mechanism never
    /// activates and contract storage stays on the fixed `contract_store_perm_periods`
    /// rule (test/codec profiles use this).
    pub const fn disabled() -> Self {
        Self {
            rule_version: CONTRACT_STORAGE_RULE_V1,
            activation_height: 0,
            target_capacity_blocks: 0,
            curve_steps: 0,
            period_floor: 0,
            max_block_discount_bytes: 0,
            supplement_schedule: &[],
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.supplement_schedule.is_empty()
    }

    /// Whether the discount rules apply at `height`.
    pub fn is_active_at(&self, height: u64) -> bool {
        !self.is_disabled() && height >= self.activation_height
    }

    /// Active `(activation_height, R)` schedule entry at `height`, `None` before `H0`.
    pub fn active_entry(&self, height: u64) -> Option<(u64, u64)> {
        if !self.is_active_at(height) {
            return None;
        }
        self.supplement_schedule
            .iter()
            .filter(|(h, _)| *h <= height)
            .last()
            .copied()
    }

    /// Active per-block supplement rate `R` at `height`.
    pub fn active_rate(&self, height: u64) -> Option<u64> {
        self.active_entry(height).map(|(_, r)| r)
    }

    /// Derived capacity ceiling `C = T × R` at `height` (checked, §7.1).
    pub fn capacity_at(&self, height: u64) -> Ret<u128> {
        let Some((_, rate)) = self.active_entry(height) else {
            return sys::errf!(
                "contract storage fee schedule has no active entry at height {}",
                height
            );
        };
        u128::from(rate)
            .checked_mul(u128::from(self.target_capacity_blocks))
            .ok_or_else(|| {
                sys::Error::abort(format!(
                    "contract storage capacity overflow: T {} × R {}",
                    self.target_capacity_blocks, rate
                ))
                .with_code("core_failed")
            })
    }

    /// Block-level discount quota `K = min(B_start, K_max)` (§3.1/§4.2).
    pub fn block_quota(&self, remaining_bytes: u128) -> u128 {
        remaining_bytes.min(u128::from(self.max_block_discount_bytes))
    }

    /// Linear integer price curve (§3.3):
    /// `used = C - B; step = clamp(ceil(N × used / C), 1, N); periods = P_min × step`.
    /// Bounded inputs; overflow is a deterministic consensus error, never truncation.
    pub fn discount_periods(
        &self,
        remaining_bytes: u128,
        capacity_bytes: u128,
        max_periods: u64,
    ) -> Ret<u64> {
        if capacity_bytes == 0 {
            return Err(sys::Error::abort("contract storage capacity is zero")
                .with_code("core_failed"));
        }
        let remaining = remaining_bytes.min(capacity_bytes);
        let used = capacity_bytes - remaining;
        let steps = u128::from(self.curve_steps);
        let step = if used == 0 {
            1u128
        } else {
            // step = ceil(N × used / C), evaluated as 1 + (N × used − 1) / C so the
            // ceiling never needs an overflowing `+ C − 1` (A18: overflow anywhere is
            // a deterministic consensus error, never a silent clamp).
            let numer = used.checked_mul(steps).ok_or_else(|| {
                sys::Error::abort(format!(
                    "contract storage price curve overflow: used {used} × steps {steps}"
                ))
                .with_code("core_failed")
            })?;
            (((numer - 1) / capacity_bytes) + 1).min(steps)
        };
        let periods = u128::from(self.period_floor)
            .checked_mul(step)
            .ok_or_else(|| {
                sys::Error::abort("contract storage price curve overflow in periods")
                    .with_code("core_failed")
            })?;
        let max = u128::from(max_periods);
        if periods > max {
            return Err(sys::Error::abort(format!(
                "contract storage curve exceeds full price: periods {periods} > P_max {max}"
            ))
            .with_code("core_failed"));
        }
        Ok(periods as u64)
    }

    /// Consensus validation of the parameter profile (§8.A2): append-only schedule
    /// monotonicity, capacity derivation, and the fixed v1 curve constants against
    /// the full-price periods `P_max`.
    pub fn validate(&self, max_periods: u64) -> Rerr {
        if self.is_disabled() {
            return Ok(());
        }
        if self.rule_version != CONTRACT_STORAGE_RULE_V1 {
            return sys::errf!(
                "unsupported contract storage rule version {}",
                self.rule_version
            );
        }
        // v1 freezes the curve: 1000 steps of 10 periods, full price = P_max.
        if self.curve_steps != 1000 || self.period_floor != 10 {
            return sys::errf!(
                "contract storage rule v1 requires curve_steps=1000 and period_floor=10, got {}/{}",
                self.curve_steps,
                self.period_floor
            );
        }
        if u64::from(self.period_floor).checked_mul(self.curve_steps) != Some(max_periods) {
            return sys::errf!(
                "contract storage period_floor {} × curve_steps {} must equal P_max {}",
                self.period_floor,
                self.curve_steps,
                max_periods
            );
        }
        if self.target_capacity_blocks == 0 {
            return sys::errf!("contract storage target_capacity_blocks must be positive");
        }
        if self.activation_height == 0 || self.activation_height % self.target_capacity_blocks != 0
        {
            return sys::errf!(
                "contract storage activation height {} must be a positive multiple of T {}",
                self.activation_height,
                self.target_capacity_blocks
            );
        }
        if self.max_block_discount_bytes == 0 {
            return sys::errf!("contract storage max_block_discount_bytes must be positive");
        }
        let mut prev: Option<(u64, u64)> = None;
        for &(height, rate) in self.supplement_schedule {
            if rate == 0 {
                return sys::errf!(
                    "contract storage schedule rate must be positive (height {})",
                    height
                );
            }
            if height % self.target_capacity_blocks != 0 {
                return sys::errf!(
                    "contract storage schedule height {} must be a multiple of T {}",
                    height,
                    self.target_capacity_blocks
                );
            }
            match prev {
                None => {
                    if height != self.activation_height {
                        return sys::errf!(
                            "contract storage schedule must start at activation height {} (H0), got {}",
                            self.activation_height,
                            height
                        );
                    }
                }
                Some((ph, pr)) => {
                    // §7.1: strictly increasing heights and rates; governance-sized
                    // expansions use 1000-byte rate steps; capacity stays derivable
                    // and strictly larger (no shrink path).
                    if height <= ph || rate <= pr {
                        return sys::errf!(
                            "contract storage schedule entry ({}, {}) violates monotonicity after ({}, {})",
                            height,
                            rate,
                            ph,
                            pr
                        );
                    }
                    if (rate - pr) % 1000 != 0 {
                        return sys::errf!(
                            "contract storage rate delta {}-{} must be a multiple of 1000 bytes/block",
                            rate,
                            pr
                        );
                    }
                }
            }
            if u128::from(rate) >= u128::from(self.max_block_discount_bytes) {
                // K_max > R keeps the budget able to drain, so prices can recover
                // (§3.1); equal or lower rates are rejected.
                return sys::errf!(
                    "contract storage K_max {} must exceed every schedule rate (found R {})",
                    self.max_block_discount_bytes,
                    rate
                );
            }
            let capacity = u128::from(rate)
                .checked_mul(u128::from(self.target_capacity_blocks))
                .ok_or_else(|| {
                    sys::Error::fault(format!(
                        "contract storage capacity T {} × R {} overflows",
                        self.target_capacity_blocks, rate
                    ))
                })?;
            if capacity > Uint12::MAX {
                return sys::errf!(
                    "contract storage capacity {} exceeds Uint12 maximum {}",
                    capacity,
                    Uint12::MAX
                );
            }
            prev = Some((height, rate));
        }
        Ok(())
    }
}

impl Default for ContractStorageFeeParams {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmExecutionParams {
    pub contract_store_perm_periods: u64,
    /// Limited-capacity discount schedule over the full-price `contract_store_perm_periods`.
    pub contract_storage_fee: ContractStorageFeeParams,
    /// Minimum fee purity in the chain pricing unit (`FEE_PRICING_UNIT`, u232),
    /// enforced per billing byte by `GasPrice` settle and the contract protocol fee.
    pub initial_fee_purity_floor: u64,
    /// Height-gated floor reductions: `(activation_height, next_floor)`, floors in
    /// the same pricing unit as `initial_fee_purity_floor`.
    pub fee_purity_reductions: &'static [(u64, u64)],
    /// Chain gas-budget vocabulary: bytecode → budget units. Engine pricing
    /// constants stay in the VM; only this table and the four budget bytes
    /// below are chain-injected.
    pub gas_budget_lookup: &'static [u32; 256],
    pub tx_gas_budget_cap_byte: u8,
    pub compute_limit_byte: u8,
    pub resource_limit_byte: u8,
    pub storage_limit_byte: u8,
}

/// Fee purity floor selected by the consensus schedule at `height`, expressed in the
/// chain pricing unit (`FEE_PRICING_UNIT` = u232) — single computation shared by
/// `VmExecutionParams::fee_purity_floor_at` and the SDK's height-aware review fact.
pub fn fee_purity_floor_at(initial: u64, reductions: &[(u64, u64)], height: u64) -> u64 {
    let mut floor = initial;
    for &(activation, next) in reductions {
        if height >= activation && next < floor {
            floor = next;
        }
    }
    floor
}

impl VmExecutionParams {
    /// Fee purity floor selected by the consensus schedule at `height`.
    pub fn fee_purity_floor_at(&self, height: u64) -> u64 {
        fee_purity_floor_at(
            self.initial_fee_purity_floor,
            self.fee_purity_reductions,
            height,
        )
    }

    /// Effective fee purity at `height` in the chain pricing unit (u232): `raw.max(floor)`.
    /// Shared by mempool admission and consensus protocol-fee pricing; callers that
    /// then multiply by bytes/periods/gas widen the product to `u128`.
    pub fn effective_fee_purity(&self, height: u64, raw: u64) -> u64 {
        raw.max(self.fee_purity_floor_at(height))
    }

    /// Consensus validation of the whole profile: the contract storage discount
    /// schedule must satisfy the §7.1 monotonicity/derivation gates against the
    /// full-price `contract_store_perm_periods`. Composition roots call this when
    /// registering a profile, so an invalid table cannot silently boot a node.
    pub fn validate(&self) -> Rerr {
        self.contract_storage_fee
            .validate(self.contract_store_perm_periods)
    }

    #[inline(always)]
    pub const fn decode_gas_budget(&self, byte: u8) -> i64 {
        self.gas_budget_lookup[byte as usize] as i64
    }
}

impl Default for VmExecutionParams {
    fn default() -> Self {
        Self {
            contract_store_perm_periods: 0,
            contract_storage_fee: ContractStorageFeeParams::disabled(),
            initial_fee_purity_floor: 0,
            fee_purity_reductions: &[],
            gas_budget_lookup: &GAS_BUDGET_LOOKUP_NONE,
            tx_gas_budget_cap_byte: 0,
            compute_limit_byte: 0,
            resource_limit_byte: 0,
            storage_limit_byte: 0,
        }
    }
}

fn require_exact<T>(what: &str, decoded: Ret<(T, usize)>, total: usize) -> Ret<T> {
    let (obj, used) = decoded?;
    if used != total {
        return normalf!(
            "{what} parse length mismatch: consumed {used} but payload length is {total}"
        );
    }
    Ok(obj)
}

/// Test codec host: implements `BinaryCodecs` + `JsonCodecs` by forwarding
/// action/tx decode to `$ty.table: WireCodecTable`. The caller constructs `$ty`
/// and fills `table`. Block methods are stubs (no block catalog).
///
/// `BinaryCodecs` has six required methods (`decode_action`, `decode_transaction`,
/// `decode_block`, `peek_block_size`, `block_hash`, `block_hasher_fn`); the
/// `*_exact` helpers stay on the trait defaults.
#[macro_export]
macro_rules! test_codec_host {
    ($ty:ty) => {
        impl $crate::BinaryCodecs for $ty {
            fn decode_action(&self, buf: &[u8]) -> sys::Ret<($crate::ActionRef, usize)> {
                self.table.decode_action(self, buf)
            }

            fn decode_transaction(&self, buf: &[u8]) -> sys::Ret<($crate::TxRef, usize)> {
                self.table.decode_transaction(self, buf)
            }

            fn decode_block(&self, _buf: &[u8]) -> sys::Ret<($crate::BlockRef, usize)> {
                sys::normalf!("no block decode")
            }

            fn peek_block_size(&self, buf: &[u8]) -> sys::Ret<usize> {
                self.decode_block(buf).map(|(_, used)| used)
            }

            fn block_hash(&self, _height: u64, _stuff: &[u8]) -> [u8; $crate::HASH_SIZE] {
                [0u8; $crate::HASH_SIZE]
            }

            fn block_hasher_fn(&self) -> $crate::BlockHasherFn {
                |_, _| [0u8; $crate::HASH_SIZE]
            }
        }

        impl $crate::JsonCodecs for $ty {
            fn decode_action_json(&self, json: &str) -> sys::Ret<$crate::ActionRef> {
                self.table.decode_action_json(self, json)
            }
        }
    };
}

pub trait BinaryCodecs: Send + Sync {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)>;
    fn decode_action_exact(&self, buf: &[u8]) -> Ret<ActionRef> {
        require_exact("action", self.decode_action(buf), buf.len())
    }
    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)>;
    fn decode_transaction_exact(&self, buf: &[u8]) -> Ret<TxRef> {
        require_exact("transaction", self.decode_transaction(buf), buf.len())
    }
    fn decode_block(&self, buf: &[u8]) -> Ret<(BlockRef, usize)>;
    fn decode_block_exact(&self, buf: &[u8]) -> Ret<BlockRef> {
        require_exact("block", self.decode_block(buf), buf.len())
    }
    fn peek_block_size(&self, buf: &[u8]) -> Ret<usize>;
    fn block_hash(&self, height: u64, stuff: &[u8]) -> [u8; HASH_SIZE];
    fn block_hasher_fn(&self) -> BlockHasherFn;
}

pub trait JsonCodecs: Send + Sync {
    fn decode_action_json(&self, json: &str) -> Ret<ActionRef>;
}

/// View passed to JSON creators. Recursive/dynamic JSON actions (AST children)
/// need binary decoding and JSON registry dispatch.
pub trait CodecRegistry: BinaryCodecs + JsonCodecs {}

impl<T: BinaryCodecs + JsonCodecs + ?Sized> CodecRegistry for T {}

#[cfg(feature = "execute")]
pub trait ExecutionServices: BinaryCodecs + JsonCodecs {
    fn assign_vm(&self, height: u64) -> Option<Box<dyn Vm>>;
    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<&VmHostActionDef>;
    fn vm_params(&self) -> Ret<&VmExecutionParams>;
    /// Concrete protocol-owned profile selected during registry assembly.
    fn execution_profile(&self) -> Ret<&'static dyn ExecutionProfile>;
    fn create_context(
        self: Arc<Self>,
        env: Env,
        chunk: StateChunkRef,
        tx: TxRef,
    ) -> Ret<Box<dyn Context>>;
}

/// Registration-time write surface for crate-owned static wire catalogs (`TX_CODECS` /
/// `ACTION_CODECS`). The SDK does not implement it; execution-only registrations live on `ExecRegistry`.
pub trait WireRegistry {
    fn register_tx_codec(&mut self, binding: TxCodecBinding) -> Rerr;
    fn register_action_codec(&mut self, binding: ActionCodecBinding) -> Rerr;
}

/// Registration-time write surface for execution services (block creator, VM assigner,
/// context creator, VM params, execution profile). Implemented by the composition root only.
#[cfg(feature = "execute")]
pub trait ExecRegistry {
    fn set_block_creator(&mut self, f: BlockCreateFn) -> Rerr;
    fn set_vm_assigner(&mut self, f: VmAssignFn) -> Rerr;
    fn register_vm_host_def(&mut self, def: VmHostActionDef) -> Rerr;
    fn set_context_creator(&mut self, f: ContextCreateFn) -> Rerr;
    fn set_vm_params(&mut self, params: VmExecutionParams) -> Rerr;
    fn set_execution_profile(&mut self, profile: &'static dyn ExecutionProfile) -> Rerr;
}

#[cfg(test)]
mod tests {
    use super::{ContractStorageFeeParams, VmExecutionParams};

    const PARAMS: VmExecutionParams = VmExecutionParams {
        contract_store_perm_periods: 10_000,
        contract_storage_fee: ContractStorageFeeParams::disabled(),
        // Schedule values are in u232: 100/80/50 u238 ≡ 10⁸/8×10⁷/5×10⁷ u232.
        initial_fee_purity_floor: 100_000_000,
        fee_purity_reductions: &[(10, 80_000_000), (20, 50_000_000)],
        gas_budget_lookup: &super::GAS_BUDGET_LOOKUP_NONE,
        tx_gas_budget_cap_byte: 0,
        compute_limit_byte: 0,
        resource_limit_byte: 0,
        storage_limit_byte: 0,
    };

    #[test]
    fn fee_purity_schedule_changes_at_activation_height() {
        assert_eq!(PARAMS.fee_purity_floor_at(9), 100_000_000);
        assert_eq!(PARAMS.fee_purity_floor_at(10), 80_000_000);
        assert_eq!(PARAMS.fee_purity_floor_at(19), 80_000_000);
        assert_eq!(PARAMS.fee_purity_floor_at(20), 50_000_000);
    }

    #[test]
    fn effective_fee_purity_applies_the_scheduled_floor() {
        assert_eq!(PARAMS.effective_fee_purity(20, 40), 50_000_000);
        assert_eq!(PARAMS.effective_fee_purity(20, 60_000_000), 60_000_000);
    }

    #[test]
    fn ceil_pricing_to_settlement_rounds_up_to_u238() {
        assert_eq!(super::SETTLEMENT_SCALE, 1_000_000);
        assert_eq!(super::ceil_pricing_to_settlement(0), 0);
        assert_eq!(super::ceil_pricing_to_settlement(1), 1);
        assert_eq!(super::ceil_pricing_to_settlement(1_000_000), 1);
        assert_eq!(super::ceil_pricing_to_settlement(1_000_001), 2);
        let amt = super::settlement_amount(1);
        assert_eq!(amt.to_unit_u128(field::UNIT_238).unwrap(), 1);
        assert!(amt.unit() >= field::UNIT_238);
    }

    /// The profile-level validator must reject illegal storage fee tables exactly
    /// like the table validator, so a composition root cannot boot them (§A14).
    #[test]
    fn profile_validate_rejects_illegal_storage_schedule() {
        assert!(PARAMS.validate().is_ok(), "disabled profile validates");
        // disabled profile passes even when P_max would be inconsistent
        let good = VmExecutionParams {
            contract_storage_fee: super::ContractStorageFeeParams {
                rule_version: super::CONTRACT_STORAGE_RULE_V1,
                activation_height: 10_000,
                target_capacity_blocks: 1_000,
                curve_steps: 1_000,
                period_floor: 10,
                max_block_discount_bytes: 16_384,
                supplement_schedule: &[(10_000, 1_000)],
            },
            ..PARAMS
        };
        assert!(good.validate().is_ok());
        // shrink / non-monotone / curve-breaking tables fail the profile gate
        let mut shrink = good;
        shrink.contract_storage_fee.supplement_schedule = &[(10_000, 1_000), (20_000, 500)];
        assert!(shrink.validate().is_err());
        let mut floor = good;
        floor.contract_storage_fee.period_floor = 11;
        assert!(floor.validate().is_err());
        let mut kmax = good;
        kmax.contract_storage_fee.max_block_discount_bytes = 999;
        assert!(kmax.validate().is_err());

        let mut zero_rate = good;
        zero_rate.contract_storage_fee.supplement_schedule = &[(10_000, 0)];
        assert!(zero_rate.validate().is_err(), "zero supplement rate is invalid");

        let mut oversized_capacity = good;
        oversized_capacity.contract_storage_fee.target_capacity_blocks = u64::MAX;
        oversized_capacity.contract_storage_fee.activation_height = u64::MAX;
        oversized_capacity.contract_storage_fee.max_block_discount_bytes = u64::MAX;
        oversized_capacity.contract_storage_fee.supplement_schedule =
            &[(u64::MAX, u64::MAX - 1)];
        assert!(
            oversized_capacity.validate().is_err(),
            "capacity must fit the Uint12 persisted representation"
        );
    }
}
