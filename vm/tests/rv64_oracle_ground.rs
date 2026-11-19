//! C1 oracle ground test: the fitsh consensus interpreter (the rust HVM
//! bytecode engine) against the fitsh→RV64IM backend — same source, same
//! input, in all three lowering modes.
//!
//! The rv64 suites (`rv64_differential`, `rv64_guard_semantics`) knock the
//! backend against HAND-WRITTEN rust models. This suite replaces the
//! hand-written model with the consensus engine itself — the interpreter
//! that executes the very same `compile_body` bytecode the HVM channel
//! deploys — so a backend bug can no longer hide behind a transcription
//! mistake in the model.
//!
//! Surfaces compared:
//! * interpreter: `(Ok(output bytes) | Err(class))` — the HVM error class is
//!   the `ItrErrCode` embedded in the fault message (`Arithmetic(90): …`),
//!   surfaced through `vm::main_call` → `NativeVm`.
//! * rv64: `(Ok(output bytes) | trap exit code)` — the guest HALT exit code
//!   (`90`/`71`/user `require`/`throw` value).
//!
//! Engine-path notes (this file is the C1 record of how to drive both):
//! * interpreter: `compile_body(.., is_ircode=false)` → `Bytecode`; the
//!   `data` argument is fed as the call argv by prepending a `PBUF/PBUFL`
//!   push in front of the body bytecode (the body's param prelude
//!   `PUT 0 (ROLL0)` pops it into slot 0 — `lang/syntax/context.rs
//!   build_param_prelude`).
//! * rv64: `compile_body` IR → `lower_body_mode_arena` →
//!   `pack_segments_profile` (in=4800, out=16, mem=64KiB) → `CalcFnObj`.
//!
//! Documented engine divergences (pinned here as tests, NOT failures):
//! * `as bytes` on a non-u64: interpreter is width-aware (u8→1B, u16→2B,
//!   u64→8B BE; `value/item.rs scalar_bytes`) vs rv64 always the 8-byte BE
//!   u64 view (rv64.rs "Documented deviations").
//! * byte/buf bounds: interpreter `StackError(42)` (+ `CastParamFail(72)`
//!   for indices/offsets ≥ 2¹⁶, `extract_u16` narrowing) vs rv64
//!   `CastFail(71)` on the full u64.
//! * shift ≥ width: interpreter `Arithmetic(90)` (checked_shl), rv64 masks
//!   the amount to 63.
//! * narrow-width arithmetic: the interpreter computes in the operands'
//!   common width (`arithmetic_args2` → `cast_same_uint_width2`), rv64 is
//!   u64-uniform — `1 << byte` overflows u8 on the interpreter and succeeds
//!   on rv64.
//! * bytes churn bounds: interpreter `OutOfValueSize(39)` at value_size
//!   1280 B vs rv64 `CastFail(71)` at the 12 KiB-arena bump (churn bound).
//! * expression depth 9: rv64-only compile rejection (shared front end
//!   accepts; the interpreter runs the bytecode).
//!
//! Everything else must agree BYTE-FOR-BYTE (outputs) and CLASS-FOR-CLASS
//! (90/71 traps and the require/throw user exit value), including the full
//! score_sum and hacd200 kernels transcribed from hacdtestnet
//! `src/tests/calc_conformance/fitsh_kernels.rs`.
//!
//! Status (P0-C task C1, 2026-11-13): the parked draft was verified as-is —
//! all of its engine-path claims checked against the source (main_call /
//! ItrErrCode / cutbyte / extract_u16 / bit_shl / emit_slice RDrop) and all
//! tests pass at HEAD 108191a. Rework this round: Nil-return equivalence for
//! the END path, PBUFL (>255 B) input plumbing pin, assert/abort mapping rows
//! (interpreter ThrowAbort(151) vs rv64 exit 1), u128 compile-rejection pin,
//! and the g7b fixed-overhead/loop cost probe. Full record:
//! docs.loc/rv64im/30-c1-interpreter-oracle.md.
//! Update (P1-F, BUG-009 fix, 2026-11-13): `g5_buf_right_drop_pointer_bug_pinned`
//! flipped from pinning the observed buggy behavior to pinning the CORRECT
//! RDrop semantics (three modes, content compared); `g3_bytes_family_agree`
//! knocks `buf_right_drop` content again (it was excluded while the bug was
//! live). Record: docs.loc/rv64im/11-p0f-backend-bugfixes.md.

use std::sync::Arc;

use base::{
    ActionRef, BinaryCodecs, BlockRef, Context, Env, ExecFrom, ExecRegistry, ExecutionServices,
    JsonCodecs, LogEntry, P2sh, StateChunkRef, StateLayer, StateRead, TexLedger, Transaction,
    TxRef, Vm, VmExecutionParams, VmHostActionDef, VmHostCallKind,
};
use field::{Address, Amount, Encode, Hash};
use sys::{errf, Rerr, Ret};

use vm::fitshc::{compile_body, CompiledCode};
use vm::ir::rv64::{lower_body_mode_arena, ARENA_SIZE_MAX, Rv64Mode};
use vm::lang::Tokenizer;
use vm::{CodeType, Value, ValueTy};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/* ================================================================== */
/* Interpreter side: fitsh body → bytecode → NativeVm (main_call)      */
/* ================================================================== */

/// Registry capturing the `vm::register_exec` VM assigner (the only public
/// constructor of the crate-private `NativeVm`).
struct OracleRegistry {
    vm_assigner: Option<base::VmAssignFn>,
}

impl ExecRegistry for OracleRegistry {
    fn set_block_creator(&mut self, _f: base::BlockCreateFn) -> Rerr {
        Ok(())
    }
    fn set_vm_assigner(&mut self, f: base::VmAssignFn) -> Rerr {
        self.vm_assigner = Some(f);
        Ok(())
    }
    fn register_vm_host_def(&mut self, _def: VmHostActionDef) -> Rerr {
        Ok(())
    }
    fn set_context_creator(&mut self, _f: base::ContextCreateFn) -> Rerr {
        Ok(())
    }
    fn set_vm_params(&mut self, _params: VmExecutionParams) -> Rerr {
        Ok(())
    }
    fn set_execution_profile(&mut self, _profile: &'static dyn base::ExecutionProfile) -> Rerr {
        Ok(())
    }
}

/// The assigner installed by `vm::register_exec` (builds a `NativeVm`).
fn oracle_vm_assigner() -> &'static base::VmAssignFn {
    static ASSIGNER: std::sync::OnceLock<base::VmAssignFn> = std::sync::OnceLock::new();
    ASSIGNER.get_or_init(|| {
        let mut reg = OracleRegistry { vm_assigner: None };
        vm::register_exec(&mut reg).expect("register vm exec");
        reg.vm_assigner
            .expect("register_exec installs the vm assigner")
    })
}

/// Stub VM params mirroring `machine::test_ctx::STUB_VM_PARAMS` (that type
/// is `pub(crate)` + `cfg(test)`, so this integration test rebuilds it).
static ORACLE_VM_PARAMS: VmExecutionParams = VmExecutionParams {
    contract_store_perm_periods: 10_000,
    contract_storage_fee: base::ContractStorageFeeParams::disabled(),
    initial_fee_purity_floor: 50_000_000_000,
    fee_purity_reductions: &[],
    gas_budget_lookup: &base::GAS_BUDGET_LOOKUP_NONE,
    tx_gas_budget_cap_byte: 0,
    compute_limit_byte: 0,
    resource_limit_byte: 0,
    storage_limit_byte: 0,
};

/// In-memory KV layer backing the oracle context.
#[derive(Default)]
struct MemLayer(std::collections::HashMap<Vec<u8>, Vec<u8>>);

impl StateRead for MemLayer {
    fn get(&self, key: &[u8]) -> Ret<Option<Vec<u8>>> {
        Ok(self.0.get(key).cloned())
    }
}

impl StateLayer for MemLayer {
    fn set(&mut self, key: &[u8], val: Vec<u8>) {
        self.0.insert(key.to_vec(), val);
    }
    fn del(&mut self, key: &[u8]) {
        self.0.remove(key);
    }
}

/// Registry-less services stub: enough for bytecode execution that touches
/// no host actions (the kernel subset cannot).
struct OracleServices;

impl BinaryCodecs for OracleServices {
    fn decode_action(&self, _buf: &[u8]) -> Ret<(ActionRef, usize)> {
        errf!("oracle services: decode_action")
    }
    fn decode_transaction(&self, _buf: &[u8]) -> Ret<(TxRef, usize)> {
        errf!("oracle services: decode_transaction")
    }
    fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
        errf!("oracle services: decode_block")
    }
    fn peek_block_size(&self, _buf: &[u8]) -> Ret<usize> {
        errf!("oracle services: peek_block_size")
    }
    fn block_hash(&self, _height: u64, _stuff: &[u8]) -> [u8; base::HASH_SIZE] {
        [0u8; base::HASH_SIZE]
    }
    fn block_hasher_fn(&self) -> base::BlockHasherFn {
        |_height, _stuff| [0u8; base::HASH_SIZE]
    }
}

impl JsonCodecs for OracleServices {
    fn decode_action_json(&self, _json: &str) -> Ret<ActionRef> {
        errf!("oracle services: decode_action_json")
    }
}

impl ExecutionServices for OracleServices {
    fn assign_vm(&self, height: u64) -> Option<Box<dyn Vm>> {
        Some(oracle_vm_assigner()(self, height))
    }
    fn vm_host_def(&self, _kind: VmHostCallKind, _id: u8) -> Option<&VmHostActionDef> {
        None
    }
    fn vm_params(&self) -> Ret<&VmExecutionParams> {
        Ok(&ORACLE_VM_PARAMS)
    }
    fn execution_profile(&self) -> Ret<&'static dyn base::ExecutionProfile> {
        errf!("oracle services: execution_profile")
    }
    fn create_context(
        self: Arc<Self>,
        _env: Env,
        _chunk: StateChunkRef,
        _tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        errf!("oracle services: create_context")
    }
}

/// Type-3 transaction stub carrying only a main address.
#[derive(Debug)]
struct OracleTx(Address);

impl Encode for OracleTx {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}

