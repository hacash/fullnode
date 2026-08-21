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
}
