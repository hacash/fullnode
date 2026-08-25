//! Generic error model: `Error { kind, code, msg }` — `kind` only describes
//! generic handling, `code` is an optional stable string owned by the creating layer. Lifecycle changes occur only at the engine boundary.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Ordinary recoverable error, including invalid input and decode errors.
    Normal,
    /// revertAST
    Revert,
    Fault,
    /// The state machine cannot safely continue.
    Abort,
}

impl ErrorKind {
    /// Handling rank. Combinations may only move toward a stricter kind.
    pub fn rank(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Revert => 1,
            Self::Fault => 2,
            Self::Abort => 3,
        }
    }

    /// Merge two kinds. A `Revert` mixed with any other kind cannot stay
    /// `Revert` (AST would capture a path that also had a harder failure);
    /// the floor is `Fault` unless the other kind is already `Abort`.
    pub fn merge_upgrade(self, other: Self) -> Self {
        let max = if self.rank() >= other.rank() {
            self
        } else {
            other
        };
        let mixed_revert = (self == Self::Revert) != (other == Self::Revert);
        if mixed_revert && max.rank() < Self::Fault.rank() {
            Self::Fault
        } else {
            max
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub msg: String,
    code: Option<&'static str>,
}

pub type Ret<T> = Result<T, Error>;
/// `Rerr`
pub type Rerr = Result<(), Error>;

impl Error {
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> Self {
        Self {
            kind,
            msg: msg.into(),
            code: None,
        }
    }
    pub fn normal(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Normal, msg)
    }
    pub fn revert(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Revert, msg)
    }
    pub fn fault(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Fault, msg)
    }
    pub fn abort(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Abort, msg)
    }

    pub fn is_normal(&self) -> bool {
        self.kind == ErrorKind::Normal
    }
    pub fn is_revert(&self) -> bool {
        self.kind == ErrorKind::Revert
    }
    pub fn is_fault(&self) -> bool {
        self.kind == ErrorKind::Fault
    }
    pub fn is_abort(&self) -> bool {
        self.kind == ErrorKind::Abort
    }

    pub fn as_str(&self) -> &str {
        &self.msg
    }
    pub fn contains(&self, pat: &str) -> bool {
        self.msg.contains(pat)
    }

    /// Attach a stable string owned by the caller's layer. `sys` does not
    /// interpret or enumerate these values.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn code(&self) -> Option<&'static str> {
        self.code
    }

    /// Attach operational context: prepend a message prefix, preserving `kind`/`code`
    /// and classification. Use to merge a secondary error's message into the primary.
    pub fn context(mut self, msg: impl Into<String>) -> Self {
        let prefix = msg.into();
        if prefix.is_empty() {
            return self;
        }
        self.msg = format!("{}: {}", prefix, self.msg);
        self
    }

    fn append_secondary_message(primary: String, secondary: String) -> String {
        if secondary.is_empty() || primary.contains(&secondary) {
            primary
        } else {
            format!("{} | secondary: {}", primary, secondary)
        }
    }

    /// Combine two execution errors. Kind only upgrades: if either is `Fault`,
    /// the result is at least `Fault`; `Abort` wins over `Fault`; a `Revert`
    /// mixed with a non-`Revert` becomes `Fault` (or `Abort` if the other is
    /// `Abort`). When exec is revert and the other is not, the other is the
    /// primary message — matching live `merge_xret_failure`.
    pub fn merge_upgrade(self, other: Self) -> Self {
        let kind = self.kind.merge_upgrade(other.kind);
        let self_is_primary = if self.kind == kind {
            true
        } else if other.kind == kind {
            false
        } else {
            // Floor (typically Revert + Normal → Fault): live uses the
            // non-revert side as primary when exec was revert.
            !self.is_revert()
        };
        let (mut primary, secondary) = if self_is_primary {
            (self, other)
        } else {
            (other, self)
        };
        let secondary_text = secondary.to_string();
        let secondary_code = secondary.code();
        let primary_was_abort = primary.is_abort();
        primary.msg = Self::append_secondary_message(primary.msg, secondary_text);
        primary.kind = kind;
        if kind == ErrorKind::Abort && !primary_was_abort {
            if let Some(code) = secondary_code {
                primary = primary.with_code(code);
            }
        }
        primary
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ErrorKind::Normal => write!(f, "[normal] {}", self.msg),
            ErrorKind::Revert => write!(f, "[revert] {}", self.msg),
            ErrorKind::Fault => write!(f, "{}", self.msg),
            ErrorKind::Abort => write!(f, "[abort] {}", self.msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<&str> for Error {
    fn from(v: &str) -> Self {
        Self::fault(v)
    }
}
impl From<String> for Error {
    fn from(v: String) -> Self {
        Self::fault(v)
    }
}

/// Fault: `return Err(..)` via `errf!`.
#[macro_export]
macro_rules! errf {
    ( $($v:expr),+ ) => { Err($crate::Error::fault(format!( $($v),+ ))) };
}

/// `Revert` `return Err(..)`
#[macro_export]
macro_rules! revertf {
    ( $($v:expr),+ ) => { Err($crate::Error::revert(format!( $($v),+ ))) };
}

/// Codec helper returning an `ErrorKind::Normal` error.
#[macro_export]
macro_rules! normalf {
    ( $($v:expr),+ ) => { Err($crate::Error::normal(format!( $($v),+ ))) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_helper_is_the_normal_category() {
        let error = Error::normal("bad input");
        assert_eq!(error.kind, ErrorKind::Normal);
        assert!(error.is_normal());
    }

    #[test]
    fn caller_owned_static_code_is_preserved() {
        const CODE: &'static str = "example_code";
        let error = Error::normal("example").with_code(CODE);
        assert_eq!(error.code(), Some(CODE));
    }

    #[test]
    fn merge_upgrade_kind_never_downgrades() {
        use ErrorKind::*;
        assert_eq!(Revert.merge_upgrade(Fault), Fault);
        assert_eq!(Fault.merge_upgrade(Revert), Fault);
        assert_eq!(Revert.merge_upgrade(Revert), Revert);
        assert_eq!(Fault.merge_upgrade(Fault), Fault);
        assert_eq!(Abort.merge_upgrade(Fault), Abort);
        assert_eq!(Fault.merge_upgrade(Abort), Abort);
        assert_eq!(Revert.merge_upgrade(Abort), Abort);
        assert_eq!(Abort.merge_upgrade(Revert), Abort);
        assert_eq!(Revert.merge_upgrade(Normal), Fault);
        assert_eq!(Normal.merge_upgrade(Revert), Fault);
        assert_eq!(Normal.merge_upgrade(Fault), Fault);
        assert_eq!(Normal.merge_upgrade(Normal), Normal);
    }

    #[test]
    fn merge_upgrade_revert_plus_fault_is_fault_with_settle_primary() {
        let exec = Error::revert("biz fail");
        let settle = Error::fault("Main gas cost invalid: 0");
        let merged = exec.merge_upgrade(settle);
        assert!(merged.is_fault(), "{merged}");
        assert!(!merged.is_revert(), "{merged}");
        assert!(merged.contains("gas cost invalid"), "{merged}");
        assert!(merged.contains("biz fail"), "{merged}");
    }

    #[test]
    fn merge_upgrade_fault_plus_revert_stays_fault_with_exec_primary() {
        let exec = Error::fault("ThrowAbort(151): boom");
        let settle = Error::revert("should not win");
        let merged = exec.merge_upgrade(settle);
        assert!(merged.is_fault(), "{merged}");
        assert!(merged.as_str().starts_with("ThrowAbort"), "{merged}");
        assert!(merged.contains("should not win"), "{merged}");
    }

    #[test]
    fn merge_upgrade_abort_is_not_downgraded_to_fault() {
        const CODE: &'static str = "storage_read_failed";
        let exec = Error::revert("biz fail");
        let settle = Error::abort("backend down").with_code(CODE);
        let merged = exec.merge_upgrade(settle);
        assert!(merged.is_abort(), "{merged}");
        assert_eq!(merged.code(), Some(CODE));
        assert!(merged.contains("backend down"), "{merged}");
    }
}