impl Transaction for OracleTx {
    fn ty(&self) -> u8 {
        3
    }
    fn main(&self) -> Address {
        self.0
    }
    fn fee(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl base::TransactionSign for OracleTx {
    fn hash(&self) -> Hash {
        Hash::default()
    }
    fn verify_signature(&self) -> Rerr {
        Ok(())
    }
    fn as_execute(&self) -> Option<&dyn base::TransactionExecute> {
        Some(self)
    }
}

impl base::TransactionExecute for OracleTx {
    fn execute(&self, _ctx: &mut dyn Context) -> Rerr {
        errf!("oracle tx: execute")
    }
}

/// Minimal consensus context: fresh in-memory layer, huge fixed gas budget.
struct OracleCtx {
    env: Env,
    tx: OracleTx,
    layer: MemLayer,
    exec_from: ExecFrom,
    tex: TexLedger,
    gas: i64,
    vm: Option<Box<dyn Vm>>,
    vm_active: bool,
}

impl OracleCtx {
    /// Bounded-gas constructor: the throttle lever for generator campaigns
    /// (C2/C4). A runaway generated program must fail as OutOfGas instead of
    /// hanging the harness — the gas charge fires per instruction, so the
    /// bound is deterministic. `Interp::run` uses `1 << 40` (effectively
    /// unbounded) via `run_bounded`.
    fn with_gas(gas: i64) -> Self {
        let mut env = Env::default();
        env.tx.ty = 3;
        let tx = OracleTx(Address::default());
        env.tx.main = tx.0;
        Self {
            env,
            tx,
            layer: MemLayer::default(),
            exec_from: ExecFrom::Top,
            tex: TexLedger::default(),
            gas,
            vm: Some(OracleServices.assign_vm(1).expect("assign oracle vm")),
            vm_active: false,
        }
    }
}

impl Context for OracleCtx {
    fn services(&self) -> Arc<dyn ExecutionServices> {
        Arc::new(OracleServices)
    }
    fn env(&self) -> &Env {
        &self.env
    }
    fn tx(&self) -> &dyn Transaction {
        &self.tx
    }
    fn exec_from(&self) -> ExecFrom {
        self.exec_from
    }
    fn exec_from_set(&mut self, from: ExecFrom) {
        self.exec_from = from;
    }
    fn check_sign(&mut self, _adr: &Address) -> Rerr {
        Ok(())
    }
    fn layer(&mut self) -> &mut dyn StateLayer {
        &mut self.layer
    }
    fn emit_log(&mut self, _entry: LogEntry) {}
    fn gas_remaining(&self) -> i64 {
        self.gas
    }
    fn gas_charge(&mut self, gas: i64) -> Rerr {
        self.gas -= gas;
        if self.gas < 0 {
            return errf!("oracle ctx out of gas");
        }
        Ok(())
    }
    fn gas_rebate(&mut self, gas: i64) -> Rerr {
        self.gas += gas;
        Ok(())
    }
    fn gas_initialize(&mut self, _budget: i64) -> Rerr {
        Ok(())
    }
    fn gas_refund(&mut self) -> Rerr {
        Ok(())
    }
    fn snapshot_volatile(&self) -> Box<dyn std::any::Any> {
        Box::new(())
    }
    fn restore_volatile(&mut self, _snap: Box<dyn std::any::Any>) {}
    fn action_call(&mut self, _kind: u16, _body: Vec<u8>) -> Ret<base::ActOut> {
        errf!("oracle ctx: action_call")
    }
    fn vm_take(&mut self) -> Option<Box<dyn Vm>> {
        if self.vm_active {
            return None;
        }
        self.vm_active = true;
        self.vm.take()
    }
    fn vm_put(&mut self, vm: Box<dyn Vm>) {
        self.vm = Some(vm);
        self.vm_active = false;
    }
    fn vm_peek(&mut self) -> Option<&mut (dyn Vm + 'static)> {
        if self.vm_active {
            return None;
        }
        self.vm.as_deref_mut()
    }
    fn as_context_mut(&mut self) -> &mut dyn Context {
        self
    }
    fn tex_ledger(&self) -> &TexLedger {
        &self.tex
    }
    fn p2sh_set(&mut self, _addr: Address, _p2sh: Box<dyn P2sh>) -> Rerr {
        Ok(())
    }
}

/// Interpreter-side error: the HVM class parsed off the fault message
/// (`ItrErr` → `sys::Error` keeps the `Class(N): tip` prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
struct InterpFault {
    class: String,
    code: u16,
    msg: String,
}

impl InterpFault {
    fn contains(&self, pat: &str) -> bool {
        self.msg.contains(pat)
    }
}

fn parse_fault(e: &sys::Error) -> InterpFault {
    let msg = e.as_str().to_string();
    // `ItrErr` Display: `Class(N): tip` (rt/error.rs `From<ItrErr> for Error`)
    if let Some(open) = msg.find('(') {
        let rest = &msg[open + 1..];
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let class = msg[..open].to_string();
            let code = rest[..digits].parse().unwrap_or(0);
            return InterpFault { class, code, msg };
        }
    }
    InterpFault {
        class: String::new(),
        code: 0,
        msg,
    }
}

/// One compiled interpreter program: body bytecode with the input push
/// prelude, ready for repeated `main_call` runs.
struct Interp {
    codes: Arc<[u8]>,
}

impl Interp {
    /// Compile a kernel body (declared `data: bytes`) to HVM bytecode.
    fn build(body: &str) -> Self {
        let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
        let (_, compiled, _) = compile_body(
            tokens,
            vec![("data".to_string(), ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .expect("compile body for the interpreter channel");
        let CompiledCode::Bytecode(bts) = compiled else {
            panic!("expected bytecode compilation")
        };
        Self { codes: bts.into() }
    }

    /// Run one input: push bytes prelude (the `data` argv) + body bytecode,
    /// through the real consensus engine (`vm::main_call` → NativeVm).
    fn run(&self, input: &[u8]) -> Result<Vec<u8>, InterpFault> {
        self.run_bounded(input, 1 << 40)
    }

    /// Same run with a finite gas budget — the throttle lever for generator
    /// campaigns (C2/C4): a runaway loop dies as an out-of-gas fault instead
    /// of hanging the harness. The charge fires per instruction, so the bound
    /// is deterministic (same program + input → same instruction count).
    fn run_bounded(&self, input: &[u8], gas: i64) -> Result<Vec<u8>, InterpFault> {
        let mut codes = Vec::with_capacity(input.len() + 8);
        if input.len() <= u8::MAX as usize {
            codes.push(vm::Bytecode::PBUF as u8);
            codes.push(input.len() as u8);
        } else {
            codes.push(vm::Bytecode::PBUFL as u8);
            codes.extend_from_slice(&(input.len() as u16).to_be_bytes());
        }
        codes.extend_from_slice(input);
        codes.extend_from_slice(&self.codes);
        let mut ctx = OracleCtx::with_gas(gas);
        let (_, value) = vm::main_call(&mut ctx, CodeType::Bytecode, codes.into())
            .map_err(|e| parse_fault(&e))?;
        match value {
            Value::Bytes(buf) => Ok(buf),
            // `end` / fall-off-the-end: the interpreter's boundary value is
            // Nil; the rv64 epilogue emits a zero-length output. Same
            // consensus-observable result: empty bytes.
            Value::Nil => Ok(vec![]),
            other => panic!("interpreter returned non-bytes value: {other:?}"),
        }
    }
}

/* ================================================================== */
/* rv64 side: same body → three lowering modes → calc containers       */
/* ================================================================== */

struct Rv64 {
    engines: Vec<(Rv64Mode, vm::calc::CalcFnObj)>,
}

impl Rv64 {
    fn build(body: &str) -> Self {
        let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
        let (ir, compiled, _) = compile_body(
            tokens,
            vec![("data".to_string(), ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .expect("compile body");
        assert!(
            matches!(compiled, CompiledCode::Bytecode(_)),
            "both channels compile the same body"
        );
        let cap = vm::calc::SpaceCap::new(0);
        let engines = MODES
            .iter()
            .map(|&mode| {
                let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).expect("lower");
                let profile = match mode {
                    Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
                    _ => vm::calc::container::CALC_PROFILE_RV64IMC,
                };
                let container = lo
                    .pack_segments_profile(&cap, 64, 4800, 16, profile)
                    .expect("pack");
                let fno =
                    vm::calc::CalcFnObj::build(container, &cap, vm::calc::CalcHostConfig::new())
                        .expect("verify+build");
                (mode, fno)
            })
            .collect();
        Self { engines }
    }

    fn run(&self, input: &[u8]) -> Vec<(Rv64Mode, Result<Vec<u8>, u32>)> {
        self.run_with_limits(
            input,
            vm::calc::CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
        )
    }

    /// Same run with explicit limits — the rv64-side throttle lever (cycle
    /// budget) for generator campaigns. Budget exhaustion is `Err` with exit
    /// code 0 (`CalcAbort::OutOfGas` carries no guest code, calc/mod.rs).
    fn run_with_limits(
        &self,
        input: &[u8],
        limits: vm::calc::CalcRunLimits,
    ) -> Vec<(Rv64Mode, Result<Vec<u8>, u32>)> {
        self.engines
            .iter()
            .map(|&(mode, ref fno)| {
                let ex = fno.run(input, limits);
                (mode, ex.result.map_err(|_| ex.exit_code as u32))
            })
            .collect()
    }
}

/* ================================================================== */
/* Knock helpers                                                       */
/* ================================================================== */

/// HVM error classes the oracle compares (rt/error.rs `ItrErrCode`).
const ARITH: u16 = 90; // Arithmetic
const CAST: u16 = 71; // CastFail
const STACK: u16 = 42; // StackError
const CAST_PARAM: u16 = 72; // CastParamFail
const OUT_OF_VALUE: u16 = 39; // OutOfValueSize
const USER_ABORT: u16 = 150;
const THROW_ABORT: u16 = 151;

/// Both engines must produce `want` byte-for-byte (all three rv64 modes).
fn knock_ok(rv: &Rv64, it: &Interp, input: &[u8], want: &[u8]) {
    let got = it.run(input);
    assert_eq!(got, Ok(want.to_vec()), "interpreter diverged for input {input:?}");
    for (mode, got) in rv.run(input) {
        assert_eq!(got, Ok(want.to_vec()), "mode {mode:?} diverged for input {input:?}");
    }
}

/// Both engines must fail with the SAME class, where the rv64 exit code is
/// the class number (90/71 only — user aborts carry the user value instead).
fn knock_trap(rv: &Rv64, it: &Interp, input: &[u8], exit: u32) {
    let err = it.run(input).expect_err("interpreter must fault");
    assert_eq!(err.code, exit as u16, "interpreter fault class mismatch: {}", err.msg);
    for (mode, got) in rv.run(input) {
        assert_eq!(got, Err(exit), "mode {mode:?} exit mismatch for input {input:?}");
    }
}

/// Documented engine divergence, pinned: interpreter fault `(code,
/// msg-contains)` vs rv64 result.
fn pin_divergence(
    rv: &Rv64,
    it: &Interp,
    input: &[u8],
    interp_want: (u16, &str),
    rv64_want: Result<Vec<u8>, u32>,
) {
    let (code, pat) = interp_want;
    let err = it.run(input).expect_err("interpreter must fault");
    assert_eq!(err.code, code, "interpreter class: {}", err.msg);
    assert!(err.contains(pat), "interpreter message {:?} must contain {pat:?}", err.msg);
    for (mode, got) in rv.run(input) {
        assert_eq!(got, rv64_want, "mode {mode:?} pinned-divergence mismatch for {input:?}");
    }
}

/// 8-byte BE → u64 helper for building inputs.
fn be(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// Two 8-byte BE input words.
fn words(a: u64, b: u64) -> Vec<u8> {
    let mut v = be(a);
    v.extend(be(b));
    v
}

/// Load `a = be8(0..8)` / `b = be8(8..16)`, compute one u64 expression.
fn ab_body(expr: &str) -> String {
    format!(
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 {{ a = a * 256 + byte(data, i)\ni += 1 }}\n\
         var b = 0u64\nvar j = 0u64\nwhile j < 8 {{ b = b * 256 + byte(data, j + 8)\nj += 1 }}\n\
         var r = {expr}\nreturn r as bytes"
    )
}

/// Load one u64 `v = be8(0..8)` and compute one u64 expression over it.
fn v_body(expr: &str) -> String {
    format!(
        "var v = 0u64\nvar i = 0u64\nwhile i < 8 {{ v = v * 256 + byte(data, i)\ni += 1 }}\n\
         var r = {expr}\nreturn r as bytes"
    )
}

/* ================================================================== */
/* G0: oracle plumbing                                                 */
/* ================================================================== */

#[test]
fn g0_oracle_plumbing_constant_and_wire_format() {
    // `as bytes` on a u64 is the 8-byte BE view on both engines.
    let body = "var r = 1234567890u64\nreturn r as bytes";
    knock_ok(&Rv64::build(body), &Interp::build(body), &[0u8; 4], &be(1234567890));
    // empty input flows through both engines (data param bound, unused)
    let body2 = "var r = 7u64\nreturn r as bytes";
    knock_ok(&Rv64::build(body2), &Interp::build(body2), &[], &be(7));
    // a bare bytes return (no cast), 12 B ≤ the 16 B rv64 output_max
    let body3 = "var r = data\nreturn r";
    let input: Vec<u8> = (0u8..12).collect();
    knock_ok(&Rv64::build(body3), &Interp::build(body3), &input, &input);
    // input > 255 B takes the PBUFL argv push (2-byte length); both engines
    // agree on a size-report echo (output stays ≤ the 16 B rv64 output_max).
    let big: Vec<u8> = (0..400).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
    let body4 = "var r = (size(data) as u64)\nreturn r as bytes";
    knock_ok(&Rv64::build(body4), &Interp::build(body4), &big, &be(400));
    // body falling off the end: empty output on both engines (END)
    let body5 = "var x = byte(data, 0)\n";
    knock_ok(&Rv64::build(body5), &Interp::build(body5), &[3u8], &[]);
}

/* ================================================================== */
/* G1: arithmetic — agreement matrices                                 */
/* ================================================================== */

#[test]
fn g1_add_sub_mul_agree_with_checked_u64() {
    let edges = [0u64, 1, u64::MAX / 2, u64::MAX - 1, u64::MAX];
    for op in ["+", "-", "*"] {
        let body = ab_body(&format!("(a {op} b)"));
        let rv = Rv64::build(&body);
        let it = Interp::build(&body);
        for a in edges {
            for b in edges {
                let model = match op {
                    "+" => a.checked_add(b),
                    "-" => a.checked_sub(b),
                    _ => a.checked_mul(b),
                };
                let mut input = be(a);
                input.extend(be(b));
                match model {
                    Some(v) => knock_ok(&rv, &it, &input, &be(v)),
                    None => knock_trap(&rv, &it, &input, ARITH as u32),
                }
            }
        }
    }
}

#[test]
fn g1_div_mod_pow_agree() {
    let pairs = [
        (7u64, 3u64),
        (7, 0),
        (0, 0),
        (u64::MAX, 1),
        (u64::MAX, 0),
        (100, 10),
    ];
    for (op, model) in [
        ("/", Box::new(|a: u64, b: u64| a.checked_div(b)) as Box<dyn Fn(u64, u64) -> Option<u64>>),
        ("%", Box::new(|a: u64, b: u64| a.checked_rem(b)) as Box<dyn Fn(u64, u64) -> Option<u64>>),
    ] {
        let body = ab_body(&format!("(a {op} b)"));
        let rv = Rv64::build(&body);
        let it = Interp::build(&body);
        for (a, b) in pairs {
            let mut input = be(a);
            input.extend(be(b));
            match model(a, b) {
                Some(v) => knock_ok(&rv, &it, &input, &be(v)),
                None => knock_trap(&rv, &it, &input, ARITH as u32),
            }
        }
    }
    // pow with constant exponent unrolls to checked muls on both engines
    let body = v_body("(v ** 3)");
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for v in [0u64, 2, 2642245, 2642246, u64::MAX] {
        match v.checked_pow(3) {
            Some(p) => knock_ok(&rv, &it, &be(v), &be(p)),
            None => knock_trap(&rv, &it, &be(v), ARITH as u32),
        }
    }
}

#[test]
fn g1_max_min_clamp_bitwise_agree() {
    let pairs = [
        (0u64, 0),
        (0, 1),
        (1, 0),
        (7, 8),
        (u64::MAX, 0),
        (u64::MAX - 1, u64::MAX),
    ];
    let cases: Vec<(&str, Box<dyn Fn(u64, u64) -> u64>)> = vec![
        ("max(a, b)", Box::new(|a, b| a.max(b))),
        ("min(a, b)", Box::new(|a, b| a.min(b))),
        ("(a ^ b)", Box::new(|a, b| a ^ b)),
        ("(a | b)", Box::new(|a, b| a | b)),
        ("(a & b)", Box::new(|a, b| a & b)),
    ];
    for (src, model) in cases {
        let body = ab_body(src);
        let rv = Rv64::build(&body);
        let it = Interp::build(&body);
        for (a, b) in pairs {
            let mut input = be(a);
            input.extend(be(b));
            knock_ok(&rv, &it, &input, &be(model(a, b)));
        }
    }
    // shifts below the faulting width (≥ 64 is pinned separately in G5)
    let shift_cases: Vec<(&str, Box<dyn Fn(u64, u64) -> u64>)> = vec![
        ("(a << b)", Box::new(|a, b| a.wrapping_shl(b as u32))),
        ("(a >> b)", Box::new(|a, b| a.wrapping_shr(b as u32))),
    ];
    for (src, model) in shift_cases {
        let body = ab_body(src);
        let rv = Rv64::build(&body);
        let it = Interp::build(&body);
        for (a, b) in [(0u64, 0), (1, 1), (0xF0F0_F0F0_F0F0_F0F0, 4), (u64::MAX, 63)] {
            let mut input = be(a);
            input.extend(be(b));
            knock_ok(&rv, &it, &input, &be(model(a, b)));
        }
    }
    // clamp across all regions
    let body = ab_body("clamp(a, 10, 100)");
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for a in [0u64, 9, 10, 55, 100, 101, u64::MAX] {
        knock_ok(&rv, &it, &words(a, 0), &be(a.clamp(10, 100)));
    }
}

/* ================================================================== */
/* G2: trap classes that agree                                         */
/* ================================================================== */

#[test]
fn g2_narrowing_cast_overflow_traps_71_on_both() {
    for (ty, max) in [("u8", 255u64), ("u16", 65535), ("u32", 4_294_967_295)] {
        let body = v_body(&format!("(v as {ty})"));
        let rv = Rv64::build(&body);
        let it = Interp::build(&body);
        for v in [0u64, max - 1, max, max + 1, u64::MAX] {
            // the narrowing result is re-widened for the u64 wire view
            let body_wide = v_body(&format!("((v as {ty}) as u64)"));
            if v <= max {
                knock_ok(&Rv64::build(&body_wide), &Interp::build(&body_wide), &be(v), &be(v));
            } else {
                knock_trap(&rv, &it, &be(v), CAST as u32);
            }
        }
    }
}

#[test]
fn g2_div_zero_in_deep_expression_agree() {
    // divisor computed at nesting depth, from input
    let body = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\n\
                var q = 100 / (a - a + byte(data, 8))\nreturn q as bytes";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    let mut input = be(12345);
    input.push(0); // divisor = 0 → both engines trap 90
    knock_trap(&rv, &it, &input, ARITH as u32);
    input[8] = 5;
    knock_ok(&rv, &it, &input, &be(20));
}

#[test]
fn g2_require_throw_user_value_mapping() {
    // require failure: rv64 exit = the user error value; interpreter raises
    // UserAbort(150) with the same value in the message.
    let req = "require 42, byte(data, 0) < 10\nreturn (byte(data, 0) as u64) as bytes";
    let rv = Rv64::build(req);
    let it = Interp::build(req);
    for x in [0u8, 9] {
        knock_ok(&rv, &it, &[x], &be(x as u64));
    }
    let err = it.run(&[10]).expect_err("require must fail");
    assert_eq!(err.code, USER_ABORT, "class: {}", err.msg);
    assert!(err.contains("require failed"), "msg: {}", err.msg);
    assert!(err.contains("42"), "user value in msg: {}", err.msg);
    for (mode, got) in rv.run(&[10]) {
        assert_eq!(got, Err(42), "mode {mode:?}: user value is the exit code");
    }

    // require passing stays silent on both engines
    let req2 = "require 5, 1 < 2\nreturn 1u64 as bytes";
    knock_ok(&Rv64::build(req2), &Interp::build(req2), &[], &be(1));

    // throw: rv64 exit = the thrown value; interpreter ThrowAbort(151).
    let thr = "throw 7";
    let err = Interp::build(thr).run(&[0; 4]).expect_err("throw must abort");
    assert_eq!(err.code, THROW_ABORT, "class: {}", err.msg);
    assert!(err.contains("7"), "msg: {}", err.msg);
    for (mode, got) in Rv64::build(thr).run(&[0; 4]) {
        assert_eq!(got, Err(7), "mode {mode:?}");
    }

    // assert-fail: rv64 traps exit code 1 (`trap(1)`, rv64.rs AST); the
    // interpreter raises ThrowAbort(151) too — call.rs merges Abort|Throw
    // ("VM return failed: Nil"). The engines do NOT carry the same payload:
    // rv64 has no user value for assert, the class number is NOT the exit.
    let ast = "assert byte(data, 0) < 10\nreturn 1u64 as bytes";
    knock_ok(&Rv64::build(ast), &Interp::build(ast), &[3], &be(1));
    let err = Interp::build(ast).run(&[10]).expect_err("assert must abort");
    assert_eq!(err.code, THROW_ABORT, "class: {}", err.msg);
    assert!(err.contains("VM return failed"), "msg: {}", err.msg);
    for (mode, got) in Rv64::build(ast).run(&[10]) {
        assert_eq!(got, Err(1), "mode {mode:?}: assert traps exit code 1");
    }

    // bare `abort`: same pair — interpreter ThrowAbort(151) / rv64 exit 1.
    let abt = "if byte(data, 0) > 100 { abort }\nreturn 1u64 as bytes";
    knock_ok(&Rv64::build(abt), &Interp::build(abt), &[1], &be(1));
    let err = Interp::build(abt).run(&[200]).expect_err("abort must abort");
    assert_eq!(err.code, THROW_ABORT, "class: {}", err.msg);
    for (mode, got) in Rv64::build(abt).run(&[200]) {
        assert_eq!(got, Err(1), "mode {mode:?}: abort traps exit code 1");
    }
}

/* ================================================================== */
/* G3: bytes family — in-bounds agreement                              */
/* ================================================================== */

#[test]
fn g3_bytes_family_agree() {
    let data: Vec<u8> = (0xA0u8..0xB0).collect(); // 16 bytes
    let cases: Vec<(&str, Box<dyn Fn(&[u8]) -> Vec<u8>>)> = vec![
        ("buf_left(4, data)", Box::new(|d: &[u8]| d[..4].to_vec())),
        (
            "buf_right(4, data)",
            Box::new(|d: &[u8]| d[d.len() - 4..].to_vec()),
        ),
        ("buf_left_drop(3, data)", Box::new(|d: &[u8]| d[3..].to_vec())),
        // buf_right_drop was excluded while the RDrop pointer bug was live
        // (fixed, BUG-009): it is knocked again — the CONTENT comparison
        // here is what the length-only differential could never provide.
        // Outputs stay ≤ the harness's 16 B output window.
        ("buf_right_drop(5, data)", Box::new(|d: &[u8]| d[..d.len() - 5].to_vec())),
        ("buf_right_drop(1, data)", Box::new(|d: &[u8]| d[..d.len() - 1].to_vec())),
        (
            "buf_right_drop(13, data) ++ buf_right_drop(12, data)",
            Box::new(|d: &[u8]| {
                let mut v = d[..d.len() - 13].to_vec();
                v.extend_from_slice(&d[..d.len() - 12]);
                v
            }),
        ),
        ("buf_cut(data, 3, 7)", Box::new(|d: &[u8]| d[3..10].to_vec())),
        (
            "(buf_cut(data, 3, 7) ++ buf_left(2, data))",
            Box::new(|d: &[u8]| {
                let mut v = d[3..10].to_vec();
                v.extend_from_slice(&d[..2]);
                v
            }),
        ),
    ];
    for (src, model) in cases {
        let body = format!("var r = {src}\nreturn r");
        knock_ok(&Rv64::build(&body), &Interp::build(&body), &data, &model(&data));
    }
    // byte extraction re-widened to the u64 wire view agrees
    let body = "var r = (byte(data, 5) as u64)\nreturn r as bytes";
    knock_ok(&Rv64::build(body), &Interp::build(body), &data, &be(data[5] as u64));
    // size re-widened to the u64 wire view agrees
    let body = "var r = (size(data) as u64)\nreturn r as bytes";
    knock_ok(
        &Rv64::build(body),
        &Interp::build(body),
        &data,
        &be(data.len() as u64),
    );
    // chained bytes op
    let body = "var r = (byte(buf_cut(data, 3, 7), 6) as u64)\nreturn r as bytes";
    knock_ok(&Rv64::build(body), &Interp::build(body), &data, &be(data[9] as u64));
}

/* ================================================================== */
/* G4: control flow — agreement                                        */
/* ================================================================== */

#[test]
fn g4_nested_loops_break_continue_agree() {
    let body = r#"
var acc = 0u64
var i = 0u64
while i < 4 {
    var j = 0u64
    while j < 4 {
        if j % 2 == 0 { j += 1
            continue }
        var k = 0u64
        while k < 4 {
            if k == 3 { break }
            if (i + j + k) % 5 == 0 { acc += 100 } else { acc += 1 }
            k += 1
        }
        j += 1
    }
    i += 1
}
return acc as bytes
"#;
    let mut acc = 0u64;
    for i in 0..4u64 {
        for j in 0..4u64 {
            if j % 2 == 0 {
                continue;
            }
            for k in 0..4u64 {
                if k == 3 {
                    break;
                }
                acc += if (i + j + k) % 5 == 0 { 100 } else { 1 };
            }
        }
    }
    knock_ok(&Rv64::build(body), &Interp::build(body), &[0u8; 8], &be(acc));
}

#[test]
fn g4_if_else_chain_and_value_if_agree() {
    let body = r#"
var x = byte(data, 0)
var cls = 0u64
if x < 10 { cls = 1 } else {
    if x < 50 { cls = 2 } else {
        if x < 200 { cls = 3 } else { cls = 4 }
    }
}
var out = cls * 10
if x % 2 == 0 { out += 1 }
if x % 3 == 0 { out += 5 }
return (out as u64) as bytes
"#;
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for x in [0u8, 9, 10, 49, 50, 199, 200, 255] {
        let cls = match x {
            0..=9 => 1u64,
            10..=49 => 2,
            50..=199 => 3,
            _ => 4,
        };
        let mut out = cls * 10;
        if x % 2 == 0 {
            out += 1;
        }
        if x % 3 == 0 {
            out += 5;
        }
        knock_ok(&rv, &it, &[x], &be(out));
    }
    // value-if (CHOOSE)
    let body = ab_body("if a > b { a } else { b + 1000 }");
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for (a, b) in [(1u64, 2), (3, 2), (0, 0)] {
        knock_ok(&rv, &it, &words(a, b), &be(if a > b { a } else { b + 1000 }));
    }
    // const-select table (dense 16-arm)
    let mut dense = String::from("var v = byte(data, 0)\nvar t = 99u64\n");
    for (i, k) in (65u16..=80).enumerate() {
        dense.push_str(&format!("if v == {k} {{ t = {} }}\n", i + 1));
    }
    dense.push_str("return (t as u64) as bytes");
    let rv = Rv64::build(&dense);
    let it = Interp::build(&dense);
    for v in [0u8, 64, 65, 70, 80, 81, 255] {
        let want = match v {
            65..=80 => (v - 64) as u64,
            _ => 99,
        };
        knock_ok(&rv, &it, &[v], &be(want));
    }
}

/* ================================================================== */
/* G5: documented divergences, pinned                                  */
/* ================================================================== */

#[test]
fn g5_as_bytes_width_divergence_pinned() {
    // CLOSED by task-1 A2: `as bytes` follows the interpreter's
    // `scalar_bytes` width on every engine (u8→1B, u16→2B, u32→4B, u64→8B
    // BE). The rows this test pinned as divergences (rv64's uniform 8-byte
    // view) are agreement rows now; the u64-pinned form keeps the 8-byte
    // wire view.
    let data: Vec<u8> = (0xA0u8..0xB4).collect(); // 20 bytes
    // size → U16: 2-byte BE everywhere
    let body = "var r = size(data)\nreturn r as bytes";
    let it = Interp::build(body);
    assert_eq!(it.run(&data), Ok(vec![0x00, 0x14]));
    for (mode, got) in Rv64::build(body).run(&data) {
        assert_eq!(got, Ok(vec![0x00, 0x14]), "mode {mode:?}");
    }
    // byte → U8: 1-byte everywhere
    let body = "var r = byte(data, 5)\nreturn r as bytes";
    let it = Interp::build(body);
    assert_eq!(it.run(&data), Ok(vec![0xA5]));
    for (mode, got) in Rv64::build(body).run(&data) {
        assert_eq!(got, Ok(vec![0xA5]), "mode {mode:?}");
    }
    // the u64-pinned wire form: 8-byte BE everywhere
    let body = "var r = (size(data) as u64)\nreturn r as bytes";
    let it = Interp::build(body);
    assert_eq!(it.run(&data), Ok(be(20)));
    for (mode, got) in Rv64::build(body).run(&data) {
        assert_eq!(got, Ok(be(20)), "mode {mode:?}");
    }
}

#[test]
fn g5_byte_bounds_divergence_pinned_42_vs_71() {
    // in-bounds reads agree (after the u64 wire pin); out-of-bounds differs:
    // interpreter StackError(42) ("read buf byte overflow", value/operand.rs
    // cutbyte) vs rv64 CastFail(71) (rv64.rs emit_byte_at bgeu → fault_cast).
    let body = "var idx = 0u64\nvar i = 0u64\nwhile i < 8 { idx = idx * 256 + byte(data, i)\ni += 1 }\n\
                var v = byte(data, idx)\nreturn (v as u64) as bytes";
    let input24: Vec<u8> = (0u8..24).collect();
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for idx in [0u64, 8, 31] {
        let mut input = be(idx);
        input.extend(input24.clone());
        knock_ok(&rv, &it, &input, &be(input[idx as usize] as u64));
    }
    for idx in [32u64, 33] {
        let mut input = be(idx);
        input.extend(input24.clone());
        pin_divergence(&rv, &it, &input, (STACK, "read buf byte overflow"), Err(CAST as u32));
    }
    // idx = u64::MAX: the interpreter's u16 argv narrowing fires BEFORE the
    // bounds check (extract_u16 → CastParamFail(72)); rv64 bounds-checks the
    // full u64 → 71.
    let mut input = be(u64::MAX);
    input.extend(input24.clone());
    pin_divergence(
        &rv,
        &it,
        &input,
        (CAST_PARAM, "cannot cast param"),
        Err(CAST as u32),
    );
    // empty input at index 0
    let body = "var v = byte(data, 0)\nreturn (v as u64) as bytes";
    pin_divergence(
        &Rv64::build(body),
        &Interp::build(body),
        &[],
        (STACK, "read buf byte overflow"),
        Err(CAST as u32),
    );
    // index ≥ 2^16: the interpreter rejects the u16 argv narrowing first
    // (CastParamFail(72) via extract_u16), rv64 bounds-checks the full u64.
    let body = "var v = byte(data, 70000)\nreturn (v as u64) as bytes";
    pin_divergence(
        &Rv64::build(body),
        &Interp::build(body),
        &[7u8; 8],
        (CAST_PARAM, "cannot cast param"),
        Err(CAST as u32),
    );
}

#[test]
fn g5_slice_bounds_divergence_pinned() {
    // buf_cut with runtime ost/ln. Agreement in bounds; divergences pinned:
    // OOB → interpreter StackError(42) ("cutout buf overflow") vs rv64 71;
    // zero-length slice → the following byte(s,0) read: 42 vs 71;
    // ost ≥ 2^16 → interpreter CastParamFail(72) vs rv64 71.
    let body = "var ost = 0u64\nvar i = 0u64\nwhile i < 8 { ost = ost * 256 + byte(data, i)\ni += 1 }\n\
                var ln = 0u64\nvar j = 0u64\nwhile j < 8 { ln = ln * 256 + byte(data, j + 8)\nj += 1 }\n\
                var s = buf_cut(data, ost, ln)\nreturn (byte(s, 0) as u64) as bytes";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    let tail16: Vec<u8> = (10u8..26).collect();
    // in bounds: 16+16 = 32-byte input, slice 16..24
    let mut input = be(16);
    input.extend(be(8));
    input.extend(tail16.clone());
    knock_ok(&rv, &it, &input, &be(input[16] as u64));
    // empty slice: byte(s, 0) traps on both, class 42 vs 71
    let mut input = be(16);
    input.extend(be(0));
    input.extend(tail16.clone());
    pin_divergence(&rv, &it, &input, (STACK, "read buf byte overflow"), Err(CAST as u32));
    // off-end slice: cutout overflow 42 vs 71
    let mut input = be(25);
    input.extend(be(8));
    input.extend(tail16.clone());
    pin_divergence(&rv, &it, &input, (STACK, "cutout buf overflow"), Err(CAST as u32));
    // ost ≥ 2^16: interpreter u16 narrowing first (72) vs rv64 71
    let mut input = be(u64::MAX);
    input.extend(be(2));
    input.extend(tail16.clone());
    pin_divergence(&rv, &it, &input, (CAST_PARAM, "cannot cast param"), Err(CAST as u32));
}

#[test]
fn g5_shift_mask_divergence_pinned_90_vs_ok() {
    // `v << n` with n ≥ 64: interpreter checked_shl → Arithmetic(90).
    // CLOSED by task-1 A1: rv64's amount check (`srli 6; bne`) faults 90
    // at the same site — the pinned rv64-Ok row (masked `n & 63`) is now
    // an agreement row.
    let src = "var v = 0u64\nvar i = 0u64\nwhile i < 8 { v = v * 256 + byte(data, i)\ni += 1 }\n\
               var n = 0u64\nvar j = 0u64\nwhile j < 8 { n = n * 256 + byte(data, j + 8)\nj += 1 }\n\
               var r = (v << n)\nreturn r as bytes";
    let rv = Rv64::build(src);
    let it = Interp::build(src);
    for (v, n) in [(1u64, 63), (0xDEAD_BEEF, 7), (u64::MAX, 1)] {
        // amount < 64: both engines compute the truncating shift
        knock_ok(&rv, &it, &words(v, n), &be(v.wrapping_shl(n as u32)));
    }
    for (v, n) in [(1u64, 64), (1, 65), (0xFFFF_FFFF, 192)] {
        // amount ≥ 64: both engines trap Arithmetic 90
        knock_trap(&rv, &it, &words(v, n), ARITH as u32);
    }
}

#[test]
fn g5_narrow_width_arithmetic_divergence_pinned() {
    // The interpreter computes in the operands' common width: `1 << hv`
    // with both sides u8 faults for hv ≥ 8 (u8 checked_shl → Arithmetic 90).
    // This is the exact shape the raw hacdtestnet score_sum transcription
    // hits (`bitmap | (1 << hv)`). CLOSED by task-1 A3: rv64's narrow-width
    // shift (amount checked against w, result truncated to w bits) faults
    // at the same site — every row is an agreement now.
    let body = "var hv = byte(data, 0) / 2\nvar r = (1 << hv)\nreturn r as bytes";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    // hv < 8: both engines compute the u8-width result (1-byte view)
    for b in [0u8, 2, 4, 14] {
        knock_ok(&rv, &it, &[b], &[(1u8 << (b / 2))]);
    }
    // 8 ≤ hv < 64 and hv ≥ 64: both engines trap Arithmetic 90
    for b in [40u8, 16, 250] {
        knock_trap(&rv, &it, &[b], ARITH as u32);
    }
    // with the width pinned (`1u64`), both engines agree everywhere —
    // including the shared trap at shift ≥ 64
    let body = "var hv = byte(data, 0) / 2\nvar r = (1u64 << hv)\nreturn r as bytes";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for b in [0u8, 2, 16, 30, 62, 124] {
        let hv = (b / 2) as u32;
        knock_ok(&rv, &it, &[b], &be(1u64.wrapping_shl(hv & 63)));
    }
    // b = 250 → hv = 125 ≥ 64: both engines trap 90 (post-A1 agreement)
    knock_trap(&rv, &it, &[250], ARITH as u32);
}

#[test]
fn g5_pow_shape_divergences_pinned() {
    // POW engine-shape divergences:
    //
    // (a) narrow-width POW — CLOSED by task-1 A3: `2 ** b` with both
    //     operands u8 runs u8::checked_pow in the interpreter (2**8
    //     overflows u8 → Arithmetic 90); rv64's post-loop range check
    //     faults at the same width now, so b ≥ 8 is an agreement row.
    let body = "var b = byte(data, 0)\nvar r = ((2 ** b) as u64)\nreturn r as bytes";
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for b in [0u8, 1, 7] {
        knock_ok(&rv, &it, &[b], &be(2u64.checked_pow(b as u32).unwrap()));
    }
    for b in [8u8, 9, 20] {
        knock_trap(&rv, &it, &[b], ARITH as u32);
    }
    // with the base width pinned both agree (incl. the shared 2^64 fault)
    let body = "var b = byte(data, 0)\nvar r = ((2u64 ** b) as u64)\nreturn r as bytes";
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for b in [0u8, 7, 8, 63] {
        match 2u64.checked_pow(b as u32) {
            Some(v) => knock_ok(&rv, &it, &[b], &be(v)),
            None => knock_trap(&rv, &it, &[b], ARITH as u32),
        }
    }
    // (b) huge RUNTIME exponent with a non-overflowing base: the interpreter
    //     computes by squaring (checked_pow → instant answer Ok(1)), while
    //     the rv64 loop decrements the exponent one `mul` per step
    //     (rv64.rs POW else-arm) — 1^(u32::MAX) keeps the accumulator in
    //     range forever, so the loop runs until the CYCLE budget →
    //     CalcAbort::OutOfGas, exit code 0. (A base ≥ 2 faults after ~63
    //     muls on BOTH engines — the per-step overflow check catches it
    //     inside any budget — so the hazard is exactly base 0/1 with huge
    //     exponents. Generator hazard: keep `**` exponents literal, the ≤64
    //     unroll path.)
    let body = "var v = 0u64\nvar i = 0u64\nwhile i < 8 { v = v * 256 + byte(data, i)\ni += 1 }\n\
                var r = ((1u64 ** v) as u64)\nreturn r as bytes";
    let mut input = vec![0u8; 8];
    input.copy_from_slice(&0x7FFF_FFFFu64.to_be_bytes()); // exp = u32::MAX
    let got = Interp::build(&body)
        .run(&input)
        .expect("interpreter pow-by-squaring answers instantly");
    assert_eq!(got, be(1));
    for (mode, res) in Rv64::build(&body).run_with_limits(
        &input,
        vm::calc::CalcRunLimits { cycles: 200_000, resource: -1, deadline: None },
    ) {
        assert_eq!(
            res,
            Err(0),
            "mode {mode:?}: rv64 linear POW loop burns the cycle budget (exit 0)"
        );
    }
}

#[test]
fn g5_bytes_churn_bound_divergence_pinned() {
    // Resource bounds differ on BOTH threshold and class:
    // * interpreter: Value::bytes valid() cap = value_size 1280 B →
    //   OutOfValueSize(39) (value/util.rs checked_value_output_add)
    // * rv64: bump arena (ARENA_SIZE_MAX) churn bound → CastFail(71)
    //   (rv64.rs emit_arena_check → fault_cast)
    let body = "var b = buf_left(200, data)\nvar i = 0u64\nwhile i < 60 { b = b ++ buf_left(200, data)\ni += 1 }\nreturn (size(b) as u64) as bytes";
    let input = vec![0u8; 256];
    pin_divergence(
        &Rv64::build(body),
        &Interp::build(body),
        &input,
        (OUT_OF_VALUE, "OutOfValueSize"),
        Err(CAST as u32),
    );
    // below both bounds (final 600 B ≤ 1280, churn ≈ 1 KiB ≤ arena): agree
    let body = "var b = buf_left(200, data)\nvar i = 0u64\nwhile i < 2 { b = b ++ buf_left(200, data)\ni += 1 }\nreturn (size(b) as u64) as bytes";
    knock_ok(
        &Rv64::build(body),
        &Interp::build(body),
        &input,
        &be(3 * 200),
    );
}

#[test]
fn g5_expression_depth_limit_is_rv64_compile_only() {
    // Depth 9: the shared front end accepts, the interpreter RUNS the
    // bytecode. Im/Imc (fixed 8-value pool) reject at lowering; E10's
    // ImcRemap bank absorbs the hot int locals (reads become zero-cost
    // Refs, at most 8 stack slots live), so that mode lowers the same body
    // and agrees with the interpreter.
    let mut decls = String::new();
    for k in 0..10 {
        decls.push_str(&format!("var v{k} = byte(data, {k})\n"));
    }
    let mut e9 = "v8".to_string();
    for k in 0..8 {
        e9 = format!("(v{k} + {e9})");
    }
    let body = format!("{decls}var r = ({e9}) as u64\nreturn r as bytes");
    // interpreter executes it: v0 + (v1 + … (v7 + v8)) over all-ones = 9
    let input = [1u8; 16];
    let got = Interp::build(&body).run(&input).expect("interpreter runs depth 9");
    assert_eq!(got, be(9));
    let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
    let (ir, _, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .unwrap();
    // Im/Imc: fixed pool → reject with the limit named
    for mode in [Rv64Mode::Im, Rv64Mode::Imc] {
        let err = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX)
            .expect_err("depth 9 must be rejected in Im/Imc");
        assert!(
            err.to_string().contains("nesting exceeds"),
            "depth-9 error must name the limit: {err}"
        );
    }
    // ImcRemap: the bank absorbs the locals — compiles and executes
    let lo = lower_body_mode_arena(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX)
        .expect("ImcRemap lowers depth 9 via the E10 bank");
    let cap = vm::calc::SpaceCap::new(0);
    let profile = vm::calc::container::CALC_PROFILE_RV64IMC;
    let c = lo
        .pack_segments_profile(&cap, 64, 4800, 16, profile)
        .expect("pack");
    let f = vm::calc::CalcFnObj::build(c, &cap, vm::calc::CalcHostConfig::new()).expect("build");
    let ex = f.run(
        &input,
        vm::calc::CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
    );
    let out = ex.result.expect("ImcRemap runs depth 9");
    assert_eq!(out, be(9), "ImcRemap depth-9 agrees with the interpreter");
}

#[test]
fn g5_u128_agrees_as_lo_hi_pair() {
    // task 3 (B3): the former compile-only u128 boundary is now a lowering
    // — a u128 value is a (lo, hi) u64 pair, `as bytes` renders the full
    // 16-byte BE view, and all three modes lower it like the interpreter.
    let body = "var v = byte(data, 0)\nvar r = (v as u128)\nreturn r as bytes";
    let got = Interp::build(body).run(&[7]).expect("interpreter runs u128");
    let mut want = vec![0u8; 16];
    want[15] = 7;
    assert_eq!(got, want);
    let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
    let (ir, _, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .unwrap();
    for mode in MODES {
        let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX)
            .expect("u128 must lower now");
        assert!(!lo.code.is_empty());
    }
}

/* ================================================================== */
/* G6: kernel transcriptions (hacdtestnet fitsh_kernels.rs)            */
/* ================================================================== */

/// The hacdtestnet score_sum fitsh transcription (Diamond Analyzer v0.1.1),
/// with the four load-bearing width pins required by the interpreter's
/// narrow-width checked shifts (`1 << hv` / `1 << li` / `1 << (digit-48)`
/// → `1u64 << …`; the rv64 channel is u64-uniform so the original runs
/// there either way). Source: hacdtestnet
/// src/tests/calc_conformance/fitsh_kernels.rs SCORE_SUM_FITSH.
const SCORE_SUM_FITSH: &str = r#"// five-dimension score, Diamond Analyzer v0.1.1
var n = size(data) / 24
var ne = n * 6
var nue = ne + n * 8
var sum = 0u64
var i = 0u64
while i < n {
    var nb = i * 6
    var xb = ne + i * 8
    var gb = nue + i * 10

    // ---- shape: nibbles 0/1 of the visual gene ----
    var b0 = byte(data, gb)
    var sa = b0 / 16
    var sb = b0 % 16
    var shape = 0u64
    if sa == 0 {
        if sb == 1 { shape = 30 }
        if sb == 2 { shape = 30 }
        if sb == 3 { shape = 50 }
        if sb == 4 { shape = 20 }
        if sb == 5 { shape = 30 }
        if sb == 6 { shape = 30 }
        if sb == 7 { shape = 20 }
        if sb == 8 { shape = 20 }
    }

    var colorish = 0u64
    if shape == 0 {
        // ---- style (Common shape): nibbles 14..17 = bytes 7/8 ----
        var h7 = byte(data, gb + 7)
        var h8 = byte(data, gb + 8)
        var c13 = h7 / 16
        var c14 = h7 % 16
        var c15 = h8 / 16
        var c16 = h8 % 16
        var pos = 0u64
        if c13 == c14 && c14 == c15 && c15 == c16 { pos = 250 } else {
        if c13 == c14 && c14 == c15 && c14 != c16 { pos = 20 } else {
        if c13 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c13 == c14 && c14 == c16 && c15 != c13 { pos = 20 } else {
        if c14 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c16 == c15 && c13 == c14 { pos = 20 } else {
        if c16 == c14 && c13 == c15 { pos = 20 } else {
        if c15 * 16 + c13 == c14 * 16 + c16 { pos = 20 } else {
        if c13 == c14 && c15 != c16 { pos = 3 } else {
        if c15 == c16 && c14 != c13 { pos = 3 }
        }}}}}}}}}
        var bitmap = 0u64
        var alldark = 1u64
        var alllight = 1u64
        var k = 2u64
        while k < 18 {
            var hv = byte(data, gb + k / 2)
            if k % 2 == 0 { hv = hv / 16 } else { hv = hv % 16 }
            bitmap = bitmap | (1u64 << hv)
            if hv > 4 { alldark = 0 }
            if hv <= 4 { alllight = 0 }
            k += 1
        }
        var px = bitmap & 65535
        px = px - ((px >> 1) & 21845)
        px = (px & 13107) + ((px >> 2) & 13107)
        px = (px + (px >> 4)) & 3855
        px = px + (px >> 8)
        var distinct = px & 31
        var cnt = 0u64
        if distinct == 4 { if 200 > cnt { cnt = 200 } }
        if distinct == 5 { if 100 > cnt { cnt = 100 } }
        if distinct == 6 { if 50 > cnt { cnt = 50 } }
        if distinct == 7 { if 20 > cnt { cnt = 20 } }
        if distinct == 14 { if 30 > cnt { cnt = 30 } }
        if distinct == 15 { if 100 > cnt { cnt = 100 } }
        if distinct == 16 { if 200 > cnt { cnt = 200 } }
        var dl = 0u64
        if alldark == 1 { if 20 > dl { dl = 20 } }
        if alllight == 1 { if 20 > dl { dl = 20 } }
        colorish = pos
        if cnt > colorish { colorish = cnt }
        if dl > colorish { colorish = dl }
    } else {
        // ---- color (non-Common shape): name[0] table ----
        var nc = byte(data, nb)
        var cs = 0u64
        if nc == 65 { cs = 7 }
        if nc == 66 { cs = 6 }
        if nc == 69 { cs = 1 }
        if nc == 72 { cs = 8 }
        if nc == 73 { cs = 8 }
        if nc == 75 { cs = 8 }
        if nc == 77 { cs = 2 }
        if nc == 78 { cs = 6 }
        if nc == 83 { cs = 6 }
        if nc == 84 { cs = 8 }
        if nc == 85 { cs = 3 }
        if nc == 86 { cs = 8 }
        if nc == 87 { cs = 3 }
        if nc == 88 { cs = 5 }
        if nc == 89 { cs = 3 }
        if nc == 90 { cs = 10 }
        var alldark2 = 1u64
        var alllight2 = 1u64
        var k2 = 2u64
        while k2 < 19 {
            var hv2 = byte(data, gb + k2 / 2)
            if k2 % 2 == 0 { hv2 = hv2 / 16 } else { hv2 = hv2 % 16 }
            if hv2 > 4 { alldark2 = 0 }
            if hv2 <= 4 { alllight2 = 0 }
            k2 += 1
        }
        if alldark2 == 1 { if 20 > cs { cs = 20 } }
        if alllight2 == 1 { if 20 > cs { cs = 20 } }
        colorish = cs
    }

    // ---- nameliteral (6 name bytes) ----
    var maxrun = 1u64
    var run = 1u64
    var q = 1u64
    while q < 6 {
        var c1 = byte(data, nb + q)
        var c0 = byte(data, nb + q - 1)
        if c1 == c0 {
            run += 1
            if run > maxrun { maxrun = run }
        } else {
            run = 1
        }
        q += 1
    }
    var bm2 = 0u64
    q = 0
    while q < 6 {
        var lc = byte(data, nb + q)
        var li = 16u64
        if lc == 87 { li = 0 }
        if lc == 84 { li = 1 }
        if lc == 89 { li = 2 }
        if lc == 85 { li = 3 }
        if lc == 73 { li = 4 }
        if lc == 65 { li = 5 }
        if lc == 72 { li = 6 }
        if lc == 88 { li = 7 }
        if lc == 86 { li = 8 }
        if lc == 77 { li = 9 }
        if lc == 69 { li = 10 }
        if lc == 75 { li = 11 }
        if lc == 66 { li = 12 }
        if lc == 83 { li = 13 }
        if lc == 90 { li = 14 }
        if lc == 78 { li = 15 }
        bm2 = bm2 | (1u64 << li)
        q += 1
    }
    var px2 = bm2 & 65535
    px2 = px2 - ((px2 >> 1) & 21845)
    px2 = (px2 & 13107) + ((px2 >> 2) & 13107)
    px2 = (px2 + (px2 >> 4)) & 3855
    px2 = px2 + (px2 >> 8)
    var distinct2 = px2 & 31
    var nl = 0u64
    if maxrun >= 3 {
        if maxrun == 6 { nl = 200 }
        if maxrun == 5 { if 50 > nl { nl = 50 } }
        if maxrun == 4 { if 10 > nl { nl = 10 } }
        if maxrun < 4 { if 2 > nl { nl = 2 } }
    }
    if distinct2 == 2 { if 10 > nl { nl = 10 } }
    if distinct2 == 3 { if 3 > nl { nl = 3 } }
    if byte(data, nb) == byte(data, nb + 3) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 5) {
        if 20 > nl { nl = 20 }
    }
    if byte(data, nb) == byte(data, nb + 5) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 3) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb) == byte(data, nb + 1) && byte(data, nb + 3) == byte(data, nb + 4) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb + 1) == byte(data, nb + 2) && byte(data, nb + 4) == byte(data, nb + 5) {
        if 10 > nl { nl = 10 }
    }

