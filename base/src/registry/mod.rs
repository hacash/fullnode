//! `Registry` ——
//!
//! - binary/json codec
//! - block hasher / vm assigner / action hooks
//! - VM host metadata

use std::any::Any;
use std::sync::Arc;
use sys::{Rerr, Ret};

use crate::runtime::Env;
use crate::{ActionRef, BlockRef, Context, StateChunkRef, TxRef, Vm};

pub const HASH_SIZE: usize = 32;

pub type BlockHasherFn = fn(u64, &[u8]) -> [u8; HASH_SIZE];
pub type VmAssignFn = fn(&dyn ExecutionServices, u64) -> Box<dyn Vm>;
pub type ActionCreateFn = fn(&dyn BinaryCodecs, u16, &[u8]) -> Ret<(ActionRef, usize)>;
pub type TxCreateFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<(TxRef, usize)>;
pub type BlockCreateFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<(BlockRef, usize)>;
/// ""—— header  pipeline feeder
pub type BlockSizeFn = fn(&dyn BinaryCodecs, &[u8]) -> Ret<usize>;
pub type ActionJsonDecodeFn = fn(&dyn CodecRegistry, u16, &str) -> Ret<ActionRef>;
pub type TxJsonDecodeFn = fn(&dyn BinaryCodecs, u8, &str) -> Ret<TxRef>;
pub type ContextCreateFn =
    fn(Env, Arc<dyn ExecutionServices>, StateChunkRef, TxRef, i64) -> Ret<Box<dyn Context>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmHostCallKind {
    Action,
    Env,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmValueType {
    Nil,
    Bool,
    U8,
    U64,
    Address,
    Bytes,
}

/// When a registered host action / env / view may be invoked from the VM.
///
/// Enforced by the interpreter (`ensure_act_allowed`), not only as metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmHostAllowedPolicy {
    /// Any non-pure call site.
    Any,
    /// Main entry, Edit effect, call depth 0 only (e.g. transfer EXTACTION).
    TopOnly,
    /// Nested Edit calls only (not top-level Main depth 0).
    CallOnly,
    /// Edit or View call sites (not Pure).
    ViewOnly,
}

#[derive(Clone, Debug)]
pub struct VmHostActionDef {
    pub id: u8,
    pub name: &'static str,
    pub kind: VmHostCallKind,
    pub ret: VmValueType,
    pub argc: usize,
    pub pass_body: bool,
    pub have_retv: bool,
    pub allowed_policy: VmHostAllowedPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmExecutionParams {
    pub contract_store_perm_periods: u64,
    pub initial_fee_purity_floor: u64,
    /// Height-gated floor reductions: `(activation_height, next_floor)`.
    pub fee_purity_reductions: &'static [(u64, u64)],
}

impl VmExecutionParams {
    /// Fee purity floor selected by the consensus schedule at `height`.
    pub fn fee_purity_floor_at(&self, height: u64) -> u64 {
        let mut floor = self.initial_fee_purity_floor;
        for &(activation, next) in self.fee_purity_reductions {
            if height >= activation && next < floor {
                floor = next;
            }
        }
        floor
    }

    /// Effective fee purity floor at `height`, then `raw.max(floor)`.
    pub fn effective_fee_purity(&self, height: u64, raw: u64) -> u64 {
        raw.max(self.fee_purity_floor_at(height))
    }
}

pub trait BinaryCodecs: Send + Sync {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)>;
    fn decode_action_exact(&self, buf: &[u8]) -> Ret<ActionRef>;
    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)>;
    fn decode_transaction_exact(&self, buf: &[u8]) -> Ret<TxRef>;
    fn decode_block(&self, buf: &[u8]) -> Ret<(BlockRef, usize)>;
    fn decode_block_exact(&self, buf: &[u8]) -> Ret<BlockRef>;
    fn peek_block_size(&self, buf: &[u8]) -> Ret<usize>;
    fn block_hash(&self, height: u64, stuff: &[u8]) -> [u8; HASH_SIZE];
    fn block_hasher_fn(&self) -> BlockHasherFn;
}

pub trait JsonCodecs: Send + Sync {
    fn decode_tx_json(&self, ty: u8, json: &str) -> Ret<Option<TxRef>>;
    fn decode_action_json(&self, kind: u16, json: &str) -> Ret<Option<ActionRef>>;
}

/// View passed to JSON creators. Recursive/dynamic JSON actions need both
/// binary decoding (for legacy `body` fields) and JSON registry dispatch.
pub trait CodecRegistry: BinaryCodecs + JsonCodecs {}

impl<T: BinaryCodecs + JsonCodecs + ?Sized> CodecRegistry for T {}

pub trait ExecutionServices: BinaryCodecs + JsonCodecs {
    fn assign_vm(&self, height: u64) -> Option<Box<dyn Vm>>;
    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<&VmHostActionDef>;
    fn vm_host_defs(&self, kind: VmHostCallKind) -> Vec<&VmHostActionDef>;
    fn vm_params(&self) -> Ret<&VmExecutionParams>;
    /// Concrete protocol-owned profile selected during registry assembly.
    fn execution_profile(&self) -> Ret<&'static (dyn Any + Send + Sync)>;
    fn create_context(
        self: Arc<Self>,
        env: Env,
        chunk: StateChunkRef,
        tx: TxRef,
    ) -> Ret<Box<dyn Context>>;
}

/// Registration-time write surface implemented by the application composition
/// root. Protocol, consensus and VM crates contribute components without
/// depending on the concrete registry container.
pub trait RegistryWriter {
    fn set_block_creator(&mut self, f: BlockCreateFn) -> Rerr;
    fn set_block_sizer(&mut self, f: BlockSizeFn) -> Rerr;
    fn set_vm_assigner(&mut self, f: VmAssignFn) -> Rerr;
    fn register_tx(&mut self, ty: u8, f: TxCreateFn) -> Rerr;
    fn register_tx_json(&mut self, ty: u8, f: TxJsonDecodeFn) -> Rerr;
    fn register_action(&mut self, kinds: &[u16], f: ActionCreateFn) -> Rerr;
    fn register_action_json(&mut self, kinds: &[u16], f: ActionJsonDecodeFn) -> Rerr;
    fn register_vm_host_def(&mut self, def: VmHostActionDef) -> Rerr;
    fn set_context_creator(&mut self, f: ContextCreateFn, gas_budget: i64) -> Rerr;
    fn set_vm_params(&mut self, params: VmExecutionParams) -> Rerr;
    fn set_execution_profile(&mut self, profile: &'static (dyn Any + Send + Sync)) -> Rerr;
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
