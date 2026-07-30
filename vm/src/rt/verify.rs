use base::{ExecutionServices, VmHostCallKind};

use crate::native::{NativeCtl, NativeEnv, NativeFunc};
use crate::value::{parse_cto_target_ty_param, parse_value_ty_param};

use super::{
    Bytecode, ItrErr, ItrErrCode, VmrtErr, VmrtRes, decode_user_call_site, is_fin_family,
    is_user_call_inst, verify_fin_runtime_supported,
};
use Bytecode::*;
use ItrErrCode::*;

pub fn verify_bytecodes(codes: &[u8]) -> VmrtRes<Vec<u8>> {
    const VERIFY_MAX_PUSH_BUF_LEN: usize = 1280;
    // No Registry: skip ACTION/ACTENV/ACTVIEW id checks (IR/compile paths).
    verify_bytecodes_with_limits(
        codes,
        VERIFY_MAX_PUSH_BUF_LEN,
        VerifyEntryStack::OptionalArgv,
        None,
    )
}

pub fn verify_bytecodes_with_registry(
    codes: &[u8],
    registry: &dyn ExecutionServices,
) -> VmrtRes<Vec<u8>> {
    const VERIFY_MAX_PUSH_BUF_LEN: usize = 1280;
    verify_bytecodes_with_limits(
        codes,
        VERIFY_MAX_PUSH_BUF_LEN,
        VerifyEntryStack::OptionalArgv,
        Some(registry),
    )
}

/// Deploy/maincall path: push-buf limit follows SpaceCap::value_size.
pub fn verify_bytecodes_for_cap(
    codes: &[u8],
    max_push_buf_len: usize,
    registry: &dyn ExecutionServices,
) -> VmrtRes<Vec<u8>> {
    verify_bytecodes_with_limits(
        codes,
        max_push_buf_len,
        VerifyEntryStack::OptionalArgv,
        Some(registry),
    )
}

/// Initial operand-stack shape assumed by bytecode stack verification.
///
/// Existing runtime entries may execute with no argv value, or with one argv
/// value already pushed onto the operand stack. Keep the default verifier
/// compatible with both, and let stricter callers opt into the exact shape they
/// know they will use.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum VerifyEntryStack {
    Empty,
    Argv,
    OptionalArgv,
}

pub fn verify_bytecodes_with_entry_stack(
    codes: &[u8],
    entry_stack: VerifyEntryStack,
) -> VmrtRes<Vec<u8>> {
    const VERIFY_MAX_PUSH_BUF_LEN: usize = 1280;
    verify_bytecodes_with_limits(codes, VERIFY_MAX_PUSH_BUF_LEN, entry_stack, None)
}

pub fn verify_bytecodes_with_entry_stack_and_registry(
    codes: &[u8],
    entry_stack: VerifyEntryStack,
    registry: &dyn ExecutionServices,
) -> VmrtRes<Vec<u8>> {
    const VERIFY_MAX_PUSH_BUF_LEN: usize = 1280;
    verify_bytecodes_with_limits(codes, VERIFY_MAX_PUSH_BUF_LEN, entry_stack, Some(registry))
}

fn verify_bytecodes_with_limits(
    codes: &[u8],
    max_push_buf_len: usize,
    entry_stack: VerifyEntryStack,
    registry: Option<&dyn ExecutionServices>,
) -> VmrtRes<Vec<u8>> {
    let cl = codes.len();
    if cl == 0 {
        return itr_err_code!(CodeEmpty);
    }
    if cl > u16::MAX as usize {
        return itr_err_code!(CodeTooLong);
    }
    let (instable, jumpdests) = verify_valid_instruction(codes, max_push_buf_len, registry)?;
    // check jump dests
    verify_jump_dests(&instable, &jumpdests)?;
    // Stack verification is intentionally layered. The linear verifier catches
    // definite underflow in straight-line bytecode, then stops at control-flow,
    // calls, and stack effects whose arity cannot be known without value
    // interpretation. Runtime Stack::pop remains the final guard.
    verify_linear_stack_effects(codes, entry_stack)?;
    verify_literal_fin_stack_prefix(codes)?;
    // ok finish
    Ok(instable)
}

/// Ensure the last instruction is a terminal one (RET/END/ERR/ABT or exposed call opcode).
/// Failure (CodeNotWithEnd) is a fitsh code compile error and propagates to the compiler
/// via compile_body -> parse_function -> parse_top_level -> fitshc::compile.
pub fn ensure_terminal_instruction(inst: Bytecode) -> VmrtErr {
    if matches!(inst, RET | END | ERR | ABT) || is_user_call_inst(inst) {
        return Ok(());
    }
    itr_err_code!(CodeNotWithEnd)
}

