use std::any::Any;

use field::Address;
use sys::{Rerr, Ret};

use crate::iface::action::TransferPayload;
use crate::iface::context::Context;
use crate::runtime::GasBuckets;

pub trait P2sh: Send + Sync {
    fn code_conf(&self) -> u8 {
        0
    }
    fn code_stuff(&self) -> &[u8];
    fn witness(&self) -> &[u8];
}

// ================================ Vm ================================
//
//
//
//
//
//
//
//
//

pub enum VmEntry {
    TransferAuthorize {
        owner: Address,
        to: Address,
        action_kind: u16,
        payload: TransferPayload,
    },
    TransferReceive {
        from: Address,
        to: Address,
        action_kind: u16,
        payload: TransferPayload,
    },
    Raw(Box<dyn Any>),
}

pub struct EmptyVm;

impl Vm for EmptyVm {
    fn call(&mut self, _ctx: &mut dyn Context, _entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)> {
        sys::errf!("vm not supported by this chain (no vm assigner registered)")
    }
}

/// VM extension contract owned by `base` and consumed through `Context` and
/// the action dispatcher. The standard Hacash implementation lives in `vm/src`.
/// Defaults below represent optional capabilities: unsupported hooks are no-op,
/// absent runtime configuration is `None`, and deferred work is empty.
pub trait Vm {
    fn call(&mut self, ctx: &mut dyn Context, entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)>;

    /// Optional cooperative execution deadline.  Normal consensus execution
    /// leaves this unset; untrusted sandbox calls set it before entering the
    /// interpreter so long-running bytecode can be aborted at instruction
    /// boundaries.
    fn set_deadline(&mut self, _deadline: Option<std::time::Instant>) {}

    fn snapshot_volatile(&mut self) -> Box<dyn Any> {
        Box::new(())
    }
    fn restore_volatile(&mut self, _snap: Box<dyn Any>) {}
    fn rollback_volatile_preserve_warm_and_gas(&mut self) {}
    fn invalidate_contract_cache(&mut self, _addr: &Address) {}
    fn runtime_config(&mut self) -> Option<Box<dyn Any>> {
        None
    }
    fn drain_deferred(&mut self, _ctx: &mut dyn Context) -> Rerr {
        Ok(())
    }
}
