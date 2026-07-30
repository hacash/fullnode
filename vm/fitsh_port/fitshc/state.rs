use std::collections::HashSet;

use super::parse_deploy::DeployInfo;
use crate::IRNode;
use crate::Token::*;
use crate::contract::Contract;
use crate::rt::SourceMap;
use crate::rt::*;
use field::Address;
use sys::*;
use sys::{Ret, errf};

pub struct ParseState {
    pub tokens: Vec<Token>,
    pub idx: usize,
    pub max: usize,
    pub contract: Contract,
    pub contract_name: String,
    pub libs: Vec<(String, Address)>,
    pub deploy: Option<DeployInfo>,
    pub source_maps: Vec<(String, SourceMap)>,
    /// Top-level constants injected into each compiled body.
    pub consts: Vec<(String, Box<dyn IRNode>)>,
    pub warnings: Vec<String>,
    pub version: Option<FitshVersion>,
    pub userfunc_signs: HashSet<[u8; 4]>,
    pub abst_signs: HashSet<u8>,
    pub library_addrs: HashSet<Address>,
    pub inherit_addrs: HashSet<Address>,
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
        let max = tokens.len();
        Self {
            tokens,
            idx: 0,
            max,
            contract: Contract::new(),
            contract_name: String::new(),
            libs: Vec::new(),
            deploy: None,
            source_maps: Vec::new(),
            consts: Vec::new(),
            warnings: Vec::new(),
            version: None,
            userfunc_signs: HashSet::new(),
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

    pub fn eat_partition(&mut self, char: char) -> Ret<()> {
        if self.idx >= self.max {
            return errf!("expected '{}' but got EOF", char);
        }
        if let Partition(c) = self.tokens[self.idx] {
            if c == char {
                self.idx += 1;
                return Ok(());
            }
        }
        errf!("expected '{}' but got {:?}", char, self.tokens[self.idx])
    }
}