    // ---- numberliteral: BE u64 fold, then the decimal digit string ----
    var num = 0u64
    var j = 0u64
    while j < 8 {
        num = num * 256 + byte(data, xb + j)
        j += 1
    }
    var nu = 0u64
    if num < 10 { nu = 30 }
    var digs = ""
    var x = num
    if x == 0 { digs = "0" }
    while x > 0 {
        var d10 = x % 10
        var ch = "9"
        if d10 == 0 { ch = "0" }
        if d10 == 1 { ch = "1" }
        if d10 == 2 { ch = "2" }
        if d10 == 3 { ch = "3" }
        if d10 == 4 { ch = "4" }
        if d10 == 5 { ch = "5" }
        if d10 == 6 { ch = "6" }
        if d10 == 7 { ch = "7" }
        if d10 == 8 { ch = "8" }
        digs = ch ++ digs
        x = x / 10
    }
    var dn = size(digs)
    var bm3 = 0u64
    j = 0
    while j < dn {
        bm3 = bm3 | (1u64 << (byte(digs, j) - 48))
        j += 1
    }
    var px3 = bm3 & 65535
    px3 = px3 - ((px3 >> 1) & 21845)
    px3 = (px3 & 13107) + ((px3 >> 2) & 13107)
    px3 = (px3 + (px3 >> 4)) & 3855
    px3 = px3 + (px3 >> 8)
    var distinct3 = px3 & 31
    if distinct3 == 1 { if 50 > nu { nu = 50 } }
    var up = 1u64
    var down = 1u64
    j = 1
    while j < dn {
        var cur = byte(digs, j)
        var pre = byte(digs, j - 1)
        if cur <= pre { up = 0 } else { if cur - pre != 1 { up = 0 } }
        if cur >= pre { down = 0 } else { if pre - cur != 1 { down = 0 } }
        j += 1
    }
    if up == 1 || down == 1 { if 25 > nu { nu = 25 } }
    var sym = 1u64
    j = 0
    while j < dn / 2 {
        if byte(digs, j) != byte(digs, dn - 1 - j) { sym = 0 }
        j += 1
    }
    if sym == 1 { if 10 > nu { nu = 10 } }
    var mrun = 1u64
    run = 1
    j = 1
    while j < dn {
        if byte(digs, j) == byte(digs, j - 1) {
            run += 1
            if run > mrun { mrun = run }
        } else {
            run = 1
        }
        j += 1
    }
    if mrun >= 4 {
        if mrun == 7 { nu = 200 }
        if mrun == 6 { if 100 > nu { nu = 100 } }
        if mrun == 5 { if 50 > nu { nu = 50 } }
        if mrun < 5 { if 20 > nu { nu = 20 } }
        if mrun > 7 { if 20 > nu { nu = 20 } }
    }

