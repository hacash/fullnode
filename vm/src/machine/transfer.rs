use std::any::Any;
use std::sync::Arc;

use base::{Context, GasBuckets, IntentScope, TransferPayload, TransferRouting};
use field::{Address, BytesW2};
use sys::Ret;

use crate::action::P2SHScriptProve;
use crate::contract::ContractAddrListW1;
use crate::rt::{AbstCall, CodeConf, CodeType, EntryKind, FnObj, FrameBindings, ItrErr, VmrtRes};
use crate::value::{ContractAddress, Value};

use super::{StubVm, peek_vm_runtime_limits};

struct TransferCall {
    kind: AbstCall,
    param: Value,
}

impl StubVm {
    fn p2sh_call_raw(
        &mut self,
        ctx: &mut dyn Context,
        code_type: CodeType,
        codes: Arc<[u8]>,
        context_addr: Address,
        libs: Arc<[Address]>,
        intent_scope: IntentScope,
        param: Value,
    ) -> Result<Value, ItrErr> {
        let exec = EntryKind::P2sh.root_exec();
        exec.ensure_call_depth(&self.runtime.warm.space_cap)?;
        param.check_vm_boundary_argv()?;
        param.check_boundary_value_cap(&self.runtime.warm.space_cap)?;
        param.check_container_cap(&self.runtime.warm.space_cap)?;
        let fnobj = FnObj::plain(code_type, codes, 0, None);
        self.do_call(
            ctx,
            exec,
            &fnobj,
            FrameBindings::root(context_addr, libs).with_intent_scope(intent_scope),
            Some(param),
        )
    }

    fn run_p2sh_entry(
        &mut self,
        ctx: &mut dyn Context,
        owner: Address,
        to: Address,
        action_kind: u16,
        payload: TransferPayload,
        intent_scope: IntentScope,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        let p2sh = ctx.p2sh(&owner)?;
        let code_conf = CodeConf::parse(p2sh.code_conf()).map_err(sys::Error::from)?;
        let (context_addr, libs, codes) = parse_p2sh_code_stuff(p2sh.code_stuff())?;
        let mut payload_args = p2sh_transfer_args(payload);
        let mut params = vec![
            Value::Bytes(p2sh.witness().to_vec()),
            Value::U16(action_kind),
            Value::Address(to),
        ];
        params.append(&mut payload_args);
        // The dev P2SH transfer ABI always exposes five arguments. HAC/SAT
        // payloads occupy one slot, so preserve the trailing second payload
        // slot with Nil instead of changing the call shape by asset type.
        while params.len() < 5 {
            params.push(Value::Nil);
        }
        let param = Value::pack_call_args(params).map_err(sys::Error::from)?;
        // Mirror dev `run_p2sh_entry`: re-validate the stored unlock inputs at
        // every p2sh VM entry before dispatch. The object was verified at
        // P2SHScriptProve set time; re-checking keeps the entry boundary robust
        // against any future p2sh_set source and guards caches against forged blobs.
        verify_p2sh_entry_inputs(ctx, code_conf, &libs, &codes, &param)?;
        let label = format!("p2sh transfer authorize {}", owner.to_readable());
        let (cost, rv) = self.run_entry(ctx, EntryKind::P2sh, move |vm, ctx| {
            vm.p2sh_call_raw(
                ctx,
                code_conf.code_type(),
                codes.into(),
                context_addr,
                libs.into(),
                intent_scope,
                param,
            )
        })?;
        Self::check_vm_return_value(&rv, &label)?;
        Ok((cost, Box::new(rv)))
    }

    pub(super) fn run_transfer_authorize(
        &mut self,
        ctx: &mut dyn Context,
        owner: Address,
        to: Address,
        action_kind: u16,
        payload: TransferPayload,
        intent_scope: IntentScope,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        if owner.is_scriptmh() {
            return self.run_p2sh_entry(ctx, owner, to, action_kind, payload, intent_scope);
        }
        let contract_addr = ContractAddress::from_addr(owner)?;
        let call = transfer_call(true, to, payload)?;
        self.run_abst_entry(ctx, call.kind, contract_addr, intent_scope, call.param)
    }

    pub(super) fn run_transfer_receive(
        &mut self,
        ctx: &mut dyn Context,
        from: Address,
        to: Address,
        payload: TransferPayload,
        intent_scope: IntentScope,
    ) -> Ret<(GasBuckets, Box<dyn Any>)> {
        let contract_addr = ContractAddress::from_addr(to)?;
        let call = transfer_call(false, from, payload)?;
        self.run_abst_entry(ctx, call.kind, contract_addr, intent_scope, call.param)
    }

