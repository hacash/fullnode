#[cfg(feature = "execute")]
#[cfg(feature = "execute")]
use crate::iface::context::Context;

/// Consensus-defined wire and transaction limits. The generic chain runtime
/// carries this shape while each consensus profile owns its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MintParams {
    pub max_block_txs: usize,
    pub max_block_size: usize,
    pub max_tx_size: usize,
    pub difficulty_adjust_blocks: u64,
    pub difficulty_group_blocks: u64,
    pub each_block_target_time: u64,
}

/// Whether `size` exceeds the consensus cap; `max == 0` means unlimited (the rule
/// `MintParams.max_tx_size` uses on pending / submit / verify / API admission).
pub fn tx_exceeds_max_size(size: usize, max: usize) -> bool {
    max > 0 && size > max
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExecFrom {
    #[default]
    Top,
    Ast,
    Call,
}

impl ExecFrom {
    pub fn name(self) -> &'static str {
        match self {
            ExecFrom::Top => "TOP",
            ExecFrom::Ast => "AST",
            ExecFrom::Call => "CALL",
        }
    }
}

impl std::fmt::Display for ExecFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TopRule {
    None,
    Only,
    OnlyCanWithGuard,
    Unique,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActScope {
    pub top: Option<TopRule>,
    pub allow_ast: bool,
    pub allow_call: bool,
}

impl Default for ActScope {
    fn default() -> Self {
        Self::TOP
    }
}

impl ActScope {
    pub const TOP: Self = Self {
        top: Some(TopRule::None),
        allow_ast: false,
        allow_call: false,
    };
    pub const TOP_ONLY: Self = Self {
        top: Some(TopRule::Only),
        allow_ast: false,
        allow_call: false,
    };
    pub const TOP_ONLY_CAN_WITH_GUARD: Self = Self {
        top: Some(TopRule::OnlyCanWithGuard),
        allow_ast: false,
        allow_call: false,
    };
    pub const TOP_UNIQUE: Self = Self {
        top: Some(TopRule::Unique),
        allow_ast: false,
        allow_call: false,
    };
    pub const AST: Self = Self {
        // Dev's AST scope is TopAndAst: structural VM entry actions (ContractMainCall) may be a
        // top-level Type3 action or an AST child, but never a CALL action.
        top: Some(TopRule::None),
        allow_ast: true,
        allow_call: false,
    };
    pub const GUARD: Self = Self {
        top: Some(TopRule::None),
        allow_ast: true,
        allow_call: false,
    };
    /// Top-only guard companion; at most one action of this kind (e.g. ReqSignList).
    pub const TOP_GUARD_UNIQUE: Self = Self {
        top: Some(TopRule::Unique),
        allow_ast: false,
        allow_call: false,
    };
    pub const CALL: Self = Self {
        top: Some(TopRule::None),
        allow_ast: true,
        allow_call: true,
    };
    pub const CALL_ONLY: Self = Self {
        top: None,
        allow_ast: false,
        allow_call: true,
    };

    pub fn allows(&self, from: ExecFrom) -> bool {
        match from {
            ExecFrom::Top => self.top.is_some(),
            ExecFrom::Ast => self.allow_ast,
            ExecFrom::Call => self.allow_call,
        }
    }

    pub fn top_rule(&self) -> Option<TopRule> {
        self.top
    }
}

pub use field::AddrOrPtr;

/// Optional VM intent: `None` = no VM context, `Some(None)` = VM entry
/// without an intent, `Some(Some(id))` = bound to intent `id`.
pub type IntentScope = Option<Option<usize>>;

/// RAII guard setting `ExecFrom` on a context for a closure, restoring the previous value
/// on drop. Lives with the VM entry/sandbox (both `execute`-only).
#[cfg(feature = "execute")]
pub struct ExecFromGuard<'a> {
    ctx: &'a mut dyn Context,
    prev: ExecFrom,
}

#[cfg(feature = "execute")]
impl<'a> ExecFromGuard<'a> {
    pub fn enter(ctx: &'a mut dyn Context, from: ExecFrom) -> Self {
        let prev = ctx.exec_from();
        ctx.exec_from_set(from);
        Self { ctx, prev }
    }

    pub fn ctx(&mut self) -> &mut dyn Context {
        self.ctx
    }
}

#[cfg(feature = "execute")]
impl Drop for ExecFromGuard<'_> {
    fn drop(&mut self) {
        self.ctx.exec_from_set(self.prev);
    }
}

#[cfg(feature = "execute")]
pub fn with_exec_from<R>(
    ctx: &mut dyn Context,
    from: ExecFrom,
    f: impl FnOnce(&mut dyn Context) -> R,
) -> R {
    let mut guard = ExecFromGuard::enter(ctx, from);
    f(guard.ctx())
}
