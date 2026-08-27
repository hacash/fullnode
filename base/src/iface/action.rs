use std::any::Any;
use std::sync::Arc;

use field::{Address, Amount, Decode, Encode};
use sys::Ret;

#[cfg(feature = "execute")]
use crate::iface::context::Context;
use crate::runtime::{ActScope, AddrOrPtr};

pub type ActOut = (u32, Vec<u8>);
/// Wire/offline view of an action — no signer/topology or execution/JSON semantics;
/// execution is the separate `ActionExecute` trait via `as_execute` (type-stable across `execute`).
pub trait ActionCodec: Encode + Send + Sync + std::fmt::Debug {
    fn kind(&self) -> u16;
    fn schema(&self) -> Option<&'static crate::ActionSchema> {
        None
    }
    fn as_any(&self) -> &dyn Any;
}

/// Canonical SDK-facing name for an action. The `ActionCodec` derive generates
/// this from the Rust type name (snake_case); an explicit `name` overrides the
/// automatic conversion — `#[base::action(...)]` / `base::action_simple!`
/// forward it into `#[action_codec(name = "...")]`, keeping this trait and the
/// type's inherent `NAME` const in lockstep.
pub trait ActionName {
    const NAME: &'static str;
}

/// Offline review view. `Action` is retained as the public compatibility name.
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

#[derive(Clone)]
pub struct TransferRouting {
    pub action_kind: u16,
    pub from: Address,
    pub to: Address,
    pub payload: TransferPayload,
    pub authorize: bool,
    pub receive: bool,
}

#[cfg(feature = "execute")]
pub fn resolve_transfer_routing(
    action: &dyn Action,
    ctx: &dyn Context,
) -> Ret<Option<TransferRouting>> {
    resolve_transfer_routing_on(action, ctx)
}

/// Same as `resolve_transfer_routing` but callable on a `Context`-bounded
/// type without forming a `&dyn Context` (needed for `?Sized` impls).
#[cfg(feature = "execute")]
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

/// Nested control-flow children of an action. `depth_inc` is the protocol AST-depth
/// cost (`AstSelect` = 1, `AstIf` = 2); `branches` preserves review-path grouping.
#[derive(Clone)]
pub struct NestedActions<'a> {
    pub depth_inc: usize,
    pub branches: Vec<Vec<&'a dyn Action>>,
}

impl<'a> NestedActions<'a> {
    pub fn flatten(&self) -> Vec<&'a dyn Action> {
        self.branches.iter().flatten().copied().collect()
    }
}

/// Cross-crate offline action-view contract owned by `base` (impls in protocol/mint/vm).
/// Execution is a separate `ActionExecute` trait. JSON encoding is `field::ToJSON`
/// on the action type itself (`Action: ActionCodec + ToJSON`); the registry only
/// decodes.
pub trait Action: ActionCodec + field::ToJSON {
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

    /// Nested control-flow children; default `None` (leaf). Topology analysis, AST signer
    /// collection and the SDK review tree walk this instead of downcasting.
    fn nested_actions(&self) -> Option<NestedActions<'_>> {
        None
    }

    /// Escape hatch to the concrete action type; downcast is for chain-specific consensus,
    /// `None` being the intended fallback. In `execute` builds this also upcasts to the execute view.
    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn ActionExecute> {
        None
    }
}

/// Execution view of an action: `Action` plus the consensus `execute` body, only when
/// `execute` is on — the SDK/wasm graph has no execution surface. JSON encoding is
/// `ToJSON` on `Action` and does not go through execute.
#[cfg(feature = "execute")]
pub trait ActionExecute: Action {
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut>;
}

#[cfg(feature = "execute")]
impl dyn Action {
    /// Consensus execution. Looks up the execute view instead of requiring
    /// `ActionRef` to be a different type.
    pub fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        match self.as_execute() {
            Some(exec) => ActionExecute::execute(exec, ctx),
            None => sys::errf!("action kind {} has no execute surface", self.kind()),
        }
    }
}

/// Static consensus placement scope of an action type, alongside its wire schema.
/// `#[base::action(...)]` / `base::action_simple!` generate this from the same
/// `scope` fact; handwritten `Action` impls (e.g. AST control-flow actions) must
/// provide it too, because `action_codec_binding!` embeds it in every binding so
/// static selection (the SDK's CALL_ONLY exclusion) never has to guess it from
/// kind arithmetic.
pub trait ActionScopeProvider {
    const SCOPE: ActScope;
}

