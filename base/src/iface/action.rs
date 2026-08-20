use std::any::Any;
use std::sync::Arc;

use field::{Address, Amount, Encode};
use sys::Ret;

use crate::iface::context::Context;
use crate::runtime::{ActScope, AddrOrPtr};

pub type ActOut = (u32, Vec<u8>);
/// The wire/offline view of an action. In full builds the same object also
/// implements `ActionExecute`, and `ActionRef` carries that view; codec-only
/// (SDK/wasm) builds keep the wire view only — execution is not compiled in.
#[cfg(feature = "execute")]
pub type ActionRef = Arc<dyn ActionExecute>;
#[cfg(not(feature = "execute"))]
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

/// Nested control-flow children of an action (AST select/if, and any future
/// branching kind). `depth_inc` is the protocol AST-depth cost of this node
/// (`AstSelect` = 1, `AstIf` = 2); `branches` preserves the review-path
/// grouping (one inner vec per branch). Leaf actions return `None` from
/// `Action::nested_actions`.
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

/// Cross-crate action contract owned by `base` and consumed by protocol,
/// mint, VM, and dispatch code. Standard Hacash implementations live in
/// `protocol/src/codec/action`, `mint/src/action`, and `vm/src/action`.
/// `ToJSON` is the canonical API-facing representation; it does not
/// participate in binary encoding, validation, or consensus execution.
/// Regular payloads derive `base::ActionCodec`; irregular payloads keep an
/// explicit codec in their owning crate. Neither path generates execution.
///
/// This is the wire + offline-semantics view (kind, scope, signers, transfer
/// routing, flags). Execution lives on the separate `ActionExecute` trait,
/// implemented only in full builds; codec-only (SDK/wasm) builds compile
/// without it, so no stub exists.
///
/// `ToJSON` is deliberately NOT a supertrait: the SDK's wasm core is JSON-free
/// (the transport is binary; JSON is a JS-side concern), and a `ToJSON`
/// supertrait would force every `dyn Action` vtable in the SDK to carry the
/// JSON formatter machinery. Code that needs the JSON view (fullnode API,
/// registry JSON decoders) bounds on `ActionJsonCodec` / `field::ToJSON`
/// explicitly.
pub trait Action: Encode + Send + Sync + std::fmt::Debug {
    fn kind(&self) -> u16;

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

    /// Nested control-flow children. Default `None` (leaf). Protocol topology
    /// analysis, AST signer collection and the SDK review tree all walk this
    /// instead of downcasting concrete AST types, so a new branching action
    /// is complete once it implements this method.
    fn nested_actions(&self) -> Option<NestedActions<'_>> {
        None
    }

    /// Escape hatch back to the concrete action type.
    ///
    /// **When to use the trait method instead**: capabilities shared by every
    /// chain's actions (signing requirements, transfer routing, scope, flags,
    /// nested children) MUST go through dedicated `Action` methods (`req_sign`,
    /// `scope`, `required_flags`, `as_transfer_like`, `nested_actions`, ...).
    /// Adding a new such method is the right fix when multiple callers
    /// `downcast_ref` to read the same generic field.
    ///
    /// **When downcast is correct**: chain-specific or consensus-mechanism
    /// business (e.g. Hacash diamond minting, inscription edits, PoW coinbase
    /// payloads). Such logic lives in the crate that owns those types (mint /
    /// protocol-internal / app composition root); base must not learn those
    /// concepts. `downcast_ref` returning `None` for an unrecognised chain is
    /// the intended fallback, not a bug.
    fn as_any(&self) -> &dyn Any;
}

/// Execution view of an action: `Action` plus the consensus `execute` body.
/// Implemented (via `impl_action!`) only when the `execute` feature is on, so
/// the SDK/wasm dependency graph has no callable execution surface at all —
/// there is no stub to reach.
///
/// `ActionExecute` additionally carries the JSON view (`ActionJsonView`), so
/// full builds that hold `dyn ActionExecute` (the execute `ActionRef`) can
/// still render action JSON in API services — while the SDK's `dyn Action`
/// vtables stay JSON-free.
pub trait ActionExecute: Action + ActionJsonView {
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut>;
}

