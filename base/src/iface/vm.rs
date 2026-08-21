#[cfg(feature = "execute")]
use std::any::Any;

#[cfg(feature = "execute")]
use field::Address;
#[cfg(feature = "execute")]
use sys::{Rerr, Ret};

#[cfg(feature = "execute")]
use crate::iface::action::TransferPayload;
#[cfg(feature = "execute")]
use crate::iface::context::Context;
#[cfg(feature = "execute")]
use crate::runtime::GasBuckets;

pub trait P2sh: Send + Sync {
    fn code_conf(&self) -> u8 {
        0
    }
    fn code_stuff(&self) -> &[u8];
    fn witness(&self) -> &[u8];
}

// ================================ Vm ================================

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
pub struct EmptyVm;

#[cfg(feature = "execute")]
impl Vm for EmptyVm {
    fn call(&mut self, _ctx: &mut dyn Context, _entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)> {
        sys::errf!("vm not supported by this chain (no vm assigner registered)")
    }
}

/// VM extension contract owned by `base`, consumed via `Context` and the action dispatcher;
/// the standard implementation lives in `vm/src`. Defaults = optional capabilities (no-op hooks).
#[cfg(feature = "execute")]
pub trait Vm {
    fn call(&mut self, ctx: &mut dyn Context, entry: VmEntry) -> Ret<(GasBuckets, Box<dyn Any>)>;

    /// Optional cooperative execution deadline: normal consensus execution leaves it unset;
    /// untrusted sandbox calls set it so long-running bytecode aborts at instruction boundaries.
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
