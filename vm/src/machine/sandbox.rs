//! Ephemeral contract call sandbox (query API / debugging).
//!
//! Operates on a caller-provided `Context` that already has a forked exec layer
//! and a sandbox transaction. Does **not** use a global RuntimePool — the VM
//! comes from `Registry::assign_vm` via context construction.

use std::sync::Arc;

use base::{
    Context, ExecFrom, GasBuckets, TX_GAS_BUDGET_CAP_BYTE, VmEntry, decode_gas_budget, hac_add,
    with_exec_from,
};
use field::{Address, Amount};
use sys::{Rerr, Ret, errf};

use crate::rt::{
    Bytecode, CodeType, FN_SIGN_WIDTH, FnSign, calc_func_sign, verify_bytecodes_with_registry,
};
use crate::value::{CallArgsPack, ContractAddress, Value, ValueTy, classify_call_args_len};

use super::VmRequest;

const SANDBOX_TX_FEE_238: u64 = 100_000;
const SANDBOX_FUND_238: u64 = 10_000_000_000;

/// Default sandbox tx fee unit238 (for API context construction).
pub const SANDBOX_TX_FEE: u64 = SANDBOX_TX_FEE_238;

#[derive(Debug, Clone)]
pub struct SandboxSpec {
    pub contract: ContractAddress,
    pub function: String,
    pub args: Vec<Value>,
    pub caller: Option<Address>,
    pub gas_budget: Option<i64>,
    pub gas_max_byte: Option<u8>,
}

impl SandboxSpec {
    pub fn new(contract: ContractAddress, function: impl Into<String>) -> Self {
        Self {
            contract,
            function: function.into(),
            args: vec![],
            caller: None,
            gas_budget: None,
            gas_max_byte: None,
        }
    }

    pub fn args(mut self, args: Vec<Value>) -> Self {
        self.args = args;
        self
    }

    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = Some(caller);
        self
    }

    pub fn gas_budget(mut self, gas_budget: i64) -> Self {
        self.gas_budget = Some(gas_budget);
        self
    }

    pub fn gas_max_byte(mut self, gas_max_byte: u8) -> Self {
        self.gas_max_byte = Some(gas_max_byte);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxResult {
    pub use_gas: i64,
    pub gas_use: GasBuckets,
    pub ret_val: Value,
}

/// Run a read-only-ish sandbox call on an already-prepared context.
///
/// The context must own a forked state layer and a tx whose `main`/addrlist
/// match the intended caller and contract. Funding and gas init happen here.
pub fn sandbox_call(ctx: &mut dyn Context, spec: SandboxSpec) -> Ret<SandboxResult> {
    let (_tx_gas_max, gas_budget) = resolve_sandbox_gas(&spec)?;
    let codes = build_call_codes(&spec.function, &spec.args)?;
    verify_bytecodes_with_registry(&codes, ctx.services().as_ref())
        .map_err(|e| sys::Error::from(e))?;
    let caller = spec.caller.unwrap_or_else(|| ctx.tx().main());
    hac_add(ctx, &caller, &Amount::unit238(SANDBOX_FUND_238))?;
    ctx.gas_initialize(gas_budget)?;
    let (gas_use, ret_box) = with_exec_from(ctx, ExecFrom::Call, |ctx| {
        ctx.vm_call(VmEntry::Raw(Box::new(VmRequest::Main {
            code_type: CodeType::Bytecode,
            codes: Arc::from(codes),
        })))
    })?;
    let ret_val = *ret_box
        .downcast::<Value>()
        .map_err(|_| sys::Error::fault("sandbox return type mismatch"))?;
    Ok(SandboxResult {
        use_gas: gas_use.total(),
        gas_use,
        ret_val,
    })
}

pub fn resolve_sandbox_gas(spec: &SandboxSpec) -> Ret<(u8, i64)> {
    match spec.gas_max_byte {
        Some(0) => errf!("sandbox gas_max byte invalid: 0"),
        Some(gmx) => {
            let capped = gmx.min(TX_GAS_BUDGET_CAP_BYTE);
            Ok((gmx, decode_gas_budget(capped)))
        }
        None => {
            let cap_budget = decode_gas_budget(TX_GAS_BUDGET_CAP_BYTE);
            let gas_budget = match spec.gas_budget {
                Some(v) if v > 0 => v.min(cap_budget),
                Some(v) => return errf!("sandbox gas budget invalid: {}", v),
                None => cap_budget,
            };
            Ok((TX_GAS_BUDGET_CAP_BYTE, gas_budget))
        }
    }
}

pub fn parse_sandbox_params(pms: &str) -> Ret<Vec<Value>> {
    let mut values = vec![];
    for part in pms.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (v, t) = match part.split_once(':') {
            Some((v, t)) => (v.trim(), t.trim()),
            None => (part, "nil"),
        };
        values.push(parse_one_param(t, v)?);
    }
    Ok(values)
}

