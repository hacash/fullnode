use std::collections::{HashMap, HashSet};

use super::parse_deploy::DeployInfo;
use crate::IRNode;
use crate::Token::*;
use crate::contract::Contract;
use crate::rt::SourceMap;
use crate::rt::*;
use field::Address;
use sys::Ret;

pub struct ParseState {
    pub tokens: Vec<Token>,
    pub idx: usize,
    pub max: usize,
    /// Parallel to `tokens` (P2-D03 T1 sidecar). Empty (or per-entry
    /// `TokenSpan::NONE`) when built through the spanless path.
    pub spans: Vec<TokenSpan>,
    /// Source file name for `{file}:{line}:{col}` error prefixes; `None` in
    /// the library path (errors then carry `{line}:{col}: ` only).
    pub file: Option<String>,
    pub contract: Contract,
    pub contract_name: String,
    pub libs: Vec<(String, Address)>,
    pub deploy: Option<DeployInfo>,
    pub source_maps: Vec<(String, SourceMap)>,
    /// Top-level constants injected into each compiled body.
    pub consts: Vec<(String, Box<dyn IRNode>)>,
    pub macros: Vec<MacroDef>,
    pub warnings: Vec<String>,
    pub version: Option<FitshVersion>,
    pub userfunc_signs: HashSet<[u8; 4]>,
    /// Container or ELF bytes supplied by `compile_with_rv64im`.
    pub external_rv64im: HashMap<String, Vec<u8>>,
    pub abst_signs: HashSet<u8>,
    pub library_addrs: HashSet<Address>,
    pub inherit_addrs: HashSet<Address>,
}

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Token>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FitshVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl FitshVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for FitshVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub const FITSH_CURRENT_VERSION: FitshVersion = FitshVersion::new(1, 0, 0);

impl ParseState {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self::with_spans(tokens, Vec::new(), None)
    }

    /// P2-D03 T1: build with the tokenizer's parallel span table and an
    /// optional source-file name for error prefixes. Tokens and their
    /// behavior are identical to [`ParseState::new`].
    pub fn with_spans(
        tokens: Vec<Token>,
        spans: Vec<TokenSpan>,
        file: Option<String>,
    ) -> Self {
        let max = tokens.len();
        Self {
            tokens,
            idx: 0,
            max,
            spans,
            file,
            contract: Contract::new(),
            contract_name: String::new(),
            libs: Vec::new(),
            deploy: None,
            source_maps: Vec::new(),
            consts: Vec::new(),
            macros: Vec::new(),
            warnings: Vec::new(),
            version: None,
            userfunc_signs: HashSet::new(),
            external_rv64im: HashMap::new(),
            abst_signs: HashSet::new(),
            library_addrs: HashSet::new(),
            inherit_addrs: HashSet::new(),
        }
    }

    pub fn current(&self) -> Option<&Token> {
        if self.idx >= self.max {
            None
        } else {
            Some(&self.tokens[self.idx])
        }
    }

    pub fn advance(&mut self) {
        self.idx += 1;
    }

    pub fn skip_soft_separators(&mut self) {
        while matches!(self.current(), Some(Partition(','))) {
            self.advance();
        }
    }

    // ---------- P2-D03 T1: position-aware error construction ----------

    /// Span of token `idx` from the sidecar table; `None` when the table is
    /// absent, the index is out of range, or the token is synthesized
    /// (line 0). Falls back to the last known span so EOF errors still point
    /// at the end of the last real token.
    pub fn span_at(&self, idx: usize) -> Option<TokenSpan> {
        if idx < self.spans.len() && self.spans[idx].known() {
            return Some(self.spans[idx]);
        }
        self.spans.iter().rev().find(|s| s.known()).copied()
    }

    /// `{file}:{line}:{col}: ` (or `{line}:{col}: ` without a file, or empty
    /// when no span is known) prefix for error messages.
    pub fn pos_prefix(&self, idx: usize) -> String {
        match self.span_at(idx) {
            Some(span) => match &self.file {
                Some(file) => format!("{file}:{span}: "),
                None => format!("{span}: "),
            },
            None => String::new(),
        }
    }

    /// Build a position-prefixed fault error (the `errf_at!` helper).
    pub fn errf_at(&self, idx: usize, msg: impl std::fmt::Display) -> sys::Error {
        sys::Error::fault(format!("{}{}", self.pos_prefix(idx), msg))
    }

    /// Position prefix at the current lookahead token.
    pub fn pos_here(&self) -> String {
        self.pos_prefix(self.idx)
    }

    /// Error anchored at the current lookahead token.
    pub fn err_here(&self, msg: impl std::fmt::Display) -> sys::Error {
        self.errf_at(self.idx, msg)
    }

    pub fn eat_partition(&mut self, char: char) -> Ret<()> {
        if self.idx >= self.max {
            return Err(self.err_here(format!("expected '{char}' but got EOF")));
        }
        if let Partition(c) = self.tokens[self.idx] {
            if c == char {
                self.idx += 1;
                return Ok(());
            }
        }
        Err(self.err_here(format!(
            "expected '{}' but got {}",
            char,
            crate::lang::token_desc_opt(self.tokens.get(self.idx))
        )))
    }
}

/// Count `'{'` vs `'}'` partition tokens over a token stream (P2-D03 T7:
/// brace-balance diagnostics for EOF-class errors).
pub fn brace_balance(tokens: &[Token]) -> (usize, usize) {
    let mut open = 0usize;
    let mut close = 0usize;
    for t in tokens {
        match t {
            Partition('{') => open += 1,
            Partition('}') => close += 1,
            _ => {}
        }
    }
    (open, close)
}