    // ---- total = max(1, shape + colorish + nl + nu) ----
    var total = shape + colorish + nl + nu
    if total < 1 { total = 1 }
    sum += total
    i += 1
}
return sum as bytes
"#;

/// Deterministic score_sum wire builder (from hacdtestnet fitsh_kernels.rs):
/// names[6n] || numbers[8n] || vg[10n].
fn score_wire(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut wire = Vec::with_capacity(n * 24);
    for _ in 0..n {
        for _ in 0..6 {
            wire.push(b'A' + (next() % 26) as u8);
        }
    }
    for _ in 0..n {
        wire.extend_from_slice(&next().to_be_bytes());
    }
    for _ in 0..n {
        for _ in 0..10 {
            wire.push((next() % 256) as u8);
        }
    }
    wire
}

#[test]
fn g6_score_sum_interpreter_matches_rv64_three_modes() {
    let rv = Rv64::build(SCORE_SUM_FITSH);
    let it = Interp::build(SCORE_SUM_FITSH);
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for n in 0..=4usize {
        cases.push(score_wire(n, 0x9E37_79B9 + n as u64));
    }
    let mk = |names: &[u8], vg: &[u8], num: u64| {
        let mut w = names.to_vec();
        w.extend(num.to_be_bytes());
        w.extend(vg);
        w
    };
    cases.push(mk(b"ABCDEF", &[0x00, 1, 2, 3, 4, 5, 6, 7, 0x11, 0x11], 0));
    cases.push(mk(
        b"AAAAAA",
        &[0xAA, 0x11, 0x11, 0x00, 0x11, 0x00, 0x11, 0x00, 0x11, 0x00],
        0,
    ));
    cases.push(mk(b"ABCABC", &[0x50; 10], 1234567890));
    cases.push(mk(b"ABCCBA", &[0x50; 10], 121));
    cases.push(mk(b"WWWWWW", &[0x51; 10], 1000000));
    cases.push(mk(b"ABCDEF", &[0x52; 10], 111111111111));
    cases.push(mk(b"ABCDEF", &[0x53; 10], 5));
    cases.push(mk(b"ABCDEF", &[0x54; 10], 123456));
    cases.push(mk(&[0; 6], &[0; 10], 0));
    cases.push(mk(b"ZZZZZZ", &[0xFF; 10], u64::MAX));
    let mut multi = score_wire(3, 7);
    multi.extend(mk(
        b"ABCABC",
        &[0x00, 9, 9, 9, 9, 9, 9, 9, 0x22, 0x22],
        999999,
    ));
    cases.push(multi);

    for (i, input) in cases.iter().enumerate() {
        let interp = it.run(input).unwrap_or_else(|e| {
            panic!("score_sum case {i}: interpreter fault {} ({})", e.code, e.msg)
        });
        assert_eq!(interp.len(), 8, "case {i}: u64 BE output");
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got,
                Ok(interp.clone()),
                "score_sum case {i} diverged in mode {mode:?} (len {})",
                input.len()
            );
        }
    }
}