    pub(super) fn drive_transfer_inner(
        &mut self,
        ctx: &mut dyn Context,
        routing: TransferRouting,
        intent_scope: IntentScope,
    ) -> VmrtRes<()> {
        let map_error = |e: sys::Error| {
            ItrErr::new(
                if e.is_revert() {
                    crate::rt::ItrErrCode::ActCallRevert
                } else {
                    // Preserve `Abort` classification at this `sys::Error ->
                    // ItrErr` conversion point so a transfer-hook state read
                    // failure stays fatal (§5).
                    crate::rt::map_native_action_code(&e)
                },
                e.as_str(),
            )
        };
        if routing.authorize {
            self.run_transfer_authorize(
                ctx,
                routing.from,
                routing.to,
                routing.action_kind,
                routing.payload.clone(),
                intent_scope,
            )
            .map_err(map_error)?;
        }
        if routing.receive {
            self.run_transfer_receive(ctx, routing.from, routing.to, routing.payload, intent_scope)
                .map_err(map_error)?;
        }
        Ok(())
    }
}

fn transfer_call(
    authorize: bool,
    counterparty: Address,
    payload: TransferPayload,
) -> Ret<TransferCall> {
    let (kind, args) = match payload {
        TransferPayload::Hac { amount } => (
            if authorize {
                AbstCall::PermitHAC
            } else {
                AbstCall::PayableHAC
            },
            vec![Value::Address(counterparty), Value::Bytes(amount)],
        ),
        TransferPayload::Sat { satoshi } => (
            if authorize {
                AbstCall::PermitSAT
            } else {
                AbstCall::PayableSAT
            },
            vec![Value::Address(counterparty), Value::U64(satoshi)],
        ),
        TransferPayload::Hacd { count, names } => (
            if authorize {
                AbstCall::PermitHACD
            } else {
                AbstCall::PayableHACD
            },
            vec![
                Value::Address(counterparty),
                Value::U32(count),
                Value::Bytes(names),
            ],
        ),
        TransferPayload::Asset { serial, amount } => (
            if authorize {
                AbstCall::PermitAsset
            } else {
                AbstCall::PayableAsset
            },
            vec![
                Value::Address(counterparty),
                Value::U64(serial),
                Value::U64(amount),
            ],
        ),
    };
    let param = Value::pack_call_args(args).map_err(sys::Error::from)?;
    Ok(TransferCall { kind, param })
}

fn p2sh_transfer_args(payload: TransferPayload) -> Vec<Value> {
    match payload {
        TransferPayload::Hac { amount } => vec![Value::Bytes(amount)],
        TransferPayload::Sat { satoshi } => vec![Value::U64(satoshi)],
        TransferPayload::Hacd { count, names } => {
            vec![Value::U32(count), Value::Bytes(names)]
        }
        TransferPayload::Asset { serial, amount } => {
            vec![Value::U64(serial), Value::U64(amount)]
        }
    }
}

fn parse_p2sh_code_stuff(raw: &[u8]) -> Ret<(Address, Vec<Address>, Arc<[u8]>)> {
    if raw.len() < Address::SIZE + 1 {
        return sys::errf!("p2sh code stuff too short");
    }
    let mut addr = [0u8; Address::SIZE];
    addr.copy_from_slice(&raw[..Address::SIZE]);
    let context_addr = Address::from(addr);
    if !context_addr.is_scriptmh() {
        return sys::errf!("p2sh context address must be scriptmh");
    }
    let lib_count = raw[Address::SIZE] as usize;
    let libs_start = Address::SIZE + 1;
    let codes_start = libs_start + lib_count * Address::SIZE;
    if raw.len() < codes_start {
        return sys::errf!("p2sh library table truncated");
    }
    let mut libs = Vec::with_capacity(lib_count);
    for item in raw[libs_start..codes_start].chunks_exact(Address::SIZE) {
        let mut lib = [0u8; Address::SIZE];
        lib.copy_from_slice(item);
        libs.push(Address::from(lib));
    }
    Ok((context_addr, libs, Arc::from(&raw[codes_start..])))
}

