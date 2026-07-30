//! `TextError(String)` + `ExecError{Revert,Fault}`
//!
//! recode.loc.md  #9
//! - `sys::Error`  `String` `ExecError{Revert, Fault}`
//! -  `"[REVERT] "`  hack
//! -  `Ret/Rerr/XRet/XRerr/TextRet`  `IntoExecRet/IntoTextRet/...`  trait
//!
//! **** `Error`  `kind`
//! - `Decode`  /
//! - `Revert`  /AST
//! - `Fault`
//!
//! `Ret<T>`

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// /
    Decode,
    /// revertAST
    Revert,
    Fault,
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
    pub fn decode(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Decode, msg)
    }
    pub fn revert(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Revert, msg)
    }
    pub fn fault(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Fault, msg)
    }

    pub fn is_decode(&self) -> bool {
        self.kind == ErrorKind::Decode
    }
    pub fn is_revert(&self) -> bool {
        self.kind == ErrorKind::Revert
    }
    pub fn is_fault(&self) -> bool {
        self.kind == ErrorKind::Fault
    }

    pub fn as_str(&self) -> &str {
        &self.msg
    }
    pub fn contains(&self, pat: &str) -> bool {
        self.msg.contains(pat)
    }

    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn code(&self) -> Option<&'static str> {
        self.code
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ErrorKind::Decode => write!(f, "[decode] {}", self.msg),
            ErrorKind::Revert => write!(f, "[revert] {}", self.msg),
            ErrorKind::Fault => write!(f, "{}", self.msg),
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

/// `Fault`  `return Err(..)` `errf!`
#[macro_export]
macro_rules! errf {
    ( $($v:expr),+ ) => { Err($crate::Error::fault(format!( $($v),+ ))) };
}

/// `Revert` `return Err(..)`
#[macro_export]
macro_rules! revertf {
    ( $($v:expr),+ ) => { Err($crate::Error::revert(format!( $($v),+ ))) };
}

/// `Decode` `return Err(..)`
#[macro_export]
macro_rules! decodef {
    ( $($v:expr),+ ) => { Err($crate::Error::decode(format!( $($v),+ ))) };
}