fn ensure_act_id(registry: Option<&dyn ExecutionServices>, act_kind: Bytecode, id: u8) -> VmrtErr {
    let Some(reg) = registry else {
        // No registry attached: defer ACTION id checks to deploy-time verify.
        return Ok(());
    };
    let kind = match act_kind {
        ACTION => VmHostCallKind::Action,
        ACTENV => VmHostCallKind::Env,
        ACTVIEW => VmHostCallKind::View,
        _ => {
            return itr_err_fmt!(InstParamsErr, "unexpected act kind {:?}", act_kind);
        }
    };
    if reg.vm_host_def(kind, id).is_none() {
        let name = match act_kind {
            ACTION => "ACTION",
            ACTENV => "ACTENV",
            ACTVIEW => "ACTVIEW",
            _ => "ACT",
        };
        return itr_err_fmt!(InstParamsErr, "{} id {} not found", name, id);
    }
    Ok(())
}

/*

*/
fn verify_valid_instruction(
    codes: &[u8],
    max_push_buf_len: usize,
    registry: Option<&dyn ExecutionServices>,
) -> VmrtRes<(Vec<u8>, Vec<isize>)> {
    // use Bytecode::*;
    let cdlen = codes.len(); // end/ret/err/abt in tail
    let mut instable = vec![0u8; cdlen];
    let mut jumpdest = vec![];
    let mut i = 0;
    let mut cur = END;
    while i < cdlen {
        let curbt = codes[i];
        let inst = Bytecode::try_from_u8(curbt)?;
        let meta = inst.metadata();
        if !meta.valid {
            return itr_err_fmt!(InstInvalid, "{}", inst as u8);
        }
        match inst {
            IRBYTECODE | IRLIST | IRBLOCK | IRBLOCKR | IRIF | IRIFR | IRWHILE | IRBREAK
            | IRCONTINUE => {
                return itr_err_fmt!(InstInvalid, "IR bytecode {:?} is not allowed", inst);
            }
            _ => {}
        }
        cur = inst;
        instable[i] = 1; // yes is valid instruction
        i += 1;
        macro_rules! pu8 {
            () => {{
                if i >= cdlen {
                    return itr_err_code!(InstParamsErr);
                }
                codes[i as usize]
            }};
        }
        macro_rules! pu16 {
            () => {{
                let r = i + 2;
                if r > cdlen {
                    return itr_err_code!(InstParamsErr);
                }
                u16::from_be_bytes(codes[i as usize..r as usize].try_into().unwrap())
            }};
        }
        macro_rules! pi8 {
            () => {
                pu8!() as i8
            };
        }
        macro_rules! pi16 {
            () => {
                pu16!() as i16
            };
        }
        macro_rules! adddest {
            ($jt:expr) => {{ jumpdest.push($jt) }};
        }
        match inst {
            // push buf
            PBUF => {
                let l = pu8!() as usize;
                if l > max_push_buf_len {
                    return itr_err_fmt!(InstParamsErr, "PBUF size {} too large", l);
                }
                i += l
            }
            PBUFL => {
                let l = pu16!() as usize;
                if l > max_push_buf_len {
                    return itr_err_fmt!(InstParamsErr, "PBUFL size {} too large", l);
                }
                i += l
            }
            // ext/native
            ACTION | ACTENV | ACTVIEW => ensure_act_id(registry, inst, pu8!())?,
            NTFUNC => {
                let idx = pu8!();
                if !NativeFunc::has_idx(idx) {
                    return itr_err_fmt!(NativeFuncError, "native func idx {} not found", idx);
                }
            }
            NTCTL => {
                let idx = pu8!();
                if !NativeCtl::has_idx(idx) {
                    return itr_err_fmt!(NativeCtlError, "native ctl idx {} not found", idx);
                }
            }
            NTENV => {
                let idx = pu8!();
                if !NativeEnv::has_idx(idx) {
                    return itr_err_fmt!(NativeEnvError, "native env idx {} not found", idx);
                }
            }
            _ if is_fin_family(inst) => {
                let sub = pu8!();
                verify_fin_runtime_supported(inst, sub)?;
            }
            CTO => {
                let _ = parse_cto_target_ty_param(pu8!())?;
            }
            TIS => {
                let _ = parse_value_ty_param(pu8!())?;
            }
            _ if is_user_call_inst(inst) => {
                let r = i + meta.param as usize;
                if r > cdlen {
                    return itr_err_code!(InstParamsErr);
                }
                let _ = decode_user_call_site(inst, &codes[i..r])?;
            }
            // jump record
            JMPL | BRL => adddest!(pu16!() as isize),
            JMPS | BRS => adddest!(i as isize + pi8!() as isize + 1),
            JMPSL | BRSL | BRSLN => adddest!(i as isize + pi16!() as isize + 2),
            _ => {}
        };
        i += meta.param as usize;
        if i > cdlen {
            return itr_err_code!(InstParamsErr);
        }
        // next
    }
    ensure_terminal_instruction(cur)?; // check end
    // finish orr
    Ok((instable, jumpdest))
}