/// hacd200 batch/single HACD scorer (hacdtestnet HACD200_FITSH), with ONE
/// load-bearing width pin: `var w8 = (total as u64) as bytes` — on the
/// interpreter `total` re-derives from narrow slots (`shape = 30` stores a
/// u8) and a bare `total as bytes` would be the 1-byte u8 view, while the
/// rv64 channel always emits the 8-byte BE view that `buf_cut(w8, 6, 2)`
/// below depends on. The only shift amount `li` is declared `16u64`, so the
/// narrow-width shift divergence never fires here.
/// hacd200 batch/single HACD scorer (hacdtestnet HACD200_FITSH + the shared
/// census .inc, P2-HACD200-COLOR body: the Common path now carries the
/// distinct-color-count rule verbatim — `1u64 << hv` needs no patch), with
/// TWO load-bearing interpreter-agreement patches on the PRE-EXISTING text
/// (both keep the body inside the interpreter<->rv64 agreement domain; the
/// divergence classes are the documented engine deviations at the top of
/// this file):
///  * width pin: `var w8 = (total as u64) as bytes` — on the interpreter
///    `total` re-derives from narrow slots (`shape = 30` stores a u8) and a
///    bare `total as bytes` would be the 1-byte u8 view, while the rv64
///    channel always emits the 8-byte BE view that `buf_cut(w8, 6, 2)`
///    below depends on;
///  * shift-width alignment: `bm2 = bm2 | (1u64 << li)` — li is u8-narrow and
///    the interpreter's matched-width checked_shl traps (90) for li >= 8 (any
///    of B/S/Z/N/V/M/E/K as a name letter), while rv64 is u64-uniform. The
///    hacdtestnet contract body was aligned to `1u64 << li` in the
///    P2-HACD200-COLOR close-out (master-agent takeover 2026-11-14; the
///    rv64 product is byte-identical for both spellings — census pins
///    2946/3072 unchanged), so this copy now matches the contract body
///    with no patch offset. The only other shift amount is hv (<= 15),
///    written `1u64 << hv` in the contract itself.
const HACD200_FITSH: &str = r#"
// batch/single HACD scorer, unified one-record-per-iteration loop
var cnt = 1u64
if size(data) != 32 {
    cnt = byte(data, 0) * 256 + byte(data, 1)
}
var out = ""
var r = 0u64
while r < cnt {
    var nb = 0
    var gb = 0
    var num = 0u64
    var wide = 0
    if size(data) == 32 {
        nb = 0
        gb = 6
        var j = 0u64
        while j < 8 { num = num * 256 + byte(data, 24 + j)
        j += 1 }
        wide = 1
    } else {
        nb = 2 + r * 20
        gb = nb + 6
        var j2 = 0u64
        while j2 < 4 { num = num * 256 + byte(data, nb + 16 + j2)
        j2 += 1 }
        wide = 0
    }
    var shape = 0u64
    var sa = byte(data, gb) / 16
    var sb = byte(data, gb) % 16
    if sa == 0 {
        if sb == 1 { shape = 30 }
        if sb == 2 { shape = 30 }
        if sb == 3 { shape = 50 }
        if sb == 4 { shape = 20 }
        if sb == 5 { shape = 30 }
        if sb == 6 { shape = 30 }
        if sb == 7 { shape = 20 }
        if sb == 8 { shape = 20 }
    }
    var colorish = 0u64
    if shape == 0 {
        var c13 = byte(data, gb + 7) / 16
        var c14 = byte(data, gb + 7) % 16
        var c15 = byte(data, gb + 8) / 16
        var c16 = byte(data, gb + 8) % 16
        var pos = 0u64
        if c13 == c14 && c14 == c15 && c15 == c16 { pos = 250 } else {
        if c13 == c14 && c14 == c15 && c14 != c16 { pos = 20 } else {
        if c13 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c13 == c14 && c14 == c16 && c15 != c13 { pos = 20 } else {
        if c14 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c16 == c15 && c13 == c14 { pos = 20 } else {
        if c16 == c14 && c13 == c15 { pos = 20 } else {
        if c15 * 16 + c13 == c14 * 16 + c16 { pos = 20 } else {
        if c13 == c14 && c15 != c16 { pos = 3 } else {
        if c15 == c16 && c14 != c13 { pos = 3 }
        }}}}}}}}}
        var alldark = 1u64
        var alllight = 1u64
        var bm3 = 0u64
        var k = 2u64
        while k < 18 {
            var hv = byte(data, gb + k / 2)
            if k % 2 == 0 { hv = hv / 16 } else { hv = hv % 16 }
            bm3 = bm3 | (1u64 << hv)
            if hv > 4 { alldark = 0 }
            if hv <= 4 { alllight = 0 }
            k += 1
        }
        var px3 = bm3 & 65535
        px3 = px3 - ((px3 >> 1) & 21845)
        px3 = (px3 & 13107) + ((px3 >> 2) & 13107)
        px3 = (px3 + (px3 >> 4)) & 3855
        px3 = px3 + (px3 >> 8)
        var distinct3 = px3 & 31
        var colcnt = 0u64
        if distinct3 == 4 { colcnt = 200 }
        if distinct3 == 5 { colcnt = 100 }
        if distinct3 == 6 { colcnt = 50 }
        if distinct3 == 7 { colcnt = 20 }
        if distinct3 == 14 { colcnt = 30 }
        if distinct3 == 15 { colcnt = 100 }
        if distinct3 == 16 { colcnt = 200 }
        colorish = pos
        if colcnt > colorish { colorish = colcnt }
        if alldark == 1 { if 20 > colorish { colorish = 20 } }
        if alllight == 1 { if 20 > colorish { colorish = 20 } }
    } else {
        var nc = byte(data, nb)
        var cs = 0u64
        if nc == 65 { cs = 7 }
        if nc == 66 { cs = 6 }
        if nc == 69 { cs = 1 }
        if nc == 72 { cs = 8 }
        if nc == 73 { cs = 8 }
        if nc == 75 { cs = 8 }
        if nc == 77 { cs = 2 }
        if nc == 78 { cs = 6 }
        if nc == 83 { cs = 6 }
        if nc == 84 { cs = 8 }
        if nc == 85 { cs = 3 }
        if nc == 86 { cs = 8 }
        if nc == 87 { cs = 3 }
        if nc == 88 { cs = 5 }
        if nc == 89 { cs = 3 }
        if nc == 90 { cs = 10 }
        var alldark2 = 1u64
        var alllight2 = 1u64
        var k2 = 2u64
        while k2 < 19 {
            var hv2 = byte(data, gb + k2 / 2)
            if k2 % 2 == 0 { hv2 = hv2 / 16 } else { hv2 = hv2 % 16 }
            if hv2 > 4 { alldark2 = 0 }
            if hv2 <= 4 { alllight2 = 0 }
            k2 += 1
        }
        colorish = cs
        if alldark2 == 1 { if 20 > colorish { colorish = 20 } }
        if alllight2 == 1 { if 20 > colorish { colorish = 20 } }
    }
    var maxrun = 1u64
    var run = 1u64
    var q = 1u64
    while q < 6 {
        var c1 = byte(data, nb + q)
        var c0 = byte(data, nb + q - 1)
        if c1 == c0 {
            run += 1
            if run > maxrun { maxrun = run }
        } else {
            run = 1
        }
        q += 1
    }
    var nl = 0u64
    if maxrun >= 3 {
        if maxrun == 6 { nl = 200 }
        if maxrun == 5 { if 50 > nl { nl = 50 } }
        if maxrun == 4 { if 10 > nl { nl = 10 } }
        if maxrun < 4 { if 2 > nl { nl = 2 } }
    }
    var bm2 = 0u64
    q = 0
    while q < 6 {
        var lc = byte(data, nb + q)
        var li = 16u64
        if lc == 87 { li = 0 }
        if lc == 84 { li = 1 }
        if lc == 89 { li = 2 }
        if lc == 85 { li = 3 }
        if lc == 73 { li = 4 }
        if lc == 65 { li = 5 }
        if lc == 72 { li = 6 }
        if lc == 88 { li = 7 }
        if lc == 86 { li = 8 }
        if lc == 77 { li = 9 }
        if lc == 69 { li = 10 }
        if lc == 75 { li = 11 }
        if lc == 66 { li = 12 }
        if lc == 83 { li = 13 }
        if lc == 90 { li = 14 }
        if lc == 78 { li = 15 }
        bm2 = bm2 | (1u64 << li)
        q += 1
    }
    var px2 = bm2 & 65535
    px2 = px2 - ((px2 >> 1) & 21845)
    px2 = (px2 & 13107) + ((px2 >> 2) & 13107)
    px2 = (px2 + (px2 >> 4)) & 3855
    px2 = px2 + (px2 >> 8)
    var distinct2 = px2 & 31
    if distinct2 == 2 { if 10 > nl { nl = 10 } }
    if distinct2 == 3 { if 3 > nl { nl = 3 } }
    if byte(data, nb) == byte(data, nb + 3) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 5) {
        if 20 > nl { nl = 20 }
    }
    if byte(data, nb) == byte(data, nb + 5) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 3) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb) == byte(data, nb + 1) && byte(data, nb + 3) == byte(data, nb + 4) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb + 1) == byte(data, nb + 2) && byte(data, nb + 4) == byte(data, nb + 5) {
        if 10 > nl { nl = 10 }
    }
    var nu = 0u64
    if num < 10 { nu = 30 }
    var digs = ""
    var x = num
    if x == 0 { digs = "0" }
    while x > 0 {
        var d10 = x % 10
        var ch = "9"
        if d10 == 0 { ch = "0" }
        if d10 == 1 { ch = "1" }
        if d10 == 2 { ch = "2" }
        if d10 == 3 { ch = "3" }
        if d10 == 4 { ch = "4" }
        if d10 == 5 { ch = "5" }
        if d10 == 6 { ch = "6" }
        if d10 == 7 { ch = "7" }
        if d10 == 8 { ch = "8" }
        digs = ch ++ digs
        x = x / 10
    }
    var dn = size(digs)
    var same = 1u64
    var j3 = 1u64
    while j3 < dn {
        if byte(digs, j3) != byte(digs, 0) { same = 0 }
        j3 += 1
    }
    if same == 1 { if 50 > nu { nu = 50 } }
    var up = 1u64
    var down = 1u64
    var j4 = 1u64
    while j4 < dn {
        var cur = byte(digs, j4)
        var pre = byte(digs, j4 - 1)
        if cur <= pre { up = 0 } else { if cur - pre != 1 { up = 0 } }
        if cur >= pre { down = 0 } else { if pre - cur != 1 { down = 0 } }
        j4 += 1
    }
    if up == 1 || down == 1 { if 25 > nu { nu = 25 } }
    var sym = 1u64
    var j5 = 0u64
    while j5 < dn / 2 {
        if byte(digs, j5) != byte(digs, dn - 1 - j5) { sym = 0 }
        j5 += 1
    }
    if sym == 1 { if 10 > nu { nu = 10 } }
    var mrun = 1u64
    run = 1
    var j6 = 1u64
    while j6 < dn {
        if byte(digs, j6) == byte(digs, j6 - 1) {
            run += 1
            if run > mrun { mrun = run }
        } else {
            run = 1
        }
        j6 += 1
    }
    if mrun >= 4 {
        if mrun == 7 { nu = 200 }
        if mrun == 6 { if 100 > nu { nu = 100 } }
        if mrun == 5 { if 50 > nu { nu = 50 } }
        if mrun < 5 { if 20 > nu { nu = 20 } }
        if mrun > 7 { if 20 > nu { nu = 20 } }
    }
    var total = shape + colorish + nl + nu
    if total < 1 { total = 1 }
    var w8 = (total as u64) as bytes
    if wide == 1 {
        out = out ++ w8
    } else {
        out = out ++ buf_cut(w8, 6, 2)
    }
    r += 1
}
return out
"#;
// rule fix landed in both in one step; see docs.loc/rv64im/68-hacd200-color-rule.md)

