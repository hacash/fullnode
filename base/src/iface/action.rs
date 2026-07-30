use std::any::Any;
use std::sync::Arc;

use field::{Address, Amount, Encode};
use sys::Ret;

use crate::iface::context::Context;
use crate::runtime::{ActScope, AddrOrPtr};

pub type ActOut = (u32, Vec<u8>);
pub type ActionRef = Arc<dyn Action>;

#[derive(Clone, Debug)]
pub enum TransferPayload {
    Hac { amount: Vec<u8> },
    Sat { satoshi: u64 },
    Hacd { count: u32, names: Vec<u8> },
    Asset { serial: u64, amount: u64 },
}

pub trait TransferLike: Send + Sync {
    fn transfer_to(&self) -> Address;
    /// Wire-level destination, preserving address-table pointers. `None`
    /// means the transaction's main address is the implicit destination.
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(AddrOrPtr::Addr(self.transfer_to()))
    }
    fn transfer_amount(&self) -> &Amount;
    fn transfer_payload(&self) -> TransferPayload;
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        None
    }
}

///
///
///
#[derive(Clone)]
pub struct TransferRouting {
    pub action_kind: u16,
    pub from: Address,
    pub to: Address,
    pub payload: TransferPayload,
    pub authorize: bool,
    pub receive: bool,
}

///
pub fn resolve_transfer_routing(
    action: &dyn Action,
    ctx: &dyn Context,
) -> Ret<Option<TransferRouting>> {
    resolve_transfer_routing_on(action, ctx)
}

/// Same as `resolve_transfer_routing` but callable on a `Context`-bounded
/// type without forming a `&dyn Context` (needed for `?Sized` impls).
pub fn resolve_transfer_routing_on<C: Context + ?Sized>(
    action: &dyn Action,
    ctx: &C,
) -> Ret<Option<TransferRouting>> {
    let Some(t) = action.as_transfer_like() else {
        return Ok(None);
    };
    let to = match t.transfer_to_ptr() {
        Some(ptr) => ctx.addr(&ptr)?,
        None => ctx.env().tx.main,
    };
    let from = match t.transfer_from() {
        Some(ptr) => ctx.addr(&ptr)?,
        None => ctx.env().tx.main,
    };
    let authorize = from.is_scriptmh() || from.is_contract();
    let receive = to.is_contract();
    if !authorize && !receive {
        return Ok(None);
    }
    Ok(Some(TransferRouting {
        action_kind: action.kind(),
        from,
        to,
        payload: t.transfer_payload(),
        authorize,
        receive,
    }))
}

// ================================ Action ================================

/// Cross-crate action contract owned by `base` and consumed by protocol,
/// mint, VM, and dispatch code. Standard Hacash implementations live in
/// `protocol/src/codec/action`, `mint/src/action`, and `vm/src/action`.
/// Keep this trait chain-neutral: concrete consensus payloads stay with their
/// owning implementation crate.
pub trait Action: Encode + Send + Sync + std::fmt::Debug {
    fn kind(&self) -> u16;
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut>;

    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        None
    }
    fn required_flags(&self) -> u64 {
        0
    }
    fn scope(&self) -> ActScope;
    fn min_tx_type(&self) -> u8 {
        1
    }
    fn extra9(&self) -> bool {
        false
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![]
    }
    fn description(&self) -> String {
        String::new()
    }

    /// Escape hatch back to the concrete action type.
    ///
    /// **When to use the trait method instead**: capabilities shared by every
    /// chain's actions (signing requirements, transfer routing, scope, flags)
    /// MUST go through dedicated `Action` methods (`req_sign`, `scope`,
    /// `required_flags`, `as_transfer_like`, ...). Adding a new such method is
    /// the right fix when multiple callers `downcast_ref` to read the same
    /// generic field.
    ///
    /// **When downcast is correct**: chain-specific or consensus-mechanism
    /// business (e.g. Hacash diamond minting, inscription edits, PoW coinbase
    /// payloads). Such logic lives in the crate that owns those types (mint /
    /// protocol-internal / app composition root); base must not learn those
    /// concepts. `downcast_ref` returning `None` for an unrecognised chain is
    /// the intended fallback, not a bug.
    fn as_any(&self) -> &dyn Any;
}