/// JSON rendering for actions, kept off the `Action` wire trait so SDK/wasm
/// `dyn Action` vtables carry no JSON formatter machinery. Full-node code that
/// renders action JSON (API services) holds the execute view or bounds on this
/// trait explicitly.
pub trait ActionJsonView: field::ToJSON {}

impl<T: field::ToJSON> ActionJsonView for T {}

/// Owned JSON construction for actions whose JSON schema maps directly to
/// their fields. This is deliberately separate from `Action`: internal or
/// dynamic actions are not required to expose a generic JSON constructor.
pub trait ActionJsonCodec: Action + Sized {
    fn decode_json(json: &str) -> Ret<Self>;
}

/// Generate the mechanical part of a regular `Action` implementation.
///
/// `name` supplies the static protocol/API identifier exposed as the
/// inherent `NAME` constant (used e.g. by the setup host-definition
/// registration in `protocol::setup`). It is a type-level constant and does
/// not enter `dyn Action`, so object safety and `ActionRef` are unaffected.
///
/// The macro emits two impls: the wire/offline `Action` surface, and the
/// `ActionExecute` impl with the real execution body. The execute impl is
/// gated as a whole block on the expanding crate's `execute` feature (which
/// forwards to `base/execute`), so codec-only (SDK/wasm) builds compile no
/// execution body and no stub at all. This macro only removes repeated
/// metadata, `as_any`, and size-based gas plumbing; it does not add a second
/// precheck or hook path.
#[macro_export]
macro_rules! impl_action {
    ($class:ty {
        name: $name:literal,
        scope: $scope:expr,
        min_tx_type: $min_tx_type:expr,
        description: $description:expr,
        execute: ($action_self:ident, $action_ctx:ident) $execute:block $(,)?
    }) => {
        $crate::impl_action! {
            $class {
                name: $name,
                scope: $scope,
                min_tx_type: $min_tx_type,
                extra9: |_: &$class| false,
                req_sign: |_: &$class| vec![],
                as_transfer_like: none,
                description: $description,
                execute: ($action_self, $action_ctx) $execute
            }
        }
    };

    ($class:ty {
        name: $name:literal,
        scope: $scope:expr,
        min_tx_type: $min_tx_type:expr,
        execute: ($action_self:ident, $action_ctx:ident) $execute:block $(,)?
    }) => {
        $crate::impl_action! {
            $class {
                name: $name,
                scope: $scope,
                min_tx_type: $min_tx_type,
                extra9: |_: &$class| false,
                req_sign: |_: &$class| vec![],
                as_transfer_like: none,
                description: |_: &$class| String::new(),
                execute: ($action_self, $action_ctx) $execute
            }
        }
    };

    ($class:ty {
        name: $name:literal,
        scope: $scope:expr,
        min_tx_type: $min_tx_type:expr,
        extra9: $extra9:expr,
        req_sign: $req_sign:expr,
        as_transfer_like: $as_transfer_like:ident,
        description: $description:expr,
        execute: ($action_self:ident, $action_ctx:ident) $execute:block $(,)?
    }) => {
        impl $class {
            pub const NAME: &'static str = $name;
        }

        impl $crate::Action for $class {
            fn kind(&self) -> u16 {
                Self::KIND
            }

            fn scope(&self) -> $crate::ActScope {
                $scope
            }

            fn min_tx_type(&self) -> u8 {
                $min_tx_type
            }

            fn extra9(&self) -> bool {
                ($extra9)(self)
            }

            fn req_sign(&self) -> Vec<$crate::AddrOrPtr> {
                ($req_sign)(self)
            }

            fn as_transfer_like(&self) -> Option<&dyn $crate::TransferLike> {
                $crate::impl_action!(@as_transfer_like self, $as_transfer_like)
            }

            fn description(&self) -> String {
                ($description)(self)
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        #[cfg(feature = "execute")]
        impl $crate::ActionExecute for $class {
            fn execute(&$action_self, $action_ctx: &mut dyn $crate::Context) -> sys::Ret<$crate::ActOut> {
                let gas = $action_self.size() as u32;
                let result: sys::Ret<Vec<u8>> = (|| $execute)();
                Ok((gas, result?))
            }
        }
    };

    (@as_transfer_like $action_self:ident, self) => {
        Some($action_self)
    };

    (@as_transfer_like $action_self:ident, none) => {
        None
    };
}

/// Registry-compatible JSON creator for regular derived actions.
///
/// Full builds store the execute view (`ActionRef = Arc<dyn ActionExecute>`),
/// so the creator needs the concrete type to carry the execution impl;
/// codec-only builds store the wire view and only need `Action`. The two
/// bodies are identical — only the bound differs.
#[cfg(feature = "execute")]
pub fn decode_regular_action_json<T>(
    _reg: &dyn crate::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef>
where
    T: ActionJsonCodec + ActionExecute + 'static,
{
    let action = T::decode_json(json)?;
    if action.kind() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            action.kind()
        );
    }
    Ok(Arc::new(action))
}

/// Registry-compatible JSON creator for regular derived actions (codec-only
/// builds; see the `execute` variant above).
#[cfg(not(feature = "execute"))]
pub fn decode_regular_action_json<T>(
    _reg: &dyn crate::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef>
where
    T: ActionJsonCodec + 'static,
{
    let action = T::decode_json(json)?;
    if action.kind() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            action.kind()
        );
    }
    Ok(Arc::new(action))
}