//
fn verify_jump_dests(instable: &[u8], jumpdests: &[isize]) -> VmrtErr {
    let itlen = instable.len();
    let right = itlen as isize - 1;
    for jp in jumpdests {
        let j = *jp;
        if j < 0 || j > right {
            return itr_err_code!(JumpOverflow);
        }
        if 0 == instable[j as usize] {
            return itr_err_code!(JumpInDataSeg);
        }
    }
    // finish
    Ok(())
}

#[derive(Debug, Copy, Clone)]
struct LinearStackState {
    min_depth: i32,
    max_depth: i32,
    top_const_u16: Option<u16>,
}

impl LinearStackState {
    fn new(entry_stack: VerifyEntryStack) -> Self {
        let (min_depth, max_depth) = match entry_stack {
            VerifyEntryStack::Empty => (0, 0),
            VerifyEntryStack::Argv => (1, 1),
            VerifyEntryStack::OptionalArgv => (0, 1),
        };
        Self {
            min_depth,
            max_depth,
            top_const_u16: None,
        }
    }

    fn push_unknown(&mut self) {
        self.min_depth += 1;
        self.max_depth += 1;
        self.top_const_u16 = None;
    }

    fn push_const(&mut self, value: u16) {
        self.min_depth += 1;
        self.max_depth += 1;
        self.top_const_u16 = Some(value);
    }

    fn apply_effect(&mut self, inst: Bytecode, input: i32, output: i32) -> VmrtErr {
        if self.max_depth < input {
            return stack_underflow_err(inst, input, self.max_depth);
        }
        let surviving_min = if self.min_depth < input {
            input
        } else {
            self.min_depth
        };
        self.min_depth = surviving_min - input + output;
        self.max_depth = self.max_depth - input + output;
        self.top_const_u16 = None;
        Ok(())
    }
}

fn stack_underflow_err(inst: Bytecode, input: i32, available: i32) -> VmrtErr {
    itr_err_fmt!(
        StackError,
        "bytecode {:?} requires {} stack values but only {} available",
        inst,
        input,
        available
    )
}

fn read_u8_param(codes: &[u8], pc: usize) -> VmrtRes<u8> {
    let Some(v) = codes.get(pc + 1).copied() else {
        return itr_err_code!(InstParamsErr);
    };
    Ok(v)
}

fn instruction_end(codes: &[u8], pc: usize, inst: Bytecode) -> VmrtRes<usize> {
    let meta = inst.metadata();
    let pend = match inst {
        PBUF => {
            let len = read_u8_param(codes, pc)? as usize;
            pc + 2 + len
        }
        PBUFL => {
            let start = pc + 1;
            let end = start + 2;
            if end > codes.len() {
                return itr_err_code!(InstParamsErr);
            }
            let len = u16::from_be_bytes(codes[start..end].try_into().unwrap()) as usize;
            end + len
        }
        _ => pc + 1 + meta.param as usize,
    };
    if pend > codes.len() {
        return itr_err_code!(InstParamsErr);
    }
    Ok(pend)
}

fn is_external_stack_boundary(inst: Bytecode) -> bool {
    matches!(
        inst,
        ACTION
            | ACTENV
            | ACTVIEW
            | NTENV
            | NTCTL
            | NTFUNC
            | CODECALL
            | CALL
            | CALLEXT
            | CALLEXTVIEW
            | CALLUSEVIEW
            | CALLUSEPURE
            | CALLTHIS
            | CALLSELF
            | CALLSUPER
            | CALLSELFVIEW
            | CALLSELFPURE
    )
}

