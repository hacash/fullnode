use std::any::Any;
use std::sync::Arc;

use base::{Context, GasBuckets, IntentScope, TransferPayload, TransferRouting};
use field::Address;
use sys::Ret;

use crate::rt::{AbstCall, CodeConf, CodeType, EntryKind, FnObj, FrameBindings, ItrErr, VmrtRes};
use crate::value::{ContractAddress, Value};

use super::StubVm;

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
                    crate::rt::ItrErrCode::ActCallError
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
