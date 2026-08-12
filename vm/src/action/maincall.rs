//! `ContractMainCall` (kind 44) top-level action.
//!
//! Ported from fullnodedev `vm/src/action/maincall.rs`. The main entry runs
//! arbitrary VM bytecode at tx scope; the codes are verified against the
//! runtime `SpaceCap`/`GasExtra` (height-derived) before being handed to the VM
//! via `VmRequest::Main`.

use std::sync::Arc;

use base::{ActScope, ActionRef, Context, VmEntry};
use field::{BytesW2, Decode, Encode, Fixed3, Uint1, Uint2};
use sys::Ret;

use crate::contract::convert_and_check;
use crate::machine::{VmRequest, peek_vm_runtime_limits};
use crate::rt::{CodeConf, CodeType};

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
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

base::impl_action! {
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
        execute: (self, ctx) {
        contract_main_call_execute(self, ctx)?;
        Ok(vec![])
        }
    }
}

fn contract_main_call_execute(this: &ContractMainCall, ctx: &mut dyn Context) -> Ret<()> {
    if !this.marks.is_zero() {
        return sys::errf!("marks bytes format invalid");
    }
    // check codes
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    let codeconf = CodeConf::parse(this.codeconf.uint()).map_err(sys::Error::from)?;
    convert_and_check(
        &cap,
        &gst,
        codeconf.code_type(),
        this.codes.as_vec(),
        hei,
        ctx.services().as_ref(),
    )
    .map_err(sys::Error::from)?;
    let _ = ctx.vm_call(VmEntry::Raw(Box::new(VmRequest::Main {
        code_type: codeconf.code_type(),
        codes: Arc::from(this.codes.to_vec()),
    })))?;
    Ok(())
}

pub fn create_contract_main_call(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = ContractMainCall::decode(buf)?;
    Ok((Arc::new(action), used))
}