/// Hand-written `ActionCodec` shell for `#[base::action(wire = manual)]` types
/// (derive skips `ActionCodec` in that mode). `kind` / `schema` / `as_any` only;
/// wire and JSON stay on the type.
#[macro_export]
macro_rules! impl_action_codec {
    ($ty:ty) => {
        impl $crate::ActionCodec for $ty {
            fn kind(&self) -> u16 {
                Self::KIND
            }

            fn schema(&self) -> Option<&'static $crate::ActionSchema> {
                Some(&<Self as $crate::ActionSchemaProvider>::ACTION_SCHEMA)
            }

            fn as_any(&self) -> &dyn ::std::any::Any {
                self
            }
        }
    };
}

/// Consensus `ActionExecute` body for an action declared by `#[base::action(...)]` /
/// `base::action_simple!`; gated on `execute`, applies size-based gas, returns
/// `Ret<Vec<u8>>`. Dispatch stays `Action::as_execute`.
#[macro_export]
macro_rules! impl_action_execute {
    ($class:ty {
        ($action_self:ident, $action_ctx:ident) $execute:block $(,)?
    }) => {
        #[cfg(feature = "execute")]
        impl $crate::ActionExecute for $class {
            fn execute(
                &$action_self,
                $action_ctx: &mut dyn $crate::Context,
            ) -> sys::Ret<$crate::ActOut> {
                let gas = field::Encode::size($action_self) as u32;
                let result: sys::Ret<Vec<u8>> = (|| $execute)();
                Ok((gas, result?))
            }
        }
    };
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct ExecuteOnlyAction;

    impl Encode for ExecuteOnlyAction {
        fn size(&self) -> usize {
            2
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            field::Uint2::from(65_000).encode_to(out);
        }
    }

    impl ActionCodec for ExecuteOnlyAction {
        fn kind(&self) -> u16 {
            65_000
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl field::ToJSON for ExecuteOnlyAction {
        fn to_json_fmt(&self, _fmt: &field::JSONFormater) -> String {
            "{\"kind\":65000}".to_owned()
        }
    }

    impl Action for ExecuteOnlyAction {
        fn scope(&self) -> ActScope {
            ActScope::TOP
        }

        fn as_execute(&self) -> Option<&dyn ActionExecute> {
            Some(self)
        }
    }

    impl ActionExecute for ExecuteOnlyAction {
        fn execute(&self, _ctx: &mut dyn Context) -> Ret<ActOut> {
            Ok((0, vec![]))
        }
    }

    #[test]
    fn execute_stays_independent_of_json() {
        let action = ExecuteOnlyAction;
        assert_eq!(action.kind(), 65_000);
        assert!(action.as_execute().is_some());
        assert_eq!(field::ToJSON::to_json(&action), "{\"kind\":65000}");

        let codec: &dyn ActionCodec = &action;
        assert_eq!(codec.kind(), 65_000);
        assert!(codec.schema().is_none());
    }
}

/// Wrap a decoded action as `ActionRef`. `ActionRef` is always the wire view,
/// so a single bound (`Action + Decode`) is enough in every build.
pub fn decode_regular_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}

/// Generic wire creator for regular derived actions. The registry peeks `kind`
/// to dispatch, then hands the **full** buffer to this creator; derived `Decode`
/// re-reads and validates the prefix. Hand-written wire decoders (AST, diamond
/// mint) register a custom creator instead.
pub fn create_regular_action<T>(
    _reg: &dyn crate::BinaryCodecs,
    buf: &[u8],
) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    decode_regular_action::<T>(buf)
}

/// Registry-compatible JSON creator for regular derived actions.
/// `kind` is read from the JSON object (required); the registry has already
/// dispatched to this type's decoder.
pub fn decode_regular_action_json<T>(_reg: &dyn crate::CodecRegistry, json: &str) -> Ret<ActionRef>
where
    T: Action + Default + field::FromJSON + crate::ActionSchemaProvider + 'static,
{
    let mut value = T::default();
    value.from_json(json)?;
    if value.kind() != T::ACTION_SCHEMA.kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            T::ACTION_SCHEMA.kind,
            value.kind()
        );
    }
    Ok(Arc::new(value))
}
