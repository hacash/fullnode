//! P2SHScriptProve execute body.

use base::{Context, ExecFrom};
use field::{Address, Encode};
use sys::{Rerr, errf};

use super::p2sh::{P2SHScriptProve, UnlockScript};
use crate::machine::peek_vm_runtime_limits;
use crate::rt::CodeConf;

fn p2sh_script_prove_execute(this: &P2SHScriptProve, ctx: &mut dyn Context) -> Rerr {
    if ctx.exec_from() != ExecFrom::Top {
        return errf!(
            "P2SHScriptProve can only run in TOP context, got {}",
            ctx.exec_from()
        );
    }
    if !this.marks.is_zero() {
        return errf!("marks bytes format invalid");
    }
    let hei = ctx.env().block.height;
    let (_, cap) = peek_vm_runtime_limits(ctx, hei);
    let p2sh_count = ctx.p2sh_count();
    if p2sh_count >= cap.p2sh_set {
        return errf!("p2sh_set overflow (>={})", cap.p2sh_set);
    }
    P2SHScriptProve::verify_merkle_depth(&cap, this.merkels.as_list().len())?;
    let adr = this.get_merkel()?;
    let stuff = get_stuff_with_merkel(this, ctx, &adr)?;
    ctx.p2sh_set(adr, Box::new(stuff))?;
    Ok(())
}

fn get_stuff_with_merkel(
    this: &P2SHScriptProve,
    ctx: &mut dyn Context,
    scriptmh: &Address,
) -> sys::Ret<UnlockScript> {
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    let codeconf = CodeConf::parse(this.codeconf.uint()).map_err(sys::Error::from)?;
    P2SHScriptProve::verify_unlock_inputs(
        hei,
        &gst,
        &cap,
        &this.adrlibs,
        codeconf,
        &this.lockbox,
        &this.argvkey,
        ctx.services().as_ref(),
    )?;
    let lockbox = this.lockbox.to_vec();
    let witness = this.argvkey.to_vec();
    let merkel = scriptmh.as_bytes().to_vec();
    let libs = this.adrlibs.encode();
    let mut stuff = Vec::with_capacity(merkel.len() + libs.len() + lockbox.len());
    stuff.extend_from_slice(&merkel);
    stuff.extend_from_slice(&libs);
    stuff.extend_from_slice(&lockbox);
    Ok(UnlockScript {
        codeconf: codeconf.raw(),
        stuff,
        witness,
    })
}

base::impl_action_execute! {
    P2SHScriptProve {
        (self, ctx) {
            p2sh_script_prove_execute(self, ctx)?;
            Ok(vec![])
        }
    }
}
