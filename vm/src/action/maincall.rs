//! `ContractMainCall` (kind 44) top-level action. Runs arbitrary VM bytecode at tx
//! scope; codes are verified against the runtime `SpaceCap`/`GasExtra` before `VmRequest::Main`.

use std::sync::Arc;

use base::{ActScope, ActionRef};
use field::{BytesW2, Decode, Fixed3, Uint1, Uint2};
use sys::Ret;

use crate::rt::{CodeConf, CodeType};

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
#[action_codec(audit = "opaque")]
pub struct ContractMainCall {
    pub kind: Uint2,
    pub marks: Fixed3,
    pub codeconf: Uint1,
    pub codes: BytesW2,
}

impl ContractMainCall {
    pub const KIND: u16 = 44;

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

base::impl_action_facts! {
    ContractMainCall {
        name: "contract_main_call",
        scope: ActScope::AST,
        min_tx_type: 3,
        extra9: |_: &ContractMainCall| false,
        req_sign: |_: &ContractMainCall| vec![],
        as_transfer_like: none,
        description: |this: &ContractMainCall| {
            format!("Run main codes with conf {}", this.codeconf.uint())
        },

    }
}

pub fn create_contract_main_call(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = ContractMainCall::decode(buf)?;
    Ok((Arc::new(action), used))
}
