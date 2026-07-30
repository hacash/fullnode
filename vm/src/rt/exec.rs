use crate::value::ContractAddress;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryKind {
    #[default]
    Main,
    P2sh,
    Abst,
}

enum_try_from_u8_by_variant!(
    EntryKind,
    ItrErrCode::CallInvalid,
    "entry kind {} not found",
    [Main, P2sh, Abst]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffectMode {
    #[default]
    Edit,
    View,
    Pure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecCtx {
    pub entry: EntryKind,
    pub effect: EffectMode,
    pub call_depth: usize,
}

pub type BoundIntentId = Option<usize>;
pub type IntentScope = Option<BoundIntentId>;

#[derive(Debug, Clone)]
pub enum CallExit {
    Abort,
    Throw,
    Finish,
    Return,
    Call(CallSpec),
}

impl EntryKind {
    pub const fn root_exec(self) -> ExecCtx {
        match self {
            Self::Main => ExecCtx::main(),
            Self::P2sh => ExecCtx::p2sh(),
            Self::Abst => ExecCtx::abst(),
        }
    }

    pub fn call_base(self, gas: &GasExtra) -> i64 {
        match self {
            Self::Main => gas.main_call_base,
            Self::P2sh => gas.p2sh_call_base,
            Self::Abst => gas.abst_call_base,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameBindings {
    pub code_contract: Option<ContractAddress>,
    pub this_contract: Option<ContractAddress>,
    pub context_addr: Address,
    pub intent_scope: IntentScope,
    pub lib_table: Arc<[Address]>,
}

impl FrameBindings {
    pub fn root(context_addr: Address, lib_table: Arc<[Address]>) -> Self {
        Self {
            code_contract: None,
            this_contract: None,
            context_addr,
            intent_scope: None,
            lib_table,
        }
    }

    pub fn contract(
        this_contract: ContractAddress,
        code_contract: ContractAddress,
        lib_table: Arc<[Address]>,
    ) -> Self {
        Self {
            context_addr: this_contract.to_addr(),
            this_contract: Some(this_contract),
            code_contract: Some(code_contract),
            intent_scope: None,
            lib_table,
        }
    }

    pub fn with_intent_scope(mut self, intent_scope: IntentScope) -> Self {
        self.intent_scope = intent_scope;
        self
    }

    pub fn next_after_call(
        &self,
        switch_context: bool,
        anchor: ContractAddress,
        code_contract: ContractAddress,
        lib_table: Arc<[Address]>,
    ) -> Self {
        if switch_context {
            Self::contract(anchor, code_contract, lib_table).with_intent_scope(self.intent_scope)
        } else {
            Self {
                code_contract: Some(code_contract),
                this_contract: self.this_contract.clone(),
                context_addr: self.context_addr,
                intent_scope: self.intent_scope,
                lib_table,
            }
        }
    }
}

impl ExecCtx {
    pub const fn new(entry: EntryKind, effect: EffectMode, call_depth: usize) -> Self {
        Self {
            entry,
            effect,
            call_depth,
        }
    }

    pub const fn main() -> Self {
        Self::new(EntryKind::Main, EffectMode::Edit, 0)
    }

    pub const fn p2sh() -> Self {
        Self::new(EntryKind::P2sh, EffectMode::Edit, 0)
    }

    pub const fn abst() -> Self {
        Self::new(EntryKind::Abst, EffectMode::Edit, 1)
    }

    pub const fn contract(entry: EntryKind, effect: EffectMode) -> Self {
        Self::new(entry, effect, 1)
    }

    pub const fn external() -> Self {
        Self::contract(EntryKind::Main, EffectMode::Edit)
    }

    pub const fn view() -> Self {
        Self::contract(EntryKind::Main, EffectMode::View)
    }

    pub const fn pure() -> Self {
        Self::contract(EntryKind::Main, EffectMode::Pure)
    }

    pub const fn is_outer_entry(self) -> bool {
        self.call_depth == 0
    }

    pub fn ensure_call_depth(self, cap: &SpaceCap) -> VmrtErr {
        if self.call_depth >= cap.call_depth {
            return itr_err_code!(OutOfCallDepth);
        }
        Ok(())
    }

    pub fn enter_call(self, effect: EffectMode, cap: &SpaceCap) -> VmrtRes<Self> {
        let next = self
            .call_depth
            .checked_add(1)
            .ok_or_else(|| ItrErr::code(OutOfCallDepth))?;
        let next = Self::new(self.entry, effect, next);
        next.ensure_call_depth(cap)?;
        Ok(next)
    }
}
