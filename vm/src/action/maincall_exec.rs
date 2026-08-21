//! ContractMainCall execute body.

use std::sync::Arc;

use base::{Context, VmEntry};
use sys::Ret;

use super::maincall::ContractMainCall;
use crate::contract::convert_and_check;
use crate::machine::{VmRequest, peek_vm_runtime_limits};
use crate::rt::CodeConf;

fn contract_main_call_execute(this: &ContractMainCall, ctx: &mut dyn Context) -> Ret<()> {
    if !this.marks.is_zero() {
        return sys::errf!("marks bytes format invalid");
    }
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

base::impl_action_execute! {
    ContractMainCall {
        (self, ctx) {
            contract_main_call_execute(self, ctx)?;
            Ok(vec![])
        }
    }
}