/// Single-mode 32-byte wire: name(6) || vg(10) || pad(8) || num(8 BE).
fn hacd200_single(name: &[u8; 6], vg: &[u8; 10], num: u64) -> Vec<u8> {
    let mut w = name.to_vec();
    w.extend(vg);
    w.extend([0u8; 8]);
    w.extend(num.to_be_bytes());
    w
}

/// Batch wire: [u16 BE count][count × name6|vg10|num4 BE u32].
fn hacd200_batch(records: &[([u8; 6], [u8; 10], u32)]) -> Vec<u8> {
    let mut w = (records.len() as u16).to_be_bytes().to_vec();
    for (n, vg, num) in records {
        w.extend(n);
        w.extend(vg);
        w.extend(num.to_be_bytes());
    }
    w
}

#[test]
fn g6_hacd200_interpreter_matches_rv64_three_modes() {
    let rv = Rv64::build(HACD200_FITSH);
    let it = Interp::build(HACD200_FITSH);
    let mut cases: Vec<Vec<u8>> = Vec::new();
    cases.push(hacd200_single(
        b"ABCDEF",
        &[0x01, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11],
        0,
    ));
    cases.push(hacd200_single(b"WWWWWW", &[0x50; 10], 7777777));
    cases.push(hacd200_single(b"ABCABC", &[0x00, 0, 9, 9, 9, 9, 9, 9, 9, 9], 123456));
    // P2-HACD200-COLOR: distinct-color-count rule cases (Common path, style
    // nibbles [2..18)) — the interpreter oracle validates the NEW contract
    // rule against all three rv64 modes (doc 68 battery mirror: d=4 fires
    // cnt=200, d=16 fires cnt=200, d=3 boundary does not fire; name AAAAAA
    // keeps the pre-existing `1 << li` name path trap-free: A -> li=5).
    {
        let mk = |bytes8: [u8; 8]| -> [u8; 10] {
            let mut vg = [0u8; 10];
            vg[0] = 0x90; // hex0=9 != 0 -> Common family gate
            vg[1..9].copy_from_slice(&bytes8);
            vg
        };
        cases.push(hacd200_single(b"AAAAAA", &mk([0x12, 0x34, 0x12, 0x34, 0x12, 0x34, 0x11, 0x34]), 37));
        cases.push(hacd200_single(b"AAAAAA", &mk([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]), 100));
        cases.push(hacd200_single(b"AAAAAA", &mk([0x11, 0x11, 0x22, 0x22, 0x33, 0x33, 0x11, 0x22]), 37));
    }
    let recs = |n: usize| -> Vec<([u8; 6], [u8; 10], u32)> {
        (0..n)
            .map(|i| {
                let name = [b'A' + (i % 26) as u8, b'W', b'T', b'Y', b'Z', b'N'];
                let vg = [
                    0x30 + i as u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
                ];
                (name, vg, 1000 * i as u32 + 7)
            })
            .collect()
    };
    for n in [1usize, 2, 5] {
        cases.push(hacd200_batch(&recs(n)));
    }
    for (i, input) in cases.iter().enumerate() {
        let interp = it.run(input).unwrap_or_else(|e| {
            panic!("hacd200 case {i}: interpreter fault {} ({})", e.code, e.msg)
        });
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got,
                Ok(interp.clone()),
                "hacd200 case {i} diverged in mode {mode:?} (len {})",
                input.len()
            );
        }
    }
}