pub fn build_call_codes(funcname: &str, args: &[Value]) -> Ret<Vec<u8>> {
    use Bytecode::*;

    let mut codes = vec![];
    for arg in args {
        append_push_value_code(&mut codes, arg)?;
    }
    match classify_call_args_len(args.len()).map_err(sys::Error::from)? {
        CallArgsPack::Nil => codes.push(PNIL as u8),
        CallArgsPack::Raw => {}
        CallArgsPack::Tuple => {
            codes.push(PU8 as u8);
            codes.push(args.len() as u8);
            codes.push(PACKTUPLE as u8);
        }
    }
    let fnsg = parse_sandbox_func_sign(funcname)?;
    codes.push(CALLEXT as u8);
    codes.push(1);
    codes.extend_from_slice(&fnsg);
    codes.push(RET as u8);
    Ok(codes)
}

fn parse_sandbox_func_sign(funcname: &str) -> Ret<FnSign> {
    if let Some(hexsig) = funcname.strip_prefix("0x") {
        if hexsig.len() != FN_SIGN_WIDTH * 2 {
            return errf!(
                "sandbox function selector length invalid: expected {} hex chars",
                FN_SIGN_WIDTH * 2
            );
        }
        let raw = hex::decode(hexsig)
            .map_err(|_| sys::Error::fault("sandbox function selector hex invalid"))?;
        return raw
            .try_into()
            .map_err(|_| sys::Error::fault("sandbox function selector length invalid"));
    }
    Ok(calc_func_sign(funcname))
}

fn append_push_value_code(codes: &mut Vec<u8>, value: &Value) -> Rerr {
    use Bytecode::*;
    use Value::*;

    match value {
        Nil => codes.push(PNIL as u8),
        Bool(true) => codes.push(PTRUE as u8),
        Bool(false) => codes.push(PFALSE as u8),
        U8(n) => {
            codes.push(PU8 as u8);
            codes.push(*n);
        }
        U16(n) => {
            codes.push(PU16 as u8);
            codes.extend_from_slice(&n.to_be_bytes());
        }
        U32(n) => {
            append_push_bytes_code(codes, &n.to_be_bytes());
            codes.push(CU32 as u8);
        }
        U64(n) => {
            append_push_bytes_code(codes, &n.to_be_bytes());
            codes.push(CU64 as u8);
        }
        U128(n) => {
            append_push_bytes_code(codes, &n.to_be_bytes());
            codes.push(CU128 as u8);
        }
        Bytes(buf) => append_push_bytes_code(codes, buf),
        Address(addr) => {
            append_push_bytes_code(codes, addr.as_bytes());
            codes.push(CTO as u8);
            codes.push(ValueTy::Address as u8);
        }
        Tuple(_) | Handle(_) | Compo(_) => {
            return errf!("sandbox argument type {:?} not supported", value.ty());
        }
    }
    Ok(())
}

fn append_push_bytes_code(codes: &mut Vec<u8>, bytes: &[u8]) {
    use Bytecode::*;

    if bytes.len() <= u8::MAX as usize {
        codes.push(PBUF as u8);
        codes.push(bytes.len() as u8);
    } else {
        codes.push(PBUFL as u8);
        codes.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    codes.extend_from_slice(bytes);
}

fn parse_one_param(t: &str, v: &str) -> Ret<Value> {
    let ty = ValueTy::from_name(t).map_err(sys::Error::from)?;
    Ok(match ty {
        ValueTy::Nil => Value::Nil,
        ValueTy::Bool => match v {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return errf!("invalid bool argument '{}'", v),
        },
        ValueTy::U8 => Value::U8(
            v.parse::<u8>()
                .map_err(|e| sys::Error::fault(format!("invalid u8 argument '{}': {}", v, e)))?,
        ),
        ValueTy::U16 => Value::U16(
            v.parse::<u16>()
                .map_err(|e| sys::Error::fault(format!("invalid u16 argument '{}': {}", v, e)))?,
        ),
        ValueTy::U32 => Value::U32(
            v.parse::<u32>()
                .map_err(|e| sys::Error::fault(format!("invalid u32 argument '{}': {}", v, e)))?,
        ),
        ValueTy::U64 => Value::U64(
            v.parse::<u64>()
                .map_err(|e| sys::Error::fault(format!("invalid u64 argument '{}': {}", v, e)))?,
        ),
        ValueTy::U128 => Value::U128(
            v.parse::<u128>()
                .map_err(|e| sys::Error::fault(format!("invalid u128 argument '{}': {}", v, e)))?,
        ),
        ValueTy::Address => {
            Value::Address(field::Address::from_readable(v).map_err(|e| {
                sys::Error::fault(format!("invalid address argument '{}': {}", v, e))
            })?)
        }
        ValueTy::Bytes => {
            let hex_body = v.strip_prefix("0x").unwrap_or(v);
            Value::Bytes(
                hex::decode(hex_body).map_err(|e| {
                    sys::Error::fault(format!("invalid bytes argument '{}': {}", v, e))
                })?,
            )
        }
        _ => return errf!("unsupported param type '{}'", t),
    })
}
