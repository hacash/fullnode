/// `TX_ACTIONS_MAX` — per-transaction action count cap, enforced by the
/// protocol codecs (`protocol/src/codec/tx.rs`). Single source: the SDK
/// re-exports it instead of re-declaring the number.
pub const TX_ACTIONS_MAX: usize = 200;

/// `MAX_TX_SIZE` — consensus transaction size cap (bytes). Single source:
/// `mint::MINT_PARAMS.max_tx_size` and the SDK both read this constant, so a
/// limit change never drifts between the full node and the wallet surface.
pub const MAX_TX_SIZE: usize = 16 * 1024;

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
        // Dev's AST scope is TopAndAst: structural VM entry actions such as
        // ContractMainCall may be a top-level Type3 action or an AST child,
        // but never a CALL action.
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

/// VM intent `None`= VM`Some(None)`= VM  intent`Some(Some(id))`= intent
pub type IntentScope = Option<Option<usize>>;
