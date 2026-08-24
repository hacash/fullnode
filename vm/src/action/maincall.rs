//! `ContractMainCall` (kind 44) top-level action. Runs arbitrary VM bytecode at tx
//! scope; codes are verified against the runtime `SpaceCap`/`GasExtra` before `VmRequest::Main`.

use field::{BytesW2, Fixed3, Uint1, Uint2};
use sys::Ret;

use crate::rt::{CodeConf, CodeType};

#[base::action(kind = 44, tx_min = 3, scope = AST, audit = "opaque", code, ctor = none,
    description = |this: &ContractMainCall| format!("Run main codes with conf {}", this.codeconf.uint()))]
#[derive(PartialEq, Eq)]
pub struct ContractMainCall {
    pub marks: Fixed3,
    pub codeconf: Uint1,
    pub codes: BytesW2,
}

impl ContractMainCall {
    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            marks: Fixed3::default(),
            codeconf: Uint1::from(0),
            codes: BytesW2::default(),
        }
    }

    pub fn from_bytecode(codes: Vec<u8>) -> Ret<Self> {
        let mut s = Self::new();
        s.codeconf = Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw());
        s.codes = BytesW2::from(codes)?;
        Ok(s)
    }
}

impl Default for ContractMainCall {
    fn default() -> Self {
        Self::new()
    }
}