/* ================================================================== */
/* G7: oracle performance (input for the C2/C4 generator tasks)        */
/* ================================================================== */


/// REAL RV64 BACKEND BUG (found by this oracle, 2026-11-12) — FIXED (P1-F,
/// BUG-009): `SliceKind::RDrop` in `vm/src/ir/rv64.rs` `emit_slice` used to
/// compute ONLY the result length (`sub dstl, rl, n`) and never materialize
/// the result pointer `dstp` — the freshly pushed pool register held
/// whatever the previous expression left there:
/// * Im / Imc: the register was uninitialized in this straight-line body →
///   the return copy read from address 0 → deterministic kernel TRAP
///   (CalcAbort::Trap, receipt exit code 0).
/// * ImcRemap: the register happened to still hold the ABI input base
///   (`data`'s pointer) → the result was correct, BY ACCIDENT (allocation
///   dependent).
/// The interpreter (value/operand.rs `dropright`) keeps the prefix
/// `buf[0..len-n]` — the spec behavior. Existing suites never caught this
/// because the differential only ever read `size(...)` (length-only) and
/// `buf_right` (a different slice kind). Fix: materialize `dstp` with the
/// same shape as `SliceKind::Left` (`if dstp != rp { mv dstp, rp }`) — the
/// condition holds exactly when the source is a bank Ref (a pool-loc source
/// already occupies the re-pushed pool slot, where the mv would be a no-op
/// self-move).
/// This test now PINS THE CORRECT SEMANTICS in all three modes (regression
/// pin for the fix; buf_right_drop = keep the prefix `buf[0..len-n]`).
#[test]
fn g5_buf_right_drop_pointer_bug_pinned() {
    let data: Vec<u8> = (0xA0u8..0xB0).collect(); // 16 bytes
    let want_prefix: Vec<u8> = data[..11].to_vec(); // dropright(5) keeps 0..11
    let body = "var r = buf_right_drop(5, data)\nreturn r";
    // interpreter: the spec behavior
    assert_eq!(
        Interp::build(body).run(&data),
        Ok(want_prefix.clone()),
        "interpreter dropright keeps the prefix"
    );
    // rv64 per mode: the fixed pointer materialization — prefix content
    for (mode, got) in Rv64::build(body).run(&data) {
        assert_eq!(
            got,
            Ok(want_prefix.clone()),
            "mode {mode:?}: RDrop must materialize the prefix pointer (BUG-009 fix)"
        );
    }
    // the same root behind CAT (memcpy from the result pointer): the
    // concatenated CONTENT is compared, not just the size
    let body = "var r = buf_right_drop(5, data)\nvar o = r ++ buf_left(2, data)\nreturn o";
    let mut want_cat = data[..11].to_vec();
    want_cat.extend_from_slice(&data[..2]);
    for (mode, got) in Rv64::build(body).run(&data) {
        assert_eq!(
            got,
            Ok(want_cat.clone()),
            "mode {mode:?}: CAT after RDrop must carry the correct content (BUG-009 fix)"
        );
    }
}


