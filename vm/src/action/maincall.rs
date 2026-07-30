//! `ContractMainCall` (kind 44) top-level action.
//!
//! Ported from fullnodedev `vm/src/action/maincall.rs`. The main entry runs
//! arbitrary VM bytecode at tx scope; the codes are verified against the
//! runtime `SpaceCap`/`GasExtra` (height-derived) before being handed to the VM
//! via `VmRequest::Main`.

use std::any::Any;
use std::sync::Arc;

use base::{ActOut, ActScope, Action, ActionRef, Context, VmEntry};
use field::{BytesW2, Encode, Fixed3, Reader, Uint1, Uint2};
use sys::Ret;

use crate::contract::convert_and_check;
use crate::machine::{VmRequest, peek_vm_runtime_limits};
use crate::rt::{CodeConf, CodeType};

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl Encode for ContractMainCall {
    fn size(&self) -> usize {
        self.kind.size() + self.marks.size() + self.codeconf.size() + self.codes.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.marks.encode_to(out);
        self.codeconf.encode_to(out);
        self.codes.encode_to(out);
    }
}

impl Action for ContractMainCall {
    fn kind(&self) -> u16 {
        Self::KIND
    }

    fn scope(&self) -> ActScope {
        ActScope::AST
    }

    fn min_tx_type(&self) -> u8 {
        3
    }

    fn extra9(&self) -> bool {
        false
    }

    fn req_sign(&self) -> Vec<base::AddrOrPtr> {
        vec![]
    }

    fn description(&self) -> String {
        format!("Run main codes with conf {}", self.codeconf.uint())
    }

    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        contract_main_call_execute(self, ctx)?;
        Ok((gas, vec![]))
    }

    fn as_any(&self) -> &dyn Any {
        self
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
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != ContractMainCall::KIND {
        return sys::decodef!("ContractMainCall codec got kind {}", kind.uint());
    }
    let marks: Fixed3 = r.read()?;
    let codeconf: Uint1 = r.read()?;
    let codes: BytesW2 = r.read()?;
    Ok((
        Arc::new(ContractMainCall {
            kind,
            marks,
            codeconf,
            codes,
        }),
        r.used(),
    ))
}