/// Register binary and JSON codecs from one regular-action type list.
///
/// Each group carries its SDK-facing friendly kind name (`""` when the group
/// has no friendly form, e.g. VM host actions); it is forwarded to
/// `register_action_family` from the same invocation as the kind list, so the
/// friendly grouping can never drift from the registered kinds.
#[macro_export]
macro_rules! register_regular_actions {
    ($registry:expr, $( $friendly:literal, $binary:path => [$( $action:ty ),+ $(,)?] ),+ $(,)?) => {{
        $(
            let kinds: &[u16] = &[$(<$action>::KIND),+];
            $registry.register_action(kinds, $binary)?;
            if !$friendly.is_empty() {
                $registry.register_action_family($friendly, kinds)?;
            }
            $(
                $registry.register_action_json(
                    &[<$action>::KIND],
                    $crate::decode_regular_action_json::<$action>,
                )?;
                $registry.register_action_schema(
                    <$action as $crate::ActionSchemaProvider>::ACTION_SCHEMA,
                )?;
            )+
        )+
        Ok::<(), sys::Error>(())
    }};
}

/// Register a custom binary/JSON action family from one authoritative kind
/// list. The registry interface stays unchanged while callers can no longer
/// accidentally use different kind lists for the two codec directions.
/// `$friendly` is the SDK-facing friendly kind name (`""` when the family has
/// no friendly form).
#[macro_export]
macro_rules! register_custom_actions {
    ($registry:expr, $friendly:literal, $binary:path, $json:path => [$( $action:ty ),+ $(,)?] $(,)?) => {{
        let kinds: &[u16] = &[$(<$action>::KIND),+];
        $registry.register_action(kinds, $binary)?;
        $registry.register_action_json(kinds, $json)?;
        if !$friendly.is_empty() {
            $registry.register_action_family($friendly, kinds)?;
        }
        $(
            $registry.register_action_schema(
                <$action as $crate::ActionSchemaProvider>::ACTION_SCHEMA,
            )?;
        )+
        Ok::<(), sys::Error>(())
    }};
}

/// Keep the JSON-only form for irregular actions whose wire codec is custom.
///
/// Builds the object with direct string pushes (no `format!`) so the generated
/// `to_json_fmt` carries no fmt machinery (wasm size).
#[macro_export]
macro_rules! impl_action_to_json {
    ($class:ty { $($field:ident),* $(,)? }) => {
        impl field::ToJSON for $class {
            fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
                let mut s = String::new();
                s.push_str("{\"kind\":");
                s.push_str(&field::ToJSON::to_json_fmt(&self.kind, fmt));
                $(
                    s.push(',');
                    s.push('"');
                    s.push_str(stringify!($field));
                    s.push_str("\":");
                    s.push_str(&field::ToJSON::to_json_fmt(&self.$field, fmt));
                )*
                s.push('}');
                s
            }
        }
    };
}

#[macro_export]
macro_rules! impl_fields_to_json {
    ($class:ty { $($field:ident),* $(,)? } optional $optional:ident when $condition:ident) => {
        field::impl_struct_json!($class { $($field),* } optional $optional when $condition);
    };
    ($class:ty { $($field:ident),* $(,)? }) => {
        field::impl_struct_json!($class { $($field),* });
    };
}