/// Extract the P2SH witness bytes from the entry parameter (dev-compatible:
/// bare `Bytes`, or a tuple whose first item is the witness).
fn extract_p2sh_witness(param: &Value) -> Ret<Vec<u8>> {
    match param {
        Value::Bytes(witness) => Ok(witness.clone()),
        Value::Tuple(items) => {
            let Some(first) = items.as_slice().first() else {
                return sys::errf!("p2sh param tuple is empty");
            };
            let Value::Bytes(witness) = first else {
                return sys::errf!("p2sh witness must be the first tuple item as bytes");
            };
            Ok(witness.clone())
        }
        _ => sys::errf!("p2sh param must be bytes or a tuple starting with witness bytes"),
    }
}

/// Dev `run_p2sh_entry` re-validation: parse the stored unlock inputs and run
/// `P2SHScriptProve::verify_unlock_inputs` (libs allowlist, lockbox size,
/// code convert+check, witness size) before any VM dispatch or cache warmup.
fn verify_p2sh_entry_inputs(
    ctx: &mut dyn Context,
    code_conf: CodeConf,
    libs: &[Address],
    codes: &[u8],
    param: &Value,
) -> Ret<()> {
    let hei = ctx.env().block.height;
    let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
    let witness = BytesW2::from(extract_p2sh_witness(param)?).map_err(sys::Error::from)?;
    let lib_list = ContractAddrListW1::from(
        libs.iter()
            .map(|addr| ContractAddress::from_addr(*addr))
            .collect::<Ret<Vec<_>>>()?,
    )
    .map_err(sys::Error::from)?;
    let lockbox = BytesW2::from(codes.to_vec()).map_err(sys::Error::from)?;
    P2SHScriptProve::verify_unlock_inputs(
        hei,
        &gst,
        &cap,
        &lib_list,
        code_conf,
        &lockbox,
        &witness,
        ctx.services().as_ref(),
    )
}

// `convert_and_check` runs the real IR compiler unconditionally now; these
// entry-validation tests exercise the full execution implementation.
#[cfg(test)]
mod transfer_tests {
    use super::*;
    use crate::machine::test_ctx::TestCtx;
    use crate::rt::{Bytecode, SpaceCap};

    /// Dev `run_p2sh_entry` re-validation: the entry boundary rejects a
    /// witness larger than `SpaceCap::value_size` before any VM dispatch.
    #[test]
    fn verify_p2sh_entry_inputs_rejects_oversized_witness() {
        let mut ctx = TestCtx::new();
        let cap = SpaceCap::new(1);
        let oversized = vec![0u8; cap.value_size + 1];
        assert!(
            oversized.len() <= u16::MAX as usize,
            "witness must fit BytesW2"
        );
        let param = Value::pack_call_args(vec![Value::Bytes(oversized)]).unwrap();
        let codes: Arc<[u8]> = Arc::from(vec![Bytecode::END as u8]);
        let err = verify_p2sh_entry_inputs(
            &mut ctx,
            CodeConf::from_type(CodeType::Bytecode),
            &[],
            &codes,
            &param,
        )
        .unwrap_err();
        assert!(err.to_string().contains("witness bytes too long"), "{err}");
    }

    /// Dev `run_p2sh_entry` re-validation: non-contract-version library
    /// addresses are rejected before VM dispatch (mirrors `ContractAddressW1`
    /// payload decode failing in fullnodedev).
    #[test]
    fn verify_p2sh_entry_inputs_rejects_non_contract_lib() {
        let mut ctx = TestCtx::new();
        let lib = Address::from([0u8; 21]); // version 0, not a contract
        let param = Value::pack_call_args(vec![Value::Bytes(vec![1u8])]).unwrap();
        let codes: Arc<[u8]> = Arc::from(vec![Bytecode::END as u8]);
        let err = verify_p2sh_entry_inputs(
            &mut ctx,
            CodeConf::from_type(CodeType::Bytecode),
            &[lib],
            &codes,
            &param,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("is not CONTRACT"),
            "unexpected error: {err}"
        );
    }

    /// Valid inputs (empty libs, valid END bytecode, small witness) pass the
    /// entry re-validation.
    #[test]
    fn verify_p2sh_entry_inputs_accepts_valid_inputs() {
        let mut ctx = TestCtx::new();
        let param = Value::pack_call_args(vec![Value::Bytes(vec![1u8])]).unwrap();
        let codes: Arc<[u8]> = Arc::from(vec![Bytecode::END as u8]);
        verify_p2sh_entry_inputs(
            &mut ctx,
            CodeConf::from_type(CodeType::Bytecode),
            &[],
            &codes,
            &param,
        )
        .unwrap();
    }
}