#[test]
fn g7_oracle_performance_probe() {
    // Timing probes for the generator knock (C2/C4): interpreter =
    // tokenize+compile once, then per-case NativeVm run; rv64 = compile +
    // lower + pack + verify once, then per-case container run (×3 modes).
    // Numbers are informational (printed); no threshold is asserted.
    let body = SCORE_SUM_FITSH;
    let input = score_wire(8, 42); // 8 records, 192 bytes

    let t = std::time::Instant::now();
    let it = Interp::build(body);
    let interp_compile = t.elapsed();

    let t = std::time::Instant::now();
    let rv = Rv64::build(body);
    let rv_build = t.elapsed();

    let n = 20;
    let mut want: Option<Vec<u8>> = None;
    let t = std::time::Instant::now();
    for _ in 0..n {
        want = Some(it.run(&input).expect("interp run"));
    }
    let interp_per_case = t.elapsed() / n;

    let t = std::time::Instant::now();
    for _ in 0..n {
        for (mode, got) in rv.run(&input) {
            assert_eq!(got, Ok(want.clone().unwrap()), "mode {mode:?}");
        }
    }
    let rv_per_case_3modes = t.elapsed() / n;

    println!(
        "oracle perf (score_sum n=8, 192B input):\n\
         \x20 interpreter compile  : {interp_compile:?}\n\
         \x20 interpreter per case : {interp_per_case:?}\n\
         \x20 rv64 build (3 modes) : {rv_build:?}\n\
         \x20 rv64 per case (3 mod): {rv_per_case_3modes:?}"
    );
    // sanity: all engines agreed above
    assert_eq!(want.unwrap().len(), 8);
}

#[test]
fn g7b_oracle_fixed_overhead_and_loop_cost_probe() {
    // Cost model inputs for C2/C4: (a) fixed per-run overhead on a trivial
    // body (ctx setup + frame + entry + one instruction), (b) per-iteration
    // loop cost (bytecode ops per `while i < N { i += 1 }` iteration), so a
    // generator campaign can budget `programs × inputs × cost` up front.
    let n = 50;

    let tiny = "return 1u64 as bytes";
    let it_tiny = Interp::build(tiny);
    let rv_tiny = Rv64::build(tiny);
    // warm once
    let _ = it_tiny.run(&[]);
    let t = std::time::Instant::now();
    for _ in 0..n {
        assert_eq!(it_tiny.run(&[]), Ok(be(1)));
    }
    let interp_fixed = t.elapsed() / n;
    let t = std::time::Instant::now();
    for _ in 0..n {
        for (_, got) in rv_tiny.run(&[]) {
            assert_eq!(got, Ok(be(1)));
        }
    }
    let rv_fixed_3modes = t.elapsed() / n;

    let iters = 20_000u64;
    let loop_body = format!("var i = 0u64\nwhile i < {iters} {{ i += 1 }}\nreturn i as bytes");
    let it_loop = Interp::build(&loop_body);
    let rv_loop = Rv64::build(&loop_body);
    let m = 5;
    let t = std::time::Instant::now();
    for _ in 0..m {
        assert_eq!(it_loop.run(&[]), Ok(be(iters)));
    }
    let interp_loop = t.elapsed() / m;
    let t = std::time::Instant::now();
    for _ in 0..m {
        for (_, got) in rv_loop.run(&[]) {
            assert_eq!(got, Ok(be(iters)));
        }
    }
    let rv_loop_3modes = t.elapsed() / m;

    println!(
        "oracle cost model (debug build; C2/C4 budget input):\n\
         \x20 interpreter fixed/run   : {interp_fixed:?}\n\
         \x20 interpreter 20k-iter loop: {interp_loop:?} ({:?}/iter)\n\
         \x20 rv64 fixed/run (3 modes): {rv_fixed_3modes:?}\n\
         \x20 rv64 20k-iter loop (3m) : {rv_loop_3modes:?} ({:?}/iter)",
        interp_loop / iters as u32,
        rv_loop_3modes / iters as u32,
    );
}

#[test]
fn g7c_runaway_loop_is_bounded_on_both_engines() {
    // The throttle policy pin for C2/C4: a generated program that never
    // terminates must fail as a budget abort on BOTH engines, deterministically.
    // * interpreter: finite gas on the oracle context (per-instruction charge
    //   → the fault mentions out-of-gas).
    // * rv64: finite cycle budget → CalcAbort::OutOfGas, which carries NO
    //   guest code: exit code 0 (calc/mod.rs engine_from_interp).
    let body = "var i = 0u64\nwhile i < 1000000000 { i += 1 }\nreturn i as bytes";
    let it = Interp::build(body);
    let rv = Rv64::build(body);
    // completion sanity on a SMALL loop first (generous budgets: both agree)
    let small = "var i = 0u64\nwhile i < 10000 { i += 1 }\nreturn i as bytes";
    let got = Interp::build(small).run(&[]).expect("interpreter completes");
    assert_eq!(got, be(10_000));
    for (mode, res) in Rv64::build(small).run(&[]) {
        assert_eq!(res, Ok(be(10_000)), "mode {mode:?}");
    }
    // interpreter runaway with a finite gas budget: dies on gas (the budget
    // is deliberately small so the debug-mode suite stays fast; the policy,
    // not the number, is pinned)
    let got = it.run_bounded(&[], 5_000_000).expect_err("must exhaust");
    assert!(
        got.msg.contains("out of gas"),
        "interpreter runaway must die on gas: {}",
        got.msg
    );
    // rv64 runaway with a finite cycle budget: Err, exit code 0
    for (mode, res) in rv.run_with_limits(
        &[],
        vm::calc::CalcRunLimits { cycles: 100_000, resource: -1, deadline: None },
    ) {
        assert_eq!(
            res,
            Err(0),
            "mode {mode:?}: rv64 cycle exhaustion is Err with exit code 0"
        );
    }
}

/* ================================================================== */
/* P2-A19 batch 2: dominated constant-mul check merge, trap timing     */
/* ================================================================== */

/// Boundary cross-value for the merged chains (kept in sync with
/// `rv64_a19_guard_merge.rs::a19_bodies`, whose pins prove these bodies
/// actually exercise the merge):
///   - 0x2AAAAAAAAAAAAAAB = ceil(2^64/6): 6x overflows AND 8x overflows —
///     the pre-merge code traps at the 6x site, the merged code at the
///     8x/10x site. Same iteration, same exit 90, no observable prefix
///     (traps produce no output).
///   - 0x2AAAAAAAAAAAAAAA = ceil(2^64/6)-1: 6x fits, 8x overflows — both
///     versions trap at the retained 8x site.
///   - 2^61 = ceil(2^64/8): 8x overflows exactly — i-chain: the deleted
///     8x check's trap inputs (8<10) land on the retained 10x check.
///   - 0x199999999999999A = ceil(2^64/10): only 10x overflows — both
///     versions trap at the retained site.
///   - 2^61-1, small, MAX: the no-merge / downstream-add boundaries.
const A19_EDGES: [u64; 8] = [
    0,
    5,
    0x1999_9999_9999_999A, // ceil(2^64/10)
    1 << 61,               // ceil(2^64/8)
    (1 << 61) - 1,
    0x2AAA_AAAA_AAAA_AAAA, // ceil(2^64/6) - 1
    0x2AAA_AAAA_AAAA_AAAB, // ceil(2^64/6)
    u64::MAX,
];

fn a19_checked_model(x: u64, ks: &[u64]) -> Option<u64> {
    // fold `((x*k1) + x*k2) + x*k3`-style chains through checked arithmetic
    let mut acc = 0u64;
    for &k in ks {
        acc = acc.checked_add(x.checked_mul(k)?)?;
    }
    Some(acc)
}

/// The n-chain (`ne = n*6; nue = ne + n*8`): for every boundary value the
/// interpreter oracle (consensus checked semantics — it traps at the 6n
/// site for the 6n-overflow inputs) and all three rv64 modes (which trap
/// at the retained 8n site after the merge) agree on the exit code, and on
/// the full output for the non-trapping values.
#[test]
fn a19_b2_n_chain_trap_timing_boundaries() {
    let body = "var n = 0u64\nvar i = 0u64\nwhile i < 8 { n = n * 256 + byte(data, i)\ni += 1 }\n\
        var ne = n * 6\nvar nue = ne + n * 8\nreturn nue as bytes\n";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for x in A19_EDGES {
        let input = be(x);
        match a19_checked_model(x, &[6, 8]) {
            Some(v) => knock_ok(&rv, &it, &input, &be(v)),
            None => knock_trap(&rv, &it, &input, ARITH as u32),
        }
    }
}

/// The i-chain (`a = x*6; b = a + x*8; c = b + x*10`): 6x and 8x checks
/// are deleted under the retained 10x check. Every boundary value agrees
/// interpreter-vs-rv64 (all modes), trap code 90, outputs identical.
#[test]
fn a19_b2_i_chain_trap_timing_boundaries() {
    let body = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
        var a = x * 6\nvar b = a + x * 8\nvar c = b + x * 10\nreturn c as bytes\n";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for x in A19_EDGES {
        let input = be(x);
        match a19_checked_model(x, &[6, 8, 10]) {
            Some(v) => knock_ok(&rv, &it, &input, &be(v)),
            None => knock_trap(&rv, &it, &input, ARITH as u32),
        }
    }
    // one value PAST each single boundary, sweeping the full region where
    // exactly one / exactly two of the checks fire
    for x in [
        0x1999_9999_9999_999Au64 + 1,
        (1 << 61) + 1,
        0x2AAA_AAAA_AAAA_AAABu64 + 1,
        u64::MAX - 1,
        u64::MAX / 2,
    ] {
        let input = be(x);
        match a19_checked_model(x, &[6, 8, 10]) {
            Some(v) => knock_ok(&rv, &it, &input, &be(v)),
            None => knock_trap(&rv, &it, &input, ARITH as u32),
        }
    }
}

/// An observable prefix BEFORE the merged region must survive the trap
/// identically: the pre-chain arena write (the digs bytes built earlier)
/// is not emitted (traps produce no output on either side), and for the
/// non-trapping values the full output — including the prefix-derived
/// parts — matches the oracle byte-for-byte.
#[test]
fn a19_b2_chain_with_observable_prefix() {
    let body = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
        var s = \"\"\nvar j = 0u64\nwhile j < 3 { s = s ++ \"z\"\nj += 1 }\n\
        var a = x * 6\nvar b = a + x * 8\nreturn (s ++ (b as bytes))\n";
    let rv = Rv64::build(body);
    let it = Interp::build(body);
    for x in A19_EDGES {
        let input = be(x);
        // model: prefix "zzz" ++ BE(6x+8x) when everything fits
        match a19_checked_model(x, &[6, 8]) {
            Some(v) => {
                let mut want = b"zzz".to_vec();
                want.extend(be(v));
                knock_ok(&rv, &it, &input, &want)
            }
            None => knock_trap(&rv, &it, &input, ARITH as u32),
        }
    }
}

/// The merge spanning a >4 KiB straight-line bulk: every guard (including
/// the retained 10x check and the in-bulk +1 checks) is in the relaxed
/// trampoline form, and the deleted checks' bytes are cut from under the
/// loop backedges (re-patched displacements). Behavior must still match
/// the interpreter oracle on the trap boundaries.
#[test]
fn a19_b2_merge_across_relaxation_bulk() {
    let mut body = String::from(
        "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
         var a = x * 6\nvar b = a + x * 8\nvar t = b\n",
    );
    for _ in 0..800 {
        body.push_str("t = t + 1\n");
    }
    body.push_str("var c = t + x * 10\nreturn c as bytes\n");
    let rv = Rv64::build(&body);
    let it = Interp::build(&body);
    for x in A19_EDGES {
        let input = be(x);
        // model: ((6x + 8x) checked) + 800 checked +1's + checked 10x
        let model = a19_checked_model(x, &[6, 8]).map(|v| {
            let mut t = v;
            for _ in 0..800u64 {
                t = t.checked_add(1).unwrap();
            }
            t.checked_add(x.checked_mul(10).unwrap()).unwrap()
        });
        match model {
            Some(v) => knock_ok(&rv, &it, &input, &be(v)),
            None => knock_trap(&rv, &it, &input, ARITH as u32),
        }
    }
}