/// Conservative straight-line stack-depth verifier.
///
/// This catches definite underflow before the first control-flow/call boundary.
/// For dynamic arity that comes from the stack, it only simulates precisely when
/// the arity is a small literal already visible on the stack. Otherwise it stops
/// rather than guessing and risking false positives.
fn verify_linear_stack_effects(codes: &[u8], entry_stack: VerifyEntryStack) -> VmrtErr {
    let mut state = LinearStackState::new(entry_stack);
    let mut pc = 0usize;

    while pc < codes.len() {
        let inst = Bytecode::try_from_u8(codes[pc])?;
        let meta = inst.metadata();
        let next = instruction_end(codes, pc, inst)?;

        match inst {
            P0 => state.push_const(0),
            P1 => state.push_const(1),
            P2 => state.push_const(2),
            P3 => state.push_const(3),
            PU8 => state.push_const(read_u8_param(codes, pc)? as u16),
            PU16 => {
                let start = pc + 1;
                let end = start + 2;
                if end > codes.len() {
                    return itr_err_code!(InstParamsErr);
                }
                state.push_const(u16::from_be_bytes(codes[start..end].try_into().unwrap()));
            }
            PBUF | PBUFL | PNIL | PNBUF | PTRUE | PFALSE | NEWLIST | NEWMAP | GET | GET0 | GET1
            | GET2 | GET3 | HREADUL | HREADU => {
                state.push_unknown();
            }
            DUP => {
                let top = state.top_const_u16;
                state.apply_effect(inst, 1, 2)?;
                state.top_const_u16 = top;
            }
            DUPN => {
                let n = read_u8_param(codes, pc)? as i32;
                if n < 2 {
                    return itr_err_fmt!(StackError, "inst dupn param must be at least 2");
                }
                let top = state.top_const_u16;
                state.apply_effect(inst, n, n * 2)?;
                state.top_const_u16 = top;
            }
            POP => state.apply_effect(inst, 1, 0)?,
            POPN => state.apply_effect(inst, read_u8_param(codes, pc)? as i32, 0)?,
            ROLL0 => state.apply_effect(inst, 1, 1)?,
            ROLL => state.apply_effect(
                inst,
                read_u8_param(codes, pc)? as i32 + 1,
                read_u8_param(codes, pc)? as i32 + 1,
            )?,
            REV => {
                let n = read_u8_param(codes, pc)? as i32;
                if n < 2 {
                    return itr_err_fmt!(StackError, "inst reverse param must be at least 2");
                }
                state.apply_effect(inst, n, n)?;
            }
            JOIN => {
                let n = read_u8_param(codes, pc)? as i32;
                if n < 3 {
                    return itr_err_fmt!(StackError, "inst join param must be at least 3");
                }
                state.apply_effect(inst, n, 1)?;
            }
            PACKLIST | PACKMAP | PACKTUPLE => {
                let Some(arity) = state.top_const_u16 else {
                    state.apply_effect(inst, 1, 1)?;
                    return Ok(());
                };
                if arity == 0 {
                    return itr_err_code!(CompoPackError);
                }
                state.apply_effect(inst, arity as i32 + 1, 1)?;
            }
            _ if is_external_stack_boundary(inst) || matches!(inst, JMPL | JMPS | JMPSL) => {
                return Ok(());
            }
            BRL | BRS | BRSL | BRSLN => {
                state.apply_effect(inst, 1, 0)?;
                return Ok(());
            }
            _ => {
                if meta.input == 255 || meta.output == 255 {
                    return Ok(());
                }
                state.apply_effect(inst, meta.input as i32, meta.output as i32)?;
            }
        }

        pc = next;
    }

    Ok(())
}

/// Lightweight literal FIN stack sanity check.
///
/// Runtime frames may legitimately start with an argv value on the operand
/// stack, and bytecode can use branches, packed containers, calls, locals, and
/// other dynamic-stack operations. A general verifier here would either become
/// a real abstract interpreter or reject valid contracts. Keep this deliberately
/// narrow: only catch the most obvious FIN underflow in a literal-only prefix.
fn verify_literal_fin_stack_prefix(codes: &[u8]) -> VmrtErr {
    const ENTRY_STACK_ALLOWANCE: i32 = 1;
    let mut depth: i32 = ENTRY_STACK_ALLOWANCE;
    let mut i = 0usize;
    while i < codes.len() {
        let inst = Bytecode::try_from_u8(codes[i])?;
        let meta = inst.metadata();
        i += 1;

        let pend = match inst {
            PBUF => {
                if i >= codes.len() {
                    return itr_err_code!(InstParamsErr);
                }
                i + 1 + codes[i] as usize
            }
            PBUFL => {
                if i + 2 > codes.len() {
                    return itr_err_code!(InstParamsErr);
                }
                let len = u16::from_be_bytes(codes[i..i + 2].try_into().unwrap()) as usize;
                i + 2 + len
            }
            _ => i + meta.param as usize,
        };
        if pend > codes.len() {
            return itr_err_code!(InstParamsErr);
        }

        if matches!(
            inst,
            PU8 | PU16 | PBUF | PBUFL | P0 | P1 | P2 | P3 | PNIL | PNBUF | PTRUE | PFALSE
        ) {
            depth += 1;
            i = pend;
            continue;
        }

        if is_fin_family(inst) {
            let input = meta.input as i32;
            if depth < input {
                return itr_err_fmt!(
                    StackError,
                    "bytecode {:?} requires {} stack values but only {} available",
                    inst,
                    input,
                    depth
                );
            }
        }
        break;
    }
    Ok(())
}
