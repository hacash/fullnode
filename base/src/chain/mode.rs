/// Execution validation mode for block application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyMode {
    Strict,
    FastSync,
}

impl ApplyMode {
    pub const fn is_fast_sync(self) -> bool {
        matches!(self, Self::FastSync)
    }

    pub const fn is_strict(self) -> bool {
        !self.is_fast_sync()
    }
}
