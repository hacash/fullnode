//! `Registry`: binary/json codec, block hasher / vm assigner / action hooks, VM host metadata.

use field::{
    Decode, Uint1, Uint2, json_expect_quoted_decoded, json_expect_unquoted, json_object_entries,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmExecutionParams {
    pub contract_store_perm_periods: u64,
    pub initial_fee_purity_floor: u64,
    /// Height-gated floor reductions: `(activation_height, next_floor)`.
    pub fee_purity_reductions: &'static [(u64, u64)],
}

/// Fee purity floor selected by the consensus schedule at `height` — single computation
/// shared by `VmExecutionParams::fee_purity_floor_at` and the SDK's height-aware review fact.
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

    /// Effective fee purity floor at `height`, then `raw.max(floor)`.
    pub fn effective_fee_purity(&self, height: u64, raw: u64) -> u64 {
        raw.max(self.fee_purity_floor_at(height))
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
    use super::VmExecutionParams;

    const PARAMS: VmExecutionParams = VmExecutionParams {
        contract_store_perm_periods: 10_000,
        initial_fee_purity_floor: 100,
        fee_purity_reductions: &[(10, 80), (20, 50)],
    };

    #[test]
    fn fee_purity_schedule_changes_at_activation_height() {
        assert_eq!(PARAMS.fee_purity_floor_at(9), 100);
        assert_eq!(PARAMS.fee_purity_floor_at(10), 80);
        assert_eq!(PARAMS.fee_purity_floor_at(19), 80);
        assert_eq!(PARAMS.fee_purity_floor_at(20), 50);
    }

    #[test]
    fn effective_fee_purity_applies_the_scheduled_floor() {
        assert_eq!(PARAMS.effective_fee_purity(20, 40), 50);
        assert_eq!(PARAMS.effective_fee_purity(20, 60), 60);
    }
}
