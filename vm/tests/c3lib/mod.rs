//! C2: Csmith-style syntax-directed generator + four-way differential knock
//! (consensus interpreter vs rv64 {Im, Imc, ImcRemap}) over a seeded corpus.
//!
//! This file is the P1-C2 deliverable (docs.loc/rv64im/70-c2-generator.md):
//! * a deterministic, seed-driven generator for the pinned kernel subset
//!   (grammar coverage self-reported per production),
//! * a knock harness reusing the C1 oracle pattern (`rv64_oracle_ground`):
//!   one program + N inputs → interpreter result + three rv64-mode results,
//! * a classifier that maps every outcome to a verdict:
//!   `AgreeOk / AgreeTrap90 / AgreeTrap71 / AgreeUserValue / PinD1..D5 / D9 /
//!   D10 / Budget / Unexpected`, where `Unexpected` = suspected bug.
//!   The pinned rows are the C1 mapping table (30-c1 E2) plus D9 (CHOOSE arm
//!   eagerness, this round) and D10 (clamp lo>hi, this round).
//! * probe tests that pin engine edge behaviors, including dedicated
//!   PINNED-UNFIXED repro pins for the two candidate bugs found by this
//!   generator's first bombardment (BUG-010 candidate: clamp lo>hi guard
//!   missing; BUG-011 candidate: value-if with bytes arms miscompiled),
//! * negative suites: frontend-reject family and rv64-compile-reject family,
//! * determinism meta-checks: same seed → same program text; same program →
//!   byte-identical bytecode and rv64 containers.
//!
//! Generation constraints (the C1 five, docs 30-c1 §Conclusions, refined):
//!   ① all arithmetic/shift/POW operands are u64-pinned at birth (`as u64`
//!      on every byte/size leaf, `Nu64` literal suffixes); narrow-width
//!      arithmetic exists only where `feat.narrow`-marked shapes emit it
//!      (the D5 family), and declared vars are always re-widened to u64;
//!   ② returns are `(u64 expr) as bytes` (8B BE) or bare bytes values whose
//!      static length bound is ≤ 8 B (harness packs output_max = 16);
//!   ③ `**` exponents are literals in 0..=64;
//!   ④ inputs ≤ 1280 B (interpreter value-size cap envelope);
//!   ⑤ comparison of engines is ALWAYS through the classifier, never a bare
//!      equality — the pinned divergence rows are expected outcomes.
//!   ⑥ value-position arms (P2-C3 unfreeze, post BUG-011 fix — doc 13 §5):
//!      value-position `if` (IRIFR) BYTES×BYTES arms ARE generated
//!      (E_bytes_if_arm; gate = c2_probe_bytes_if_arms, 240 cases zero-
//!      unexpected, incl. banked Ref arms). CHOOSE (ternary/choose()) with
//!      bytes arms is NOT generated: the eager-CHOOSE selection used to
//!      swap (len, ptr) pairs — BUG-017, FIXED by P2-C3FIX (selection arm
//!      collects bottom-first via take_arm_values; pinned FLIPPED in
//!      vm/tests/rv64_c3_coverage.rs::c3_pin_bug017_choose_bytes_pair_swap)
//!      — the generation face stays frozen pending fix soak. CHOOSE with
//!      MIXED int/bytes arms (int-first) IS generated as the NEGATIVE
//!      channel: rv64 cleanly rejects ("choose arms must have the same
//!      result type", the P2-C3FIX up-front arm-type check), the
//!      interpreter accepts and runs (S_choose_mixed_arm + run_campaign
//!      mixed_arm routing). Bare value-expression statements stay frozen
//!      (P19a/P19b subset boundary; generated form would land in the
//!      reject family).
//!
//! Loop discipline: every generated `while` increments its counter FIRST, so
//! `continue` cannot skip the increment → all trips are compile-time bounded
//! (outer ≤ 16, inner ≤ 4), and both engines are cycle/gas bounded in the
//! harness (rv64 `CalcRunLimits.cycles`, interpreter `OracleCtx` gas).
//!
//! Depth discipline: the lowerer rejects bodies whose value-register stack
//! exceeds the 8-entry pool ("expression nesting exceeds…", rv64.rs
//! `depth_err`). The generator budgets expression depth (≤ 3), condition
//! depth (≤ 2) and control-flow nesting (≤ 3, decremented by EVERY
//! if/while/value-if), and the campaign retries a seed with halved budgets
//! if the lowerer still rejects (deterministic: the retry is a pure function
//! of the seed). Compile rejects are therefore never expected from the
//! generator; any that appear are counted and asserted zero.
//!
//! Tiers:
//!   default:  `c2_*` (probes + negatives + smoke + 150-case campaign +
//!              quick determinism) — debug gate increment ≈ 2 s;
//!   `#[ignore]`: `c2_bombard_mass` (env GEN_CORPUS_N, default 1500),
//!              `c2_mutation_pass`, `c2_determinism_50` — release runs.
//!
//! Status (P1-C2 re-dispatch, 2026-11-13, HEAD cd53275): the parked draft
//! (`_parked_p1c2/rv64_gen_corpus.rs`, 4/9 tests failing) was audited and
//! reworked. Draft bugs fixed: decl-splice order (use-before-decl), missing
//! control-depth budget (nesting rejects), classifier require/throw value
//! extraction (the `Nu64` suffix broke the trailing digit-run), and
//! fault-site divergence rows that only handled the rv64-Ok outcome (a
//! masked shift / lazy CHOOSE arm can be followed by ANY later rv64 trap).
//! Draft claims verified: D9 (CHOOSE arm eagerness) is a REAL divergence
//! (bytecode evaluates all three operands eagerly — `IRNodeTriple::
//! codegen_into` pushes subx/suby/subz, node.rs:508-518 — while rv64 lowers
//! IRIFR/CHOOSE branch-lazily, rv64.rs:3258/4058). New findings this round:
//! D10 clamp lo>hi (interpreter Arithmetic(90), instruction.rs:978-989 vs
//! rv64 unchecked min/max chain, rv64.rs:3910-3930) and BUG-011 candidate
//! (value-if bytes arms: Im/Imc Err(71), ImcRemap Err(0), interpreter Ok —
//! mode divergence). Full record: docs.loc/rv64im/70-c2-generator.md.
//!
//! Status (P1-C2 re-dispatch 2, 2026-11-13, HEAD 90244dc): D9 re-verified
//! from source with a NARROWER boundary than the draft claimed — only the
//! CHOOSE node (ternary `? :` and `choose()`, both → IRNodeTriple{CHOOSE},
//! expr.rs:46/51-66) is eager in the bytecode channel (compile.rs:362-365
//! compile_triple handles only IRIF|IRIFR; CHOOSE → None → node.rs:519-528
//! eager); value-position if is IRIFR (expr.rs:518) and lazy on BOTH
//! engines, so E_choose's value-if shape no longer sets `feat.choose`.
//! First bombardment (1199 cases × ~7 inputs, release) found THREE things,
//! all pinned below (P20/P21 probes + classifier rows):
//!   * BUG-013 candidate (compile-reject soundness gap): self-referencing
//!     XOP (`zr /= zr`, `a += a`, …) on a bank-promoted slot leaks one
//!     vstack entry in the ImcRemap lowering — rv64.rs XOP arm, the
//!     `VLoc::Ref(v) if v == rw` materialization does `push_vreg()` and
//!     never pops. Im/Imc lower+run fine; ImcRemap rejects EVERY such body
//!     with "value stack not empty at function end" (the leak persists to
//!     function end, so it can never silently miscompile — it always
//!     rejects valid source). Pinned: c2_probe_bug013_xop_self_leak.
//!   * BUG-014 candidate (trap-skipping optimization): `is_pure_cond`
//!     (rv64.rs:2594-2610) marks EQ/NEQ/LT/GT/LE/GE pure WITHOUT recursing
//!     into faultable children, so branch_cond's fault-free-conjunction
//!     AND chain (rv64.rs:4126-4132) short-circuits past a consensus-
//!     visible trap (byte OOB / div-zero / cast) inside the right operand.
//!     The bytecode channel is eager for AND/OR (compile.rs:64-68 — only
//!     IRWHILE is lazy in compile_double; interpreter AND at
//!     execute.rs:1104). Minimal repro: interp Err(42) vs rv64 Err(90)
//!     (it skipped the byte OOB, then trapped at a later MUL overflow).
//!     Pinned: c2_probe_bug014_and_chain_skipped_trap; campaign rows
//!     PinAndChainSkippedTrap (feat.and_chain).
//!   * D11 bytes flavor: an uninitialized BYTES slot read faults the
//!     interpreter with BytesHandle(91) (canbe.rs:125-133: Nil →
//!     itr_err_code!(BytesHandle), from operand.rs cutbyte/cutleft/…),
//!     while rv64 zero-fills the slot to empty bytes and faults 71 at the
//!     byte op (or continues). Same root as BUG-012 (int Nil-compare 90 vs
//!     zero-fill), NEW mapping-table row (91 vs 71/Ok). Classified via
//!     PinD11UninitNil on fault class 91; pinned: BUG-012 bytes pin in
//!     c2_probe_candidate_bug_pins.

use std::collections::BTreeMap;
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

pub const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// Interpreter gas per run (deterministic per-instruction charge; the
/// generator's loop bounds keep real programs orders of magnitude below).
pub const INTERP_GAS: i64 = 100_000_000;
/// rv64 cycle budget per run; retried once at 20x before declaring Budget.
pub const RV64_CYCLES: u64 = 10_000_000;
pub const RV64_CYCLES_RETRY: u64 = 200_000_000;

/* ================================================================== */
/* Section 1: deterministic PRNG (splitmix64) — generation is a pure   */
/* function of the u64 seed (+ the halvable depth budget).             */
/* ================================================================== */

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0xC2A1_5EED_D10F_1234)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    pub fn range(&mut self, lo: u64, hi_incl: u64) -> u64 {
        lo + self.below(hi_incl - lo + 1)
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/* ================================================================== */
/* Section 2: oracle harness (interpreter + rv64, three modes).        */
/* Rebuild of the C1 `rv64_oracle_ground` wrappers with Result-based    */
/* builds (so compile rejects are data, not panics) and u64 exit codes  */
/* (user require/throw values go up to u64::MAX).                       */
/* ================================================================== */

pub struct OracleRegistry {
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

pub fn oracle_vm_assigner() -> &'static base::VmAssignFn {
    static ASSIGNER: std::sync::OnceLock<base::VmAssignFn> = std::sync::OnceLock::new();
    ASSIGNER.get_or_init(|| {
        let mut reg = OracleRegistry { vm_assigner: None };
        vm::register_exec(&mut reg).expect("register vm exec");
        reg.vm_assigner
            .expect("register_exec installs the vm assigner")
    })
}

pub static ORACLE_VM_PARAMS: VmExecutionParams = VmExecutionParams {
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

#[derive(Default)]
pub struct MemLayer(std::collections::HashMap<Vec<u8>, Vec<u8>>);

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
        self.0.remove(&key.to_vec());
    }
}

pub struct OracleServices;

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

#[derive(Debug)]
pub struct OracleTx(Address);

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

pub struct OracleCtx {
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
    fn p2sh_set(&mut self, _adr: Address, _p2sh: Box<dyn P2sh>) -> Rerr {
        Ok(())
    }
}

/// Interpreter-side fault: the HVM class parsed off `Class(N): tip`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpFault {
    pub code: u16,
    pub msg: String,
}

pub fn parse_fault(e: &sys::Error) -> InterpFault {
    let msg = e.as_str().to_string();
    if let Some(open) = msg.find('(') {
        let rest = &msg[open + 1..];
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let code = rest[..digits].parse().unwrap_or(0);
            return InterpFault { code, msg };
        }
    }
    InterpFault { code: 0, msg }
}

pub struct Interp {
    codes: Arc<[u8]>,
}

impl Interp {
    pub fn try_build(body: &str) -> Result<Self, String> {
        let tokens = Tokenizer::new(body.as_bytes())
            .parse()
            .map_err(|e| format!("frontend(tokenize): {e}"))?;
        let (_, compiled, _) = compile_body(
            tokens,
            vec![("data".to_string(), ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .map_err(|e| format!("frontend(compile): {e}"))?;
        let CompiledCode::Bytecode(bts) = compiled else {
            return Err("frontend(compile): expected bytecode".to_string());
        };
        Ok(Self { codes: bts.into() })
    }

    pub fn run(&self, input: &[u8]) -> Result<Vec<u8>, InterpFault> {
        self.run_bounded(input, INTERP_GAS)
    }

    pub fn run_bounded(&self, input: &[u8], gas: i64) -> Result<Vec<u8>, InterpFault> {
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
            // `end` / fall-off-the-end ≡ empty bytes (pinned C1 equivalence)
            Value::Nil => Ok(vec![]),
            other => panic!("interpreter returned non-bytes value: {other:?}"),
        }
    }
}

/// One rv64 program: all three lowering modes built once.
pub struct Rv64 {
    pub engines: Vec<(Rv64Mode, vm::calc::CalcFnObj)>,
    /// each mode's packed container bytes (determinism meta-checks)
    pub containers: Vec<(Rv64Mode, Vec<u8>)>,
    /// HVM bytecode size (budget stats)
    pub hvm_size: usize,
    /// rv64 code mirror size in bytes (budget stats)
    pub code_size: usize,
    /// lowered code stream per mode (C3 emit-site instrumentation)
    pub codes: Vec<(Rv64Mode, Vec<u8>)>,
}

/// Compile the shared front end only (tokenizer + compile_body → IR).
pub fn frontend(body: &str) -> Result<(vm::ir::IRNodeArray, usize), String> {
    let tokens = Tokenizer::new(body.as_bytes())
        .parse()
        .map_err(|e| format!("frontend(tokenize): {e}"))?;
    let (ir, compiled, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .map_err(|e| format!("frontend(compile): {e}"))?;
    let CompiledCode::Bytecode(bts) = compiled else {
        return Err("frontend(compile): expected bytecode".to_string());
    };
    Ok((ir, bts.len()))
}

impl Rv64 {
    /// Lower + pack + verify all three modes (containers are round-tripped
    /// through encode/decode so the wire path is what gets verified+built).
    /// `Err` carries the first backend error (the negative suite asserts on
    /// its text).
    pub fn try_build(body: &str) -> Result<Self, String> {
        Self::try_build_out(body, 16, ARENA_SIZE_MAX)
    }

    /// P2-C3 B3 (bytes>16 return channel): parameterized pack envelope.
    /// `output_max` 16 with the full arena (ARENA_SIZE_MAX) is the C2 default
    /// (bit-identical to the pre-C3 build); the `bigout` family packs
    /// output_max 64 over a 28 KiB arena — the 64 KiB chain mem cap cannot
    /// hold input 4800 + output 64 + the full 0x9D30 arena (needs 65 KiB,
    /// measured), so the arena budget is the harness knob that admits the
    /// larger output channel.
    pub fn try_build_out(body: &str, output_max: u16, arena: u64) -> Result<Self, String> {
        let (ir, hvm_size) = frontend(body)?;
        let cap = vm::calc::SpaceCap::new(0);
        let mut engines = Vec::new();
        let mut containers = Vec::new();
        let mut codes = Vec::new();
        let mut code_size = 0usize;
        for &mode in MODES.iter() {
            let lo = lower_body_mode_arena(&ir, true, mode, arena)
                .map_err(|e| format!("rv64(lower,{mode:?}): {e}"))?;
            code_size = lo.code.len();
            codes.push((mode, lo.code.clone()));
            let profile = match mode {
                Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
                _ => vm::calc::container::CALC_PROFILE_RV64IMC,
            };
            let container = lo
                .pack_segments_profile(&cap, 64, 4800, output_max, profile)
                .map_err(|e| format!("rv64(pack,{mode:?}): {e}"))?;
            let bytes = container.encode().map_err(|e| format!("rv64(encode,{mode:?}): {e}"))?;
            let (rebuilt, _) = vm::calc::container::CalcContainer::decode(&bytes)
                .map_err(|e| format!("rv64(roundtrip,{mode:?}): {e}"))?;
            let fno = vm::calc::CalcFnObj::build(rebuilt, &cap, vm::calc::CalcHostConfig::new())
                .map_err(|e| format!("rv64(build,{mode:?}): {e}"))?;
            engines.push((mode, fno));
            containers.push((mode, bytes));
        }
        Ok(Self { engines, containers, hvm_size, code_size, codes })
    }

    pub fn run(&self, input: &[u8]) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
        self.run_with_limits(
            input,
            vm::calc::CalcRunLimits { cycles: RV64_CYCLES, resource: -1, deadline: None },
        )
    }

    pub fn run_with_limits(
        &self,
        input: &[u8],
        limits: vm::calc::CalcRunLimits,
    ) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
        self.engines
            .iter()
            .map(|&(mode, ref fno)| {
                let ex = fno.run(input, limits);
                (mode, ex.result.map_err(|_| ex.exit_code))
            })
            .collect()
    }
}

/* ================================================================== */
/* Section 3: the generator.                                           */
/* ================================================================== */

/// Per-production usage counters (grammar coverage self-report).
#[derive(Default, Clone)]
pub struct Coverage(pub BTreeMap<String, u64>);

impl Coverage {
    pub fn hit(&mut self, prod: &str) {
        *self.0.entry(prod.to_string()).or_insert(0) += 1;
    }
    pub fn merge(&mut self, other: &Coverage) {
        for (k, v) in other.0.iter() {
            *self.0.entry(k.clone()).or_insert(0) += v;
        }
    }
}

/// Case features the classifier consumes (to distinguish pinned-divergence
/// families from genuine bugs).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Feat {
    /// a `? :` / `choose(...)` (CHOOSE-node) expression exists (D9: the
    /// interpreter's bytecode channel evaluates ALL THREE operands eagerly
    /// while rv64 lowers IRIFR|CHOOSE branch-lazily). Value-position `if`
    /// (IRIFR) does NOT set this: both engines are lazy there.
    pub choose: bool,
    /// a `&&`/`||` was emitted inside a condition (cond_expr) — the BUG-014
    /// watch shape: rv64's fault-free-conjunction branch chain (branch_cond
    /// AND arm, gated by is_pure_cond) can SKIP the right operand that the
    /// interpreter evaluates eagerly; is_pure_cond marks comparisons pure
    /// without recursing into faultable children (byte()/div/cast…), so a
    /// consensus-visible trap inside the skipped operand is never raised.
    pub and_chain: bool,
    /// bytes churn loop (D4 family)
    pub churn: bool,
    /// RDROP / CAT byte-slice ops (BUG-009 regression watch shape)
    pub rdrop_cat: bool,
    /// at least one `var` decl was emitted inside a CONDITIONAL arm, so the
    /// slot can be read while never-initialized (D11/BUG-012: interpreter
    /// Nil-fault vs rv64 zero-fill — 0-propagation can redirect control
    /// flow, so ANY interp-fault/rv64-different-fault pairing in such a case
    /// may be this mechanism). NOTE (audit): `note_conditional_decl` is
    /// never called by the current generator, so this flag is always false;
    /// the D11 rows key on the fault MESSAGE/class (Nil / class 91), not on
    /// this flag.
    pub uninit: bool,
    /// P2-C3 B1 (constraint ⑥ unfreeze): a value-position `if` with
    /// BYTES×BYTES arms was emitted (IRIFR bytes arms — the BUG-011-fixed
    /// take_arm_values/join path; verified agreeing incl. banked Ref arms).
    /// The CHOOSE-eager bytes-arm shape (ternary/choose() with bytes arms)
    /// is NOT generated: it miscompiles (BUG-017 candidate, pinned in the
    /// c3 coverage suite) — see rv64_c3_coverage.rs.
    pub bytes_if_arm: bool,
    /// P2-C3 B1: a choose() with MIXED int/bytes arms was emitted — a clean
    /// rv64 compile reject (vstack leaks one entry to function end), handled
    /// as a negative-family channel in run_campaign (interp must accept).
    pub mixed_arm: bool,
    /// P2-C3 B3: this case returns >16 static bytes (bigout family; the
    /// harness packs output_max 64 over a 28 KiB arena).
    pub bigout: bool,
}

#[derive(Debug)]
#[allow(dead_code)] // seed/family/n_stmts/budget: case provenance for the docs
pub struct Case {
    pub seed: u64,
    pub family: &'static str,
    pub body: String,
    pub inputs: Vec<Vec<u8>>,
    pub feat: Feat,
    /// statement count the generator was asked for (pre-downsize)
    pub n_stmts: u64,
    /// depth budget in force for this generation (pre-halving)
    pub budget: GenBudget,
}

/// Halvable depth budgets (the deterministic depth-err retry lever). The
/// lowerer's value-register pool is 8 entries; these caps keep ordinary
/// products inside it and the campaign halves them on the rare reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenBudget {
    pub expr_depth: u64,
    pub cond_depth: u64,
    pub ctrl_depth: u64,
}

impl Default for GenBudget {
    fn default() -> Self {
        GenBudget { expr_depth: 3, cond_depth: 2, ctrl_depth: 3 }
    }
}

impl GenBudget {
    pub fn halve(&mut self) {
        self.expr_depth = (self.expr_depth / 2).max(1);
        self.cond_depth = (self.cond_depth / 2).max(1);
        self.ctrl_depth = (self.ctrl_depth / 2).max(1);
    }
}

/// Generation families. Family is seed-derived: normal 1/7 each — the
/// special families carry the deliberate trap/divergence inducement.
pub const FAMILIES: [&str; 8] = [
    "normal", "trap", "churn", "bigbody", "narrow", "choose", "loopmaze", "bigout",
];

pub struct Gen {
    rng: Rng,
    cov: Coverage,
    feat: Feat,
    u64_vars: Vec<String>,
    bytes_vars: Vec<(String, usize)>, // (name, static length bound)
    counter: u64,
    bcounter: u64,
    family: &'static str,
    narrow_mode: bool,
    budget: GenBudget,
    /// true while generating statements inside a CONDITIONAL arm (stmt_or_block
    /// call from S_if) — decls made there may be skipped at runtime
    in_arm: bool,
    /// pending var declarations to splice BEFORE the statement that made them
    decls: Vec<String>,
    /* ---- P3-V2IMPL: the in-shape list productions ---- */
    /// live list variables (creation is only admitted at ctrl_nest == 0)
    list_vars: Vec<ListVar>,
    /// growable lists declared/upgraded so far — HARD CAP 1: two 1024B
    /// frame blocks would exceed the 2040B locals-frame addi range and the
    /// body would (correctly) reject; the generator stays in-shape
    list_growable: u32,
    /// control-flow nesting depth (if arms + while bodies): list CREATION
    /// productions are gated on zero (plan() rejects conditional list
    /// declarations — the Nil-hazard fail-fast)
    ctrl_nest: u32,
}

/// Generator-side bookkeeping for one live list variable.
struct ListVar {
    name: String,
    /// true once the list is (or will be) APPEND/CLEAR reachable — const
    /// indices are then bounded by 128 (runtime shadow guard); static
    /// lists bound const indices by their literal length
    growable: bool,
    /// an append/clear was ACTUALLY emitted — plan() only upgrades the
    /// storage class on real mutations, so a `[]` that never mutates is
    /// the Empty class (length 0) and every const index would compile-
    /// reject; const reads need `mutated || static_n > 0`
    mutated: bool,
    /// literal element count for non-growable lists (0 for growable)
    static_n: u64,
    /// current runtime length UPPER bound (OOB read induction)
    len_bound: u64,
}

impl Gen {
    fn new(seed: u64, family: &'static str, budget: GenBudget) -> Self {
        Gen {
            rng: Rng::new(seed),
            cov: Coverage::default(),
            feat: Feat::default(),
            u64_vars: vec![],
            bytes_vars: vec![],
            counter: 0,
            bcounter: 0,
            family,
            narrow_mode: family == "narrow",
            budget,
            in_arm: false,
            decls: vec![],
            list_vars: vec![],
            list_growable: 0,
            ctrl_nest: 0,
        }
    }

    /// A fresh variable declaration inside a conditional arm can be SKIPPED
    /// at runtime — the slot is then read while never-initialized (the D11
    /// mechanism). Record it in the case features.
    fn note_conditional_decl(&mut self) {
        if self.in_arm {
            self.feat.uninit = true;
        }
    }

    fn fresh_u64(&mut self) -> String {
        let name = format!("t{}", self.counter);
        self.counter += 1;
        name
    }

    fn fresh_bytes(&mut self, bound: usize) -> String {
        let name = format!("s{}", self.bcounter);
        self.bcounter += 1;
        self.bytes_vars.push((name.clone(), bound));
        name
    }

    /// Pull pending var declarations. Call sites MUST emit these BEFORE the
    /// statement text that requested them (the request happens while
    /// generating that statement's expression text).
    fn drain_decls(&mut self) -> String {
        if self.decls.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for d in self.decls.drain(..) {
            out.push_str(&d);
            out.push('\n');
        }
        out
    }

    /// True when the expression text is a bare `Nu64` literal.
    fn is_pure_lit(e: &str) -> bool {
        let t = e.trim();
        t.ends_with("u64") && t[..t.len() - 3].chars().all(|c| c.is_ascii_digit())
    }

    /// True when the expression text mentions any u64 variable. Add/sub/mul
    /// of two var-free operands folds at the shared front end — and a fold
    /// whose u128 value exceeds u64::MAX is emitted as a CU128 constant,
    /// which rv64 rejects ("outside the translatable kernel subset"). The
    /// generator therefore never builds a var-free add/sub/mul pair.
    fn text_has_var(&self, e: &str) -> bool {
        self.u64_vars.iter().any(|v| e.contains(v.as_str()))
    }

    /* ---------------- expressions ---------------- */

    /// Literal with u64 suffix (constraint ①).
    fn lit_u64(&mut self) -> String {
        self.cov.hit("E_lit_u64");
        match self.rng.below(8) {
            0 => format!("{}u64", self.rng.below(16)),
            1 => format!("{}u64", self.rng.below(256)),
            2 => format!("{}u64", u64::MAX),
            3 => format!("{}u64", u64::MAX - self.rng.below(4)),
            4 => "255u64".to_string(),
            5 => "256u64".to_string(),
            6 => format!("{}u64", 1u64 << self.rng.below(63)),
            _ => format!("{}u64", self.rng.below(1 << 20)),
        }
    }

    /// Depth-0 u64 leaf: variable / input byte / size / literal. Every leaf
    /// is u64-typed (constraint ①).
    fn leaf_u64(&mut self) -> String {
        match self.rng.below(10) {
            0..=3 if !self.u64_vars.is_empty() => {
                self.cov.hit("E_var_u64");
                let i = self.rng.below(self.u64_vars.len() as u64) as usize;
                self.u64_vars[i].clone()
            }
            4 | 5 => {
                self.cov.hit("E_byte");
                let idx = if self.rng.chance(60) {
                    format!("{}", self.rng.below(24))
                } else if !self.u64_vars.is_empty() {
                    // input-derived index: byte-OOB (D2 pinned) becomes likely
                    let v = self.u64_vars[self.rng.below(self.u64_vars.len() as u64) as usize].clone();
                    format!("({v} % 40u64)")
                } else {
                    format!("{}", self.rng.below(24))
                };
                format!("(byte(data, {idx}) as u64)")
            }
            6 if !self.bytes_vars.is_empty() => {
                self.cov.hit("E_size");
                let b = self.bytes_vars[self.rng.below(self.bytes_vars.len() as u64) as usize].0.clone();
                format!("(size({b}) as u64)")
            }
            7 => self.lit_u64(),
            8 => {
                self.cov.hit("E_lit_u64");
                format!("{}u64", self.rng.range(1, 9))
            }
            9 if !self.u64_vars.is_empty() => {
                self.cov.hit("E_var_u64");
                let i = self.rng.below(self.u64_vars.len() as u64) as usize;
                self.u64_vars[i].clone()
            }
            _ => self.lit_u64(),
        }
    }

    /// The deliberate narrow-width shape (D5 family): u8×u8 arithmetic that
    /// the interpreter computes in u8 (fault at ≥ 256) and rv64 in u64.
    fn narrow_arith_shape(&mut self) -> String {
        self.cov.hit("E_narrow_arith");
        let op = self.rng.pick(&["*", "+", "&"]);
        format!("(((byte(data, 0) * byte(data, 1)) {op} 1u8) as u64)")
    }

    /// A u64-valued expression, AST depth ≤ `depth` (hard cap 8, C1 B-row).
    fn expr_u64(&mut self, depth: u64) -> String {
        if depth == 0 {
            return self.leaf_u64();
        }
        // narrow family: sprinkle the D5 shape
        if self.narrow_mode && self.rng.chance(18) {
            return self.narrow_arith_shape();
        }
        // deterministic division-trap leaf in the trap/choose families
        if (self.family == "trap" || self.family == "choose") && self.rng.chance(22) {
            if let Some(z) = self.zero_var() {
                let x = self.leaf_u64();
                let op = self.rng.pick(&["/", "%"]);
                self.cov.hit("E_div_zero");
                return format!("(({x}) {op} ({z}))");
            }
        }
        match self.rng.below(24) {
            0..=5 => {
                let op = self.rng.pick(&["+", "-", "*"]);
                self.cov.hit(match *op {
                    "+" => "E_add",
                    "-" => "E_sub",
                    _ => "E_mul",
                });
                let a = self.expr_u64(depth - 1);
                // snapshot before b: the var-free fold guard below may DROP
                // b's text — same feat/cov rollback as the E_pow/E_add arms
                // (audit invariant feat⇔text; P2-C3 saturation round found
                // this second drop site via audit seed 0xB0DB…+507 after
                // the E_bytes_if_arm rewiring reflowed the corpus)
                let feat_snap = self.feat.clone();
                let cov_snap = self.cov.clone();
                let mut b = self.expr_u64(depth - 1);
                // var-free pairs fold at the shared front end (nested folds
                // included): a fold past u64::MAX becomes a CU128 constant
                // that rv64 rejects — force one runtime operand
                if !self.text_has_var(&a) && !self.text_has_var(&b) {
                    b = self.var_leaf();
                    self.feat = feat_snap;
                    self.cov = cov_snap;
                }
                format!("(({a}) {op} ({b}))")
            }
            6 | 7 => {
                let op = self.rng.pick(&["/", "%"]);
                self.cov.hit(if *op == "/" { "E_div" } else { "E_mod" });
                let a = self.expr_u64(depth - 1);
                let b = if self.family == "trap" && self.rng.chance(50) {
                    self.expr_u64(depth - 1) // may be 0 → trap 90 (agreed row)
                } else {
                    let v = self.expr_u64(depth - 1);
                    format!("(({v}) | 1u64)")
                };
                format!("(({a}) {op} ({b}))")
            }
            8 => {
                // shift: amount sometimes ≥ 64 → D1 pinned row (the
                // interpreter faults 90, rv64 masks &63 and continues —
                // possibly into a LATER trap; the classifier handles both)
                let a = self.expr_u64(depth - 1);
                let amt = if self.rng.chance(35) {
                    self.rng.range(60, 130)
                } else {
                    self.rng.below(64)
                };
                let op = self.rng.pick(&["<<", ">>"]);
                self.cov.hit(if *op == "<<" { "E_shl" } else { "E_shr" });
                // the base is parenthesized: without it `cond ? x : y << 3`
                // parses the shift into the ternary's NO-arm
                format!("(({a}) {op} {amt}u64)")
            }
            9 => {
                let op = self.rng.pick(&["&", "|", "^"]);
                self.cov.hit(match *op {
                    "&" => "E_band",
                    "|" => "E_bor",
                    _ => "E_bxor",
                });
                let a = self.expr_u64(depth - 1);
                let b = self.expr_u64(depth - 1);
                format!("(({a}) {op} ({b}))")
            }
            10 | 11 => {
                let op = self.rng.pick(&["<", ">", "<=", ">=", "==", "!="]);
                self.cov.hit(&format!("E_cmp_{}", op));
                let a = self.expr_u64(depth - 1);
                let b = self.expr_u64(depth - 1);
                format!("((({a}) {op} ({b})) as u64)")
            }
            12 => {
                let op = self.rng.pick(&["&&", "||"]);
                self.cov.hit(if *op == "&&" { "E_and" } else { "E_or" });
                let a = self.cond_expr(depth - 1);
                let b = self.cond_expr(depth - 1);
                format!("((({a}) {op} ({b})) as u64)")
            }
            13 => {
                self.cov.hit("E_not");
                let a = self.cond_expr(depth - 1);
                format!("((!({a})) as u64)")
            }
            14 => {
                let f = self.rng.pick(&["max", "min"]);
                self.cov.hit(if *f == "max" { "E_max" } else { "E_min" });
                let a = self.expr_u64(depth - 1);
                let b = self.expr_u64(depth - 1);
                format!("({f}(({a}), ({b})))")
            }
            15 => {
                // clamp with random (x, lo, hi): lo>hi happens by construction
                // → D10 row (interpreter 90, rv64 silently min/max-chains)
                self.cov.hit("E_clamp");
                let x = self.expr_u64(depth - 1);
                let lo = self.expr_u64(depth - 1);
                let hi = self.expr_u64(depth - 1);
                format!("clamp(({x}), ({lo}), ({hi}))")
            }
            16 => {
                let f = self.rng.pick(&["abs_diff", "div_up", "div_exact_op"]);
                self.cov.hit(match *f {
                    "abs_diff" => "E_absdiff",
                    "div_up" => "E_divup",
                    _ => "E_divexact",
                });
                let a = self.expr_u64(depth - 1);
                let b = if *f == "abs_diff" {
                    self.expr_u64(depth - 1)
                } else {
                    let v = self.expr_u64(depth - 1);
                    format!("(({v}) | 1u64)")
                };
                format!("{f}(({a}), ({b}))")
            }
            17 => {
                // POW with literal exponent ≤ 64 (constraint ③)
                self.cov.hit("E_pow");
                let feat_snap = self.feat.clone();
                let cov_snap = self.cov.clone();
                let mut a = self.expr_u64(depth - 1);
                let e = self.rng.below(65);
                // a var-free base folds (nested folds included): the POW
                // fold can exceed u64::MAX → CU128 constant → rv64 reject.
                // The base is ALSO parenthesized: `cond ? x : lit ** 48`
                // parses the POW into the ternary's NO-arm (a literal-fold
                // that can exceed u128 — the front end rejects outright)
                if e > 1 && !self.text_has_var(&a) {
                    // P2-C3 saturation round: the replacement DROPS the
                    // generated base text — roll back the feat/cov
                    // side-effects that text set (a var-free base can
                    // still carry an E_choose/feat.choose ternary or an
                    // E_bytes_if_arm). Without the rollback the audit
                    // invariant feat⇔text desyncs ("feat.choose but no
                    // CHOOSE shape", audit seeds 0xB0DB…+507/+965 era).
                    // RNG stream unchanged: pure state restore, no draws.
                    a = self.var_leaf();
                    self.feat = feat_snap;
                    self.cov = cov_snap;
                }
                format!("(({a}) ** {e}u64)")
            }
            18 => {
                // narrowing round-trip: out-of-range → runtime 71 on BOTH
                // engines. The operand MUST be a variable: the shared front
                // end folds literal casts at compile time and REJECTS an
                // out-of-range fold (even foldable-literal expressions like
                // `(1u64 << 40u64)`), which is the C1 B-row shape, not a
                // runtime trap.
                self.cov.hit("E_narrow_roundtrip");
                let t = self.rng.pick(&["u8", "u16", "u32"]);
                let a = self.var_leaf();
                format!("((({a}) as {t}) as u64)")
            }
            19 => {
                // value-position CHOOSE: ternary, choose() or value-if.
                // CHOOSE keeps u64 arms in generation (constraint ⑥ as
                // re-frozen by P2-C3: the eager-CHOOSE bytes-arm selection
                // swaps (len, ptr) pairs — BUG-017 candidate, pinned in
                // rv64_c3_coverage.rs; the IRIFR value-if BYTES×BYTES shape
                // IS generated, see E_bytes_if_arm).
                self.cov.hit("E_choose");
                let c = self.cond_expr(depth - 1);
                let (a, b) = if self.family == "choose" && self.rng.chance(60) {
                    // deliberate trapping arm (CHOOSE laziness shape):
                    // one arm divides by the guaranteed-zero var
                    match self.zero_var() {
                        Some(z) => {
                            let x = self.leaf_u64();
                            let trap_arm = format!("({x} / {z})");
                            let ok_arm = self.safe_arm(depth - 1);
                            if self.rng.chance(50) {
                                (trap_arm, ok_arm)
                            } else {
                                (ok_arm, trap_arm)
                            }
                        }
                        None => (self.safe_arm(depth - 1), self.safe_arm(depth - 1)),
                    }
                } else {
                    (self.safe_arm(depth - 1), self.safe_arm(depth - 1))
                };
                match self.rng.below(3) {
                    // ternary and choose() compile to IRNodeTriple{CHOOSE}
                    // (expr.rs:46/51-66): the bytecode channel evaluates all
                    // three operands eagerly (compile.rs:362-365 has no
                    // CHOOSE arm) while rv64 is branch-lazy → the D9 shape.
                    0 | 1 => {
                        self.feat.choose = true;
                        if self.rng.chance(50) {
                            format!("({c}) ? ({a}) : ({b})")
                        } else {
                            format!("choose({c}, {a}, {b})")
                        }
                    }
                    // value-position if is IRIFR (expr.rs:518): lazy on BOTH
                    // engines → NOT a D9 shape (do not set feat.choose).
                    _ => format!("if ({c}) {{ {a} }} else {{ {b} }}"),
                }
            }
            20 => self.narrow_arith_shape(),
            _ => {
                let a = self.expr_u64(depth - 1);
                // snapshot before b: the var-free fold guard below may DROP
                // b's text (see the E_pow rollback note — same audit
                // invariant)
                let feat_snap = self.feat.clone();
                let cov_snap = self.cov.clone();
                let mut b = self.expr_u64(depth - 1);
                if !self.text_has_var(&a) && !self.text_has_var(&b) {
                    b = self.var_leaf();
                    self.feat = feat_snap;
                    self.cov = cov_snap;
                }
                self.cov.hit("E_add");
                format!("({a} + {b})")
            }
        }
    }

    /// A var-only u64 leaf (never a literal) — the overflow guard's escape.
    fn var_leaf(&mut self) -> String {
        if self.u64_vars.is_empty() {
            let name = self.fresh_u64();
            self.u64_vars.push(name.clone());
            self.decls.push(format!("var {name} = 1u64"));
            self.cov.hit("S_decl_u64");
            return name;
        }
        let i = self.rng.below(self.u64_vars.len() as u64) as usize;
        self.u64_vars[i].clone()
    }

    /// CHOOSE/if arms: trap-free in normal families, free in trap families.
    fn safe_arm(&mut self, depth: u64) -> String {
        if self.family == "trap" || self.family == "choose" {
            return self.expr_u64(depth.max(1) - 1);
        }
        match self.rng.below(4) {
            0 => self.leaf_u64(),
            1 => self.lit_u64(),
            2 => {
                let a = self.leaf_u64();
                let mut b = self.leaf_u64();
                // var-free pairs fold (u128 → CU128 → rv64 reject), same
                // guard as the E_add family
                if !self.text_has_var(&a) && !self.text_has_var(&b) {
                    b = self.var_leaf();
                }
                format!("(({a}) + ({b}))")
            }
            _ => {
                let f = self.rng.pick(&["max", "min"]);
                let a = self.leaf_u64();
                let mut b = self.leaf_u64();
                if !self.text_has_var(&a) && !self.text_has_var(&b) {
                    b = self.var_leaf();
                }
                format!("{f}(({a}), ({b}))")
            }
        }
    }

    /// Boolean-valued condition (comparisons/logic only — no bytes values).
    fn cond_expr(&mut self, depth: u64) -> String {
        if depth == 0 {
            let a = self.leaf_u64();
            let b = self.leaf_u64();
            self.cov.hit("E_cmp_eq");
            return format!("(({a}) == ({b}))");
        }
        match self.rng.below(8) {
            0..=4 => {
                let op = self.rng.pick(&["<", ">", "<=", ">=", "==", "!="]);
                self.cov.hit(&format!("E_cmp_{}", op));
                let a = self.expr_u64(depth - 1);
                let b = self.expr_u64(depth - 1);
                format!("(({a}) {op} ({b}))")
            }
            5 => {
                self.cov.hit("E_and");
                self.feat.and_chain = true; // BUG-014 watch shape
                let a = self.cond_expr(depth - 1);
                let b = self.cond_expr(depth - 1);
                format!("({a}) && ({b})")
            }
            6 => {
                self.cov.hit("E_or");
                self.feat.and_chain = true; // BUG-014 watch shape
                let a = self.cond_expr(depth - 1);
                let b = self.cond_expr(depth - 1);
                format!("({a}) || ({b})")
            }
            _ => {
                self.cov.hit("E_not");
                let a = self.cond_expr(depth - 1);
                format!("!({a})")
            }
        }
    }

    /// A bytes expression with a static length bound (≤ 16 keeps returns
    /// inside the harness output_max; the churn tail uses data directly).
    fn expr_bytes(&mut self, depth: u64, max_bound: usize) -> (String, usize) {
        if depth == 0 {
            let n = self.rng.range(1, (max_bound as u64).min(8).max(1));
            self.cov.hit("E_bytes_slice_data");
            return (format!("buf_left({n}, data)"), n as usize);
        }
        // P2-C3: below(9) — arms 0..=6 + 7 (E_bytes_if_arm, the constraint-⑥
        // unfreeze) + `_` = 8 (E_bytes_lit). The predecessor draft used
        // below(8) with the unfreeze arm grafted after `_`: first the 7 arm
        // was dead (unreachable pattern), and after reordering, `_` became
        // dead (below(8) fully covered → E_bytes_lit never fired). Both
        // directions of grafting a new arm onto a saturated match silently
        // kill a production; the grammar-coverage self-report is the check
        // that catches it (E_bytes_lit vanished from the bombard dump).
        match self.rng.below(9) {
            0..=2 => {
                let src = self.pick_bytes_var(max_bound);
                let n = self.rng.range(1, (max_bound as u64).min(8).max(1));
                let op = self.rng.pick(&["buf_left", "buf_right"]);
                self.cov.hit(if *op == "buf_left" { "E_buf_left" } else { "E_buf_right" });
                let bound = src.1.min(max_bound).min(n as usize).max(0);
                (format!("{op}({n}, {})", src.0), bound)
            }
            3 => {
                let src = self.pick_bytes_var(max_bound);
                let n = self.rng.range(0, (max_bound as u64).min(8).max(1));
                let ost = if self.u64_vars.is_empty() || self.rng.chance(50) {
                    format!("{}", self.rng.below(8))
                } else {
                    let v = self.u64_vars[self.rng.below(self.u64_vars.len() as u64) as usize].clone();
                    format!("({v} % 12u64)")
                };
                self.cov.hit("E_buf_cut");
                let bound = src.1.min(max_bound).min(n as usize).max(0);
                (format!("buf_cut({}, {ost}, {n})", src.0), bound)
            }
            4 => {
                let half = (max_bound / 2).max(1);
                let (a, ba) = self.expr_bytes(depth - 1, half);
                let (b, bb) = self.expr_bytes(depth - 1, max_bound.saturating_sub(ba).max(1));
                self.cov.hit("E_cat");
                self.feat.rdrop_cat = true;
                (format!("({a} ++ {b})"), (ba + bb).min(max_bound))
            }
            5 => {
                let src = self.pick_bytes_var(max_bound + 4);
                let n = self.rng.range(0, 4);
                self.cov.hit("E_ldrop");
                ((format!("buf_left_drop({n}, {})", src.0)), src.1.saturating_sub(n as usize))
            }
            6 => {
                let src = self.pick_bytes_var(max_bound + 4);
                let n = self.rng.range(0, 4);
                self.cov.hit("E_rdrop");
                self.feat.rdrop_cat = true;
                ((format!("buf_right_drop({n}, {})", src.0)), src.1.saturating_sub(n as usize))
            }
            7 => {
                // P2-C3 B1 — constraint ⑥ unfreeze: value-position `if`
                // (IRIFR) with BYTES×BYTES arms. Post BUG-011-fix this path
                // materializes Ref arms per-position and joins bottom-first
                // (take_arm_values + no-.rev() re-push); verified agreeing
                // incl. banked Ref arms (c2_probe_bytes_if_arms, ≥200 cases).
                // The arms keep a shared static bound: each sub-arm is
                // budgeted at half the remaining max so the join stays ≤ the
                // caller's envelope (and ≤ output_max for returns).
                // NOTE (P2-C3 saturation round): this arm MUST precede the
                // `_` catch-all — in the predecessor draft it sat AFTER
                // `_`, making it an unreachable pattern (compiler warning
                // c3lib/mod.rs:1240, "no value can reach this") and the
                // production never fired (E_bytes_if_arm absent from every
                // coverage dump despite the probe being green — the probe
                // hand-builds bodies, so it validated the ENGINE path, not
                // the production wiring). The bombard's grammar-coverage
                // self-report is the check that catches this class.
                self.cov.hit("E_bytes_if_arm");
                self.feat.bytes_if_arm = true;
                let c = self.cond_expr(self.budget.cond_depth.min(1));
                let half = (max_bound / 2).max(1);
                let (a, ba) = self.expr_bytes(depth.saturating_sub(1), half);
                let (b, bb) = self.expr_bytes(depth.saturating_sub(1), half);
                let bound = ba.max(bb).min(max_bound);
                (
                    format!("if ({c}) {{ {a} }} else {{ {b} }}"),
                    bound,
                )
            }
            _ => {
                let s = self.rng.range(1, 6);
                let lit: String =
                    (0..s).map(|_| self.rng.range(0x41, 0x5A) as u8 as char).collect();
                self.cov.hit("E_bytes_lit");
                (format!("\"{lit}\""), s as usize)
            }
        }
    }

    fn pick_bytes_var(&mut self, max_bound: usize) -> (String, usize) {
        let cands: Vec<usize> = self
            .bytes_vars
            .iter()
            .enumerate()
            .filter(|(_, (_, b))| *b <= max_bound.max(1) * 2)
            .map(|(i, _)| i)
            .collect();
        if cands.is_empty() {
            let n = self.rng.range(1, (max_bound as u64).min(8).max(1));
            self.cov.hit("E_bytes_slice_data");
            return (format!("buf_left({n}, data)"), n as usize);
        }
        let i = cands[self.rng.below(cands.len() as u64) as usize];
        self.bytes_vars[i].clone()
    }

    /// A var statically known to be 0 (used for deterministic division
    /// traps); creates `zr = c0 - c0` on first use.
    fn zero_var(&mut self) -> Option<String> {
        if self.u64_vars.iter().any(|v| v == "zr") {
            return Some("zr".to_string());
        }
        if self.u64_vars.is_empty() || self.u64_vars.len() >= 24 {
            return None;
        }
        let src = self.u64_vars[0].clone();
        self.u64_vars.push("zr".to_string());
        self.decls.push(format!("var zr = ({src} - {src})"));
        self.cov.hit("S_decl_zero");
        Some("zr".to_string())
    }

    /* ---------------- statements ---------------- */

    /// One statement. `ctrl_left` is the control-flow nesting budget —
    /// DECREMENTED by every if/while/value-if arm (the lowerer's 8-entry
    /// value pool bounds total nesting; C1 B-row).
    ///
    /// The wrapper splices pending var declarations BEFORE the statement
    /// text: expressions request fresh vars while being GENERATED (var_leaf
    /// / zero_var), so the decl text must precede the use text — including
    /// on statement arms that return early (S_early_return).
    ///
    /// Scoping: only decls pushed DURING this statement's generation are
    /// drained (split_off at the entry mark) — decls pending from an outer
    /// context (e.g. an if-condition's zero_var) must stay pending so the
    /// OUTERMOST wrapper places them ahead of the outer statement text
    /// (a nested stmt() draining them would splice the decl INSIDE the
    /// arm, after the outer condition that uses the var — the exact
    /// "cannot find symbol 'zr'" bug this ordering fixed).
    fn stmt(&mut self, ctrl_left: u64) -> String {
        let mark = self.decls.len();
        let s = self.stmt_inner(ctrl_left);
        let mine = self.decls.split_off(mark);
        let mut d = String::new();
        for decl in mine {
            d.push_str(&decl);
            d.push('\n');
        }
        format!("{d}{s}")
    }

    fn stmt_inner(&mut self, ctrl_left: u64) -> String {
        // P3-V2IMPL: arms 26..=30 are the in-shape list productions (the
        // v2 shape set); they fall through to `_` whenever their guards
        // fail (no live list / budget spent / conditional position).
        match self.rng.below(32) {
            0 | 1 => {
                self.cov.hit("S_assign");
                let v = self.pick_or_make_u64();
                let e = self.expr_u64(self.budget.expr_depth);
                format!("{v} = {e}")
            }
            2 => {
                let op = self.rng.pick(&["+=", "-=", "*=", "/="]);
                self.cov.hit(match *op {
                    "+=" => "S_xop_add",
                    "-=" => "S_xop_sub",
                    "*=" => "S_xop_mul",
                    _ => "S_xop_div",
                });
                let v = self.pick_or_make_u64();
                let e = if *op == "/=" && self.family == "trap" && self.rng.chance(40) {
                    self.zero_var()
                        .unwrap_or_else(|| format!("({} | 1u64)", self.expr_u64(2)))
                } else if *op == "/=" {
                    format!("({} | 1u64)", self.expr_u64(2))
                } else {
                    self.expr_u64(self.budget.expr_depth.min(2))
                };
                format!("{v} {op} {e}")
            }
            3 => {
                self.cov.hit("S_decl_u64");
                let name = self.fresh_u64();
                let e = self.expr_u64(self.budget.expr_depth);
                self.u64_vars.push(name.clone());
                format!("var {name} = {e}")
            }
            4 => {
                // declared var is IMMEDIATELY re-widened to u64 (constraint
                // ①): the cast keeps its runtime-71 trap shape without
                // leaving a narrow-typed variable behind (D5 stays confined
                // to the deliberate narrow shapes). Cast operand is a
                // variable (literal casts fold at the shared front end and
                // reject out-of-range — see E_narrow_roundtrip).
                self.cov.hit("S_decl_narrow");
                let name = self.fresh_u64();
                let t = self.rng.pick(&["u8", "u16", "u32"]);
                let e = self.var_leaf();
                self.u64_vars.push(name.clone());
                format!("var {name} = (({e} as {t}) as u64)")
            }
            5 => {
                self.cov.hit("S_decl_bytes");
                let (e, bound) = self.expr_bytes(2, 16);
                let name = self.fresh_bytes(bound);
                format!("var {name} = {e}")
            }
            6 => {
                if self.bytes_vars.is_empty() {
                    return self.stmt(0);
                }
                self.cov.hit("S_assign_bytes");
                let i = self.rng.below(self.bytes_vars.len() as u64) as usize;
                let name = self.bytes_vars[i].0.clone();
                let (e, bound) = self.expr_bytes(2, 16);
                self.bytes_vars[i].1 = bound;
                format!("{name} = {e}")
            }
            7 if ctrl_left > 0 => {
                self.cov.hit("S_if");
                let c = self.cond_expr(self.budget.cond_depth);
                let a = self.stmt_or_block(ctrl_left - 1);
                if self.rng.chance(55) {
                    self.cov.hit("S_else");
                    let b = self.stmt_or_block(ctrl_left - 1);
                    format!("if {c} {{\n{a}\n}} else {{\n{b}\n}}")
                } else {
                    format!("if {c} {{\n{a}\n}}")
                }
            }
            8 => {
                self.cov.hit("S_assign_if_value");
                let v = self.pick_or_make_u64();
                let c = self.cond_expr(self.budget.cond_depth);
                let a = self.safe_arm(2);
                let b = self.safe_arm(2);
                format!("{v} = if ({c}) {{ {a} }} else {{ {b} }}")
            }
            9 if ctrl_left > 0 => self.while_stmt(ctrl_left - 1),
            10 => {
                self.cov.hit("S_require");
                let ev = self.rng.pick(&[7u64, 42, 71, 90, 150, 151, 1000, 5_000_000, u64::MAX]);
                let c = self.cond_expr(self.budget.cond_depth);
                format!("require {ev}u64, {c}")
            }
            11 => {
                self.cov.hit("S_throw");
                let ev = self.rng.pick(&[7u64, 99, 12345, u64::MAX, 90, 71]);
                let c = self.cond_expr(self.budget.cond_depth);
                format!("if {c} {{ throw {ev}u64 }}")
            }
            12 => {
                self.cov.hit("S_assert");
                let c = self.cond_expr(self.budget.cond_depth);
                format!("assert {c}")
            }
            13 => {
                self.cov.hit("S_abort");
                let c = self.cond_expr(self.budget.cond_depth);
                format!("if {c} {{ abort }}")
            }
            14 => {
                // NOTE: a BARE value-expression statement is a known
                // rv64-compile-reject shape (the bytecode machine tolerates a
                // leftover operand-stack value — a fall-off-the-end would even
                // RETURN it — while the rv64 lowerer leaks the vstack and
                // rejects; pinned as P19 in the engine-edges probe). The
                // generator therefore never emits one; this arm mixes into
                // `acc` instead (value consumed by the assignment).
                self.cov.hit("S_expr_stmt");
                let e = self.expr_u64(self.budget.expr_depth.min(2));
                format!("acc = (acc ^ ({e}))")
            }
            15 => {
                self.cov.hit("S_early_return");
                let c = self.cond_expr(self.budget.cond_depth);
                if !self.bytes_vars.is_empty() && self.rng.chance(40) {
                    let cands: Vec<(String, usize)> = self
                        .bytes_vars
                        .iter()
                        .filter(|(_, b)| *b <= 8)
                        .cloned()
                        .collect();
                    if !cands.is_empty() {
                        let b = self.rng.pick(&cands);
                        return format!("if {c} {{ return {} }}", b.0);
                    }
                }
                let e = self.expr_u64(2);
                format!("if {c} {{ return ({e}) as bytes }}")
            }
            16 => {
                self.cov.hit("S_narrow_assign");
                let v = self.pick_or_make_u64();
                let e = self.var_leaf(); // literal casts fold+reject (see above)
                let t = self.rng.pick(&["u8", "u16", "u32"]);
                format!("{v} = (({e} as {t}) as u64)")
            }
            17 if !self.bytes_vars.is_empty() => {
                self.cov.hit("S_byte_extract");
                let b = self.bytes_vars[self.rng.below(self.bytes_vars.len() as u64) as usize]
                    .0
                    .clone();
                let v = self.pick_or_make_u64();
                let idx = if self.rng.chance(50) {
                    format!("{}", self.rng.below(10))
                } else {
                    let u = self.pick_or_make_u64();
                    format!("({u} % 12u64)")
                };
                format!("{v} = (byte({b}, {idx}) as u64)")
            }
            18 if ctrl_left > 0 => {
                self.cov.hit("S_while_nested");
                let outer = self.rng.range(1, 4);
                let inner = self.rng.range(1, 3);
                let oi = self.fresh_u64();
                let ii = self.fresh_u64();
                self.u64_vars.push(oi.clone());
                self.u64_vars.push(ii.clone());
                let mut body = String::new();
                let n = self.rng.range(1, 3);
                self.ctrl_nest += 1; // P3-V2IMPL: list creation gated out
                for _ in 0..n {
                    body.push_str(&self.stmt(0));
                    body.push('\n');
                }
                self.ctrl_nest -= 1;
                if self.rng.chance(50) {
                    self.cov.hit("S_break");
                    body.push_str(&format!("if {} {{ break }}\n", self.cond_expr(1)));
                }
                if self.rng.chance(50) {
                    self.cov.hit("S_continue");
                    body.push_str(&format!(
                        "if {} {{ {ii} += 1\ncontinue }}\n",
                        self.cond_expr(1)
                    ));
                }
                format!(
                    "var {oi} = 0u64\nwhile {oi} < {outer} {{ {oi} += 1\nvar {ii} = 0u64\nwhile {ii} < {inner} {{ {ii} += 1\n{body}}}\n}}"
                )
            }
            19 if !self.bytes_vars.is_empty() => {
                self.cov.hit("S_bytes_arith");
                let b = self.bytes_vars[self.rng.below(self.bytes_vars.len() as u64) as usize]
                    .0
                    .clone();
                let v = self.pick_or_make_u64();
                let e = self.expr_u64(1);
                format!("{v} = {v} + (byte({b}, 0) as u64) + ({e} % 7u64)")
            }
            20 if self.family == "loopmaze" && ctrl_left > 0 => {
                // loopmaze: dense control flow (branch-relaxation pressure)
                self.cov.hit("S_loopmaze_while");
                self.while_stmt(ctrl_left - 1)
            }
            21 if self.family == "choose" && self.rng.chance(12) => {
                // P2-C3 B1 — constraint ⑥ unfreeze, MIXED int/bytes arms:
                // choose() with one u64 and one bytes arm. rv64 cleanly
                // REJECTS the body (the eager-CHOOSE arm count comes from
                // the yes-arm type; the mismatched arm leaks one vstack
                // entry to function end — deterministic text "value stack
                // not empty at function end"). The interpreter accepts and
                // runs (dynamic Values). run_campaign routes these cases
                // through the negative channel: no rv64 knocks, interp is
                // executed on every input, the reject text is asserted.
                // (value-if/ternary mixed arms are NOT generated: they hit
                // the IRIFR debug_assert in debug builds — pinned as a
                // documented engine edge in rv64_c3_coverage.rs instead.)
                self.cov.hit("S_choose_mixed_arm");
                self.feat.mixed_arm = true;
                let c = self.cond_expr(self.budget.cond_depth);
                let int_arm = format!("({})", self.safe_arm(1));
                let (bexpr, _b) = self.expr_bytes(1, 8);
                let name = self.fresh_u64();
                self.u64_vars.push(name.clone());
                // int-first ONLY: choose(c, int, bytes) cleanly rejects
                // (P2-C3FIX-BUG017: "choose arms must have the same result
                // type" — the up-front arm-type check; pre-fix the
                // mismatched arm leaked one vstack entry and the body
                // rejected late with "value stack not empty"). The
                // bytes-first form choose(c, bytes, int) took vals=2 from
                // the yes arm while the no arm pushed one value — the
                // c_reg index underflowed and PANICKED in both profiles
                // (BUG-017 third facet, fixed by the same check); it is
                // pinned FLIPPED in the c3 negative suite, never generated.
                format!("var {name} = choose({c}, {int_arm}, {bexpr})")
            }
            22 if self.family == "bigout" && ctrl_left > 0 => {
                // bigout: statements build wider bytes values toward the
                // 17..=64 B return channel (tail in program()); bodies stay
                // ordinary so the four-way knock exercises the wide-output
                // copy path over real control flow.
                self.cov.hit("S_bigout_decl");
                let (e, bound) = self.expr_bytes(2, 48);
                let name = self.fresh_bytes(bound);
                format!("var {name} = {e}")
            }
            // ---- P3-V2IMPL: in-shape list productions (v2 shape set) ----
            // creation: literal table (1..=4 u64 elements; all-literal →
            // rodata class, computed element → frame-static) or the empty
            // growable form. ONLY at ctrl_nest == 0 — plan() rejects
            // conditional list declarations. One growable max (frame
            // budget), ≤4 lists per body.
            26 if self.ctrl_nest == 0
                && !self.in_arm
                && self.list_vars.len() < 4
                && (self.list_growable == 0 || self.rng.chance(70)) =>
            {
                self.cov.hit("S_list_decl");
                let name = format!("l{}", self.list_vars.len());
                let empty_ok = self.list_growable == 0;
                if self.rng.chance(35) && empty_ok {
                    self.list_growable += 1;
                    self.list_vars.push(ListVar {
                        name: name.clone(),
                        growable: true,
                        mutated: false,
                        static_n: 0,
                        len_bound: 0,
                    });
                    format!("var {name} = []")
                } else {
                    let n = self.rng.range(1, 4);
                    let mut elems = Vec::new();
                    let mut all_lit = true;
                    for k in 0..n {
                        if self.rng.chance(55) {
                            elems.push(format!("{}u64", self.rng.below(1000)));
                        } else {
                            // computed element → frame-static class
                            all_lit = false;
                            elems.push(format!(
                                "((byte(data, {}) as u64) + {}u64)",
                                k,
                                self.rng.below(16)
                            ));
                        }
                    }
                    let _ = all_lit; // (rodata vs frame-static is the lowerer's call)
                    self.list_vars.push(ListVar {
                        name: name.clone(),
                        growable: false,
                        mutated: false,
                        static_n: n as u64,
                        len_bound: n as u64,
                    });
                    format!("var {name} = [{}]", elems.join(", "))
                }
            }
            // append: statement-position mutation (admitted under any
            // control flow). Appending a static list upgrades it to the
            // growable class — same 1-growable frame budget.
            27 if !self.list_vars.is_empty() => {
                let i = self.rng.below(self.list_vars.len() as u64) as usize;
                let lv = &mut self.list_vars[i];
                if !lv.growable {
                    if self.list_growable > 0 {
                        return self.stmt_inner(ctrl_left);
                    }
                    lv.growable = true;
                    self.list_growable += 1;
                }
                if lv.len_bound >= 120 {
                    return self.stmt_inner(ctrl_left); // stay off the 40 cap in the corpus
                }
                self.cov.hit("S_list_append");
                let name = lv.name.clone();
                lv.mutated = true;
                lv.len_bound += 1;
                // BUG-029 FIXUP: the appended value must be DEFINITE — an
                // expr_u64 var expression can carry a may-Nil slot
                // (declared under a conditional) and the append
                // compile-rejects may-Nil values now (the interpreter
                // would silently store the Nil; the u64 frame block cannot
                // represent it — doc 98 §E4). Pure input-byte arithmetic
                // is definite on every corpus body (the S_pair_fuse
                // narrowing precedent).
                let b = self.rng.below(8);
                let e = format!(
                    "((byte(data, {b}) as u64) + {}u64)",
                    self.rng.below(1000)
                );
                format!("append({name}, {e})")
            }
            // read: constant or variable index (OOB values induce the
            // AgreeTrap85 family; constant indices stay inside the
            // compile-checked bound for static lists)
            28 if !self.list_vars.is_empty() => {
                self.cov.hit("S_list_read");
                let v = self.pick_or_make_u64();
                let i = self.rng.below(self.list_vars.len() as u64) as usize;
                let lv = &self.list_vars[i];
                let idx = if self.rng.chance(50) || (!lv.mutated && lv.static_n == 0) {
                    // variable index: mixes in/out of bounds (and the only
                    // admissible form for a never-mutated empty list —
                    // every const index on length 0 compile-rejects)
                    let m = (lv.len_bound.max(1) + self.rng.below(3)) as u64;
                    format!("((byte(data, 0) as u64) % {m}u64)")
                } else if lv.mutated {
                    // runtime shadow guard: any const < 128 compiles
                    format!("{}u64", self.rng.below(120))
                } else {
                    // static list: const must stay under the literal length
                    format!("{}u64", self.rng.below(lv.static_n.max(1)))
                };
                format!("{v} = {name}[{idx}]", name = lv.name)
            }
            // folds: length / has_key with the u64 width discipline
            29 if !self.list_vars.is_empty() => {
                self.cov.hit("S_list_folds");
                let v = self.pick_or_make_u64();
                let lv = &self.list_vars[self.rng.below(self.list_vars.len() as u64) as usize];
                let m = (lv.len_bound.max(1) + 2) as u64;
                let form = self.rng.below(3);
                let stmt = match form {
                    0 => format!("{v} = ((length({})) as u64)", lv.name),
                    1 => format!(
                        "{v} = ((has_key({}, ((byte(data, 0) as u64) % {m}u64))) as u64)",
                        lv.name
                    ),
                    _ => format!(
                        "{v} = {v} + (((length({})) as u64) * 10u64) + (((has_key({}, 0u64)) as u64))",
                        lv.name, lv.name
                    ),
                };
                stmt
            }
            // clear: length shadow ← 0 (later reads/OOBs agree); only for
            // growable lists — clear forces the growable class
            30 if self.list_vars.iter().any(|l| l.growable) => {
                self.cov.hit("S_list_clear");
                let grow: Vec<usize> = self
                    .list_vars
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.growable)
                    .map(|(i, _)| i)
                    .collect();
                let i = *self.rng.pick(&grow);
                self.list_vars[i].len_bound = 0;
                self.list_vars[i].mutated = true;
                format!("clear({})", self.list_vars[i].name)
            }
            // ---- P3-IDIOMS M3-b: fused byte-assembly groups ----
            // arm 31 was the dead below(32) slot (fell through to `_`);
            // grafting here keeps every existing arm's probability identical.
            // Two bounded BE-2/LE-2 forms over constant offsets — the shapes
            // the rv64 wide-load fusion recognizer admits (doc 94). The
            // single span guard replaces both byte guards with identical
            // observable behavior (rv64 71; interp D2 pinned rows unchanged),
            // so these bodies land in the agree/pinned families like every
            // other byte() statement.
            31 => {
                self.cov.hit("S_byte_fuse");
                let a = self.rng.below(6);
                let name = self.fresh_u64();
                self.u64_vars.push(name.clone());
                if self.rng.chance(50) {
                    // BE-2 (the hacd200 :629 mul-256+add shape)
                    format!(
                        "var {name} = (((byte(data, {a}) as u64) * 256u64) + (byte(data, {}) as u64)) as u64",
                        a + 1
                    )
                } else {
                    // LE-2 single-expression OR tree (the keccak chain head shape)
                    format!(
                        "var {name} = ((byte(data, {a}) as u64) | ((byte(data, {}) as u64) << 8u64))",
                        a + 1
                    )
                }
            }
            // ---- P3-PAIRPKG M6-b: the remove+insert pair production ----
            // arm 23 was a dead below(32) slot (23|24|25 fell through to
            // `_`); grafting here keeps every existing arm's probability
            // identical (the M3-b dead-arm technique). The emitted
            // two-statement pair satisfies the plan() recognizer by
            // construction: identical index text both sides (structural
            // equality), the index/value expressions draw from u64 vars and
            // the input only (never the receiver — the shifted-view
            // discipline), and the value is a definite u64 expression (no
            // may-Nil face). A static list touched by a pair upgrades to
            // the growable class (same 1-growable frame budget as append).
            23 if !self.list_vars.is_empty() => {
                let i = self.rng.below(self.list_vars.len() as u64) as usize;
                let lv = &mut self.list_vars[i];
                if !lv.growable {
                    if self.list_growable > 0 {
                        return self.stmt_inner(ctrl_left);
                    }
                    lv.growable = true;
                    self.list_growable += 1;
                }
                self.cov.hit("S_pair_fuse");
                let name = lv.name.clone();
                lv.mutated = true;
                // identical index text on both sides: variable form mixes
                // in/out-of-bounds (len_bound + 0..2), const form stays
                // under the runtime-guard bound (any k < 128 compiles on a
                // growable — the BUG-028 form)
                let idx = if self.rng.chance(50) {
                    let m = (lv.len_bound.max(1) + self.rng.below(3)) as u64;
                    format!("((byte(data, 0) as u64) % {m}u64)")
                } else {
                    let k = self.rng.below((lv.len_bound.max(1) + 2).min(120) as u64);
                    format!("{k}u64")
                };
                // value: pure input-byte arithmetic ONLY — a u64-var
                // expression could carry a may-Nil slot (declared under a
                // conditional) and the pair compile-rejects may-Nil values
                // (the B-family face doc 98 pins); byte(data, k) is
                // definite on every corpus body
                let a = self.rng.below(6);
                let val = format!(
                    "(((byte(data, {a}) as u64) * 3u64) + {}u64)",
                    self.rng.below(1000)
                );
                format!("remove({name}, {idx})\ninsert({name}, {idx}, {val})")
            }
            _ => {
                self.cov.hit("S_assign");
                let v = self.pick_or_make_u64();
                let e = self.expr_u64(self.budget.expr_depth);
                format!("{v} = {e}")
            }
        }
    }

    fn stmt_or_block(&mut self, ctrl_left: u64) -> String {
        let n = self.rng.range(1, 3);
        let mut out = String::new();
        self.ctrl_nest += 1; // P3-V2IMPL: list creation gated out of arms
        for _ in 0..n {
            out.push_str(&self.stmt(ctrl_left));
            out.push('\n');
        }
        self.ctrl_nest -= 1;
        out
    }

    fn pick_or_make_u64(&mut self) -> String {
        if self.u64_vars.is_empty() || self.rng.chance(15) {
            let name = self.fresh_u64();
            self.u64_vars.push(name.clone());
            self.decls.push(format!("var {name} = 0u64"));
            self.cov.hit("S_decl_u64");
            return name;
        }
        let i = self.rng.below(self.u64_vars.len() as u64) as usize;
        self.u64_vars[i].clone()
    }

    fn while_stmt(&mut self, ctrl_left: u64) -> String {
        self.cov.hit("S_while");
        let trips = self.rng.range(1, 16);
        let iv = self.fresh_u64();
        self.u64_vars.push(iv.clone());
        let mut body = String::new();
        let n = self.rng.range(1, 4);
        self.ctrl_nest += 1; // P3-V2IMPL: list creation gated out of bodies
        for _ in 0..n {
            body.push_str(&self.stmt(ctrl_left));
            body.push('\n');
        }
        self.ctrl_nest -= 1;
        // break/continue AFTER the top increment → termination is structural
        if self.rng.chance(45) {
            self.cov.hit("S_break");
            body.push_str(&format!("if {} {{ break }}\n", self.cond_expr(1)));
        }
        if self.rng.chance(45) {
            self.cov.hit("S_continue");
            body.push_str(&format!("if {} {{ continue }}\n", self.cond_expr(1)));
        }
        format!("var {iv} = 0u64\nwhile {iv} < {trips} {{ {iv} += 1\n{body}}}")
    }

    /* ---------------- program ---------------- */

    fn program(&mut self, n_stmts: u64) -> String {
        let mut out = String::new();
        // input decoders: 2-3 u64 words + bounded bytes views
        let words = self.rng.range(2, 3);
        for w in 0..words {
            let name = format!("c{w}");
            let off = w * 8;
            out.push_str(&format!(
                "var {name} = 0u64\nvar k{w} = 0u64\nwhile k{w} < 8 {{ {name} = {name} * 256 + (byte(data, k{w} + {off}) as u64)\nk{w} += 1 }}\n"
            ));
            self.u64_vars.push(name);
        }
        if self.rng.chance(60) {
            out.push_str("var c9 = (byte(data, 0) as u64)\n");
            self.u64_vars.push("c9".to_string());
        }
        let bslices = self.rng.range(1, 2);
        for _ in 0..bslices {
            let (e, bound) = self.expr_bytes(1, 8);
            let name = self.fresh_bytes(bound);
            out.push_str(&format!("var {name} = {e}\n"));
        }
        out.push_str("var acc = 0u64\n");
        self.u64_vars.push("acc".to_string());
        for _ in 0..n_stmts {
            // stmt() splices its own pending decls ahead of the statement
            // text (a fresh var requested DURING generation must be declared
            // before that statement — the front end resolves at parse time)
            out.push_str(&self.stmt(self.budget.ctrl_depth));
            out.push('\n');
        }
        match self.family {
            "bigout" => {
                // P2-C3 B3: a >16 B static-length return (17..=64). The
                // harness packs output_max 64 over a 28 KiB arena for this
                // family; the interpreter has no such limit below its 1280 B
                // value cap. The static bound arithmetic mirrors expr_bytes
                // (cat bounds add, slices min) so the four engines must
                // agree byte-for-byte on the returned content.
                self.cov.hit("F_bigout");
                self.feat.bigout = true;
                self.feat.rdrop_cat = true;
                let n = self.rng.range(17, 57); // head size
                let m = self.rng.range(0, 64 - n.min(56)); // tail up to 64
                let tail = if self.rng.chance(50) {
                    format!("buf_left({m}, data)")
                } else {
                    format!("\"{}\"", (0..m).map(|_| 'K').collect::<String>())
                };
                out.push_str(&format!(
                    "return (buf_left({n}, data) ++ {tail})\n"
                ));
            }
            "churn" => {
                self.cov.hit("F_churn");
                self.feat.churn = true;
                self.feat.rdrop_cat = true;
                let iters = self.rng.pick(&[1u64, 3, 8, 12, 30, 60]);
                out.push_str(&format!(
                    "var cb = buf_left(200, data)\nvar ci = 0u64\nwhile ci < {iters} {{ cb = cb ++ buf_left(200, data)\nci += 1 }}\nreturn (size(cb) as u64) as bytes\n"
                ));
            }
            "bigbody" => {
                // long flat if-chain inside a loop: drives the branch
                // relaxation ladder (c.beqz ±256B window → B-type → jal →
                // trampoline) and mirrors >4.7KiB bodies; the campaign's
                // mirror-overflow retry downsizes the surrounding statements
                // if the whole image ever exceeds 16383 B
                self.cov.hit("F_bigbody");
                // chain length scales with n_stmts so the campaign's
                // mirror-overflow retry (halving n_stmts) genuinely shrinks
                // the chain; 10-stmt default → 150..=700 branches (the upper
                // end crosses the ±256 B branch window and approaches the
                // 16383 B mirror)
                let jitter = self.rng.range(0, 550);
                let n = 150 + jitter * n_stmts / 10;
                let mut chain = String::new();
                for k in 0..n {
                    chain.push_str(&format!(
                        "if (byte(data, 0) as u64) != {k}u64 {{ acc += {}u64 }}\n",
                        k % 97
                    ));
                }
                out.push_str(&format!(
                    "var bi = 0u64\nwhile bi < 3 {{ bi += 1\n{chain}}}\nreturn acc as bytes\n"
                ));
            }
            _ => match self.rng.below(10) {
                1..=2 => {
                    self.cov.hit("S_return_bytes");
                    let cands: Vec<(String, usize)> = self
                        .bytes_vars
                        .iter()
                        .filter(|(_, b)| *b <= 8)
                        .cloned()
                        .collect();
                    if cands.is_empty() {
                        out.push_str("return (acc) as bytes\n");
                    } else {
                        let b = self.rng.pick(&cands);
                        out.push_str(&format!("return {}\n", b.0));
                    }
                }
                3 => {
                    self.cov.hit("S_return_size");
                    if !self.bytes_vars.is_empty() {
                        let b =
                            self.bytes_vars[self.rng.below(self.bytes_vars.len() as u64) as usize]
                                .0
                                .clone();
                        out.push_str(&format!("return (size({b}) as u64) as bytes\n"));
                    } else {
                        out.push_str("return (acc) as bytes\n");
                    }
                }
                4 => {
                    self.cov.hit("S_fallthrough_end");
                }
                _ => out.push_str("return (acc) as bytes\n"),
            },
        }
        out
    }
}

/// Generate one case: family by seed, body by (seed, budget), inputs by seed.
/// Deterministic: same seed + budget → identical body and identical inputs.
pub fn generate_case(seed: u64, n_stmts: u64, budget: GenBudget) -> Case {
    let fam_idx = Rng::new(seed ^ 0xF00D).below(FAMILIES.len() as u64) as usize;
    let family = FAMILIES[fam_idx];
    let mut g = Gen::new(seed, family, budget);
    let body = g.program(n_stmts);
    let inputs = inputs_for(seed, family);
    Case { seed, family, body, inputs, feat: g.feat, n_stmts, budget }
}

/// Seed-derived input set: always ≥ 6, always covering the boundaries
/// (empty / 1 B / zero words / ramp / 0xFF / 256 B) plus family extras
/// (1280 B envelope, random words, distinctive loopmaze words).
pub fn inputs_for(seed: u64, family: &str) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(seed ^ 0x1A9707);
    let mut v: Vec<Vec<u8>> = Vec::new();
    v.push(vec![]); // empty → byte(data,0) OOB (D2 pinned rows)
    v.push(vec![0u8]); // 1 B
    v.push(vec![0u8; 16]); // zero words → c0 = c1 = 0 (division traps)
    v.push((0..16u8).collect()); // ramp
    v.push(vec![0xFFu8; 32]);
    let mut p256 = vec![0u8; 256];
    for (i, b) in p256.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    v.push(p256);
    if family == "churn" || family == "normal" {
        v.push(vec![0x61u8; 1280]); // input envelope max
    }
    if family == "trap" || family == "choose" || family == "narrow" {
        let mut r = vec![0u8; 24];
        for b in r.iter_mut() {
            *b = (rng.next() & 0xFF) as u8;
        }
        v.push(r);
    }
    if family == "loopmaze" || family == "bigbody" {
        v.push(vec![42u8, 0, 1, 255, 7, 7, 7, 9]);
    }
    if family == "bigout" {
        // exactly-64 B and 65 B inputs: the return channel's arena-copy
        // boundary (returns slice data[0..n] ++ tail; 64 B covers the max
        // head, 65 B leaves one byte past every head size)
        v.push((0u8..64).collect());
        v.push((0u8..65).collect());
    }
    v
}

/* ================================================================== */
/* Section 4: the classifier (C1 mapping table A-rows + B-rows + the   */
/* D9/D10 rows added by this round).                                   */
/* ================================================================== */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    AgreeOk,
    AgreeTrap90,
    AgreeTrap71,
    AgreeUserValue(u64),
    /// P2-C3 BUG-019 (FIXED by P2-C3FIX 2026-11-15; found by the feedback
    /// saturation corpus 2026-11-15): interpreter Ok, ALL THREE rv64 modes
    /// trapped 71, body contains an eager CHOOSE. Mechanism (decode-listing
    /// verified): the CHOOSE arm's nil-tag copies were gated `if my || mn`
    /// — when only ONE arm had nil machinery, the OTHER arm's tag byte at
    /// its vstack depth was never written by its evaluation and held a
    /// stale zero ("Nil") from the prologue zero-fill; the copy propagated
    /// it to the result position and the consumer's Nil guard faulted 71
    /// on a live value. Corpus-reachable (loopmaze family). FIX (rv64.rs
    /// CHOOSE arm): the no-path copy is gated on `mn` and the yes-path
    /// copy on `my`, and a mechanism-less arm publishes a non-Nil tag (1)
    /// on its select path (the complementary mn-only/taken-yes quadrant
    /// diverged too). The row name is KEPT as a REGRESSION WATCH: post-fix
    /// the signature (feat.choose ∧ interp-Ok ∧ rv64-trap) must never
    /// occur — assert_fixed_rows_zero asserts it zero, so a recurrence is
    /// loud. Pin flipped in
    /// rv64_c3_coverage.rs::c3_pin_bug019_choose_stale_nil_tag.
    PinBug019ChooseStaleNilTag,
    /// P2-C4 S5 explicit cross-mode audit row: the three rv64 modes
    /// disagree among THEMSELVES (any pair differs in output/trap —
    /// engine-vs-engine, no interpreter ambiguity involved). Previously
    /// this surfaced as Unexpected("mode-divergence") (the check runs
    /// FIRST in classify, before any pinned row, so a split could never
    /// be absorbed by a pinned verdict); P2-C4 promotes it to its OWN
    /// audit row + assert_fixed_rows_zero zero-watch so the mass runs
    /// report engine splits as a first-class count. Post-BUG-009 this
    /// must never fire.
    ModeSplit,
    /// interpreter faults 90 at a shift ≥ 64; rv64 masks &63 and continues
    /// (may Ok, or hit ANY later trap — the trajectory legitimately differs)
    PinD1ShiftMask,
    /// byte/buf index-slice OOB: interpreter StackError(42)/CastParamFail(72)
    /// vs rv64 CastFail(71) at the same site (full-u64 check)
    PinD2BoundsClass,
    /// assert failure / guarded abort: interpreter ThrowAbort(151)+Nil vs
    /// rv64 exit 1
    PinD3AssertAbort,
    /// bytes churn past the interpreter's 1280 B value cap: 39 vs whatever
    /// the rv64 churn bound does next (71 at the arena end, or Ok inside it)
    PinD4ChurnBand,
    /// narrow-width arithmetic (u8×u8 etc): the interpreter computes in the
    /// operands' common narrow width (fault payload carries U8(/U16(/U32()
    /// while rv64 computes in u64 — any later rv64 outcome is expected
    PinD5NarrowWidth,
    /// value-position CHOOSE arms: the interpreter's bytecode evaluates ALL
    /// THREE operands eagerly (`IRNodeTriple::codegen_into` pushes
    /// subx/suby/subz, node.rs) while rv64 lowers IRIFR/CHOOSE branch-lazily
    /// (rv64.rs IRIFR|CHOOSE arm). An interpreter fault inside an untaken arm
    /// is skipped by rv64 — which may then Ok or hit any LATER trap.
    PinD9ChooseLazy,
    /// uninit BYTES-slot use agreeing on both engines (BUG-012 fixed):
    /// interpreter BytesHandle(91) = rv64 guard fault 91
    AgreeTrap91,
    /// P2-C3: bare-Nil int params of bytes-family ops agree at 72 on both
    /// engines (generator-unreachable — offsets are always arithmetic-
    /// wrapped, which faults 71 first; the C3 directed probes reach it:
    /// canbe.rs extract_uint_cast vs the rv64 param-Nil guard)
    AgreeTrap72,
    /// P2-C3: require/throw VALUE = Nil agrees at 76 on both engines
    /// (generator-unreachable — values are always literals; reached by the
    /// C3 directed probes: validate_error_scalar vs the rv64 76 guard)
    AgreeTrap76,
    /// P3-V2IMPL: compo-list item_get OOB agrees at CompoNoFindItem(85) on
    /// both engines (value/list.rs:40 ↔ the runtime length guard's 85
    /// thunk) — list.rs reads through extract_u32 first, so idx ≥ 2³²
    /// surfaces as AgreeTrap72 above
    AgreeTrap85,
    /// P3-V2IMPL: append past the 128-item cap agrees at OutOfCompoLen(40)
    /// (compo.rs:164-165 ↔ the len*8 ≥ 1024 guard's 40 thunk)
    AgreeTrap40,
    /// clamp(x, lo, hi) with lo > hi: interpreter Arithmetic(90)
    /// ("clamp check failed", instruction.rs clamp_checked) vs rv64's
    /// unchecked min/max chain (silently returns max(min(x,hi),lo)).
    /// Candidate backend conformance gap (BUG-010 candidate).
    PinD10ClampLoHi,
    /// uninitialized-slot read: the consensus interpreter keeps HVM slots as
    /// Values — a slot whose declaring `var` statement never executed reads
    /// as Nil, and any use faults — INT uses fault Arithmetic(90)
    /// ("cannot … U64(n) and Nil"); BYTES-family uses fault BytesHandle(91)
    /// (canbe.rs:125-133: `Nil → itr_err_code!(BytesHandle)`, reached from
    /// operand.rs cutbyte/cutleft/cutright/cutout/dropleft/dropright/
    /// concat) — while the rv64 backend zero-fills frame slots in the
    /// prologue (rv64.rs needs_zero machinery), so the same body reads
    /// 0 / empty bytes and continues (or traps 71 at the byte op: the
    /// D11-bytes row discovered by the first bombardment).
    /// Candidate backend conformance gap (BUG-012 candidate, both flavors).
    PinD11UninitNil,
    /// BUG-014 candidate watch row: an `&&` (or `||`) inside a condition —
    /// rv64's fault-free-conjunction branch chain (branch_cond AND arm,
    /// gated by is_pure_cond, rv64.rs:4126-4132) skipped the operand whose
    /// evaluation faulted the interpreter (is_pure_cond marks comparisons
    /// pure without recursing into faultable children). The interpreter's
    /// bytecode channel is eager for AND/OR (compile.rs:64-68), so the
    /// consensus-visible trap never fires on rv64, which continues (to Ok
    /// or any later trap). Expected ONLY while BUG-014 is unfixed; each
    /// hit must be minimized and its mechanism confirmed (audit discipline
    /// as for D9).
    PinAndChainSkippedTrap,
    /// Bool-in-uint coercion (D5 width-model family): the interpreter's
    /// dynamic Value model rejects Bool operands in uint contexts
    /// (`to_uint` → CastFail(71) "cannot cast Bool(..) to uint"), while
    /// rv64's uniform-u64 model treats bools as 0/1 ints and continues
    /// (e.g. `x + (!cmp)` as a ternary condition). Same family as
    /// narrow-width arithmetic: a pinned model divergence, not a bug.
    PinD5BoolUint,
    Budget,
    Unexpected(&'static str),
}

impl Verdict {
    pub fn is_unexpected(&self) -> bool {
        matches!(self, Verdict::Unexpected(_))
    }
}

/// Extract the require/throw USER VALUE from a fault message. The HVM
/// renders scalar values with their width suffix (`require failed: 42u64`),
/// so the suffix must be stripped before taking the trailing digit-run
/// (taking it raw would return 64 — the draft's exact bug).
pub fn trailing_number(msg: &str) -> Option<u64> {
    let mut s = msg;
    for suf in ["u64", "u32", "u16", "u8"] {
        if s.ends_with(suf) {
            s = &s[..s.len() - suf.len()];
            break;
        }
    }
    let last_run = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .next_back()?;
    last_run.parse().ok()
}

/// The interpreter's narrow-width payloads print as `U8(..)` / `U16(..)` /
/// `U32(..)`; a u64 payload prints `U64(..)`.
pub fn narrow_payload(msg: &str) -> bool {
    msg.contains("U8(") || msg.contains("U16(") || msg.contains("U32(")
}

pub fn same_outcome(a: &Result<Vec<u8>, u64>, b: &Result<Vec<u8>, u64>) -> bool {
    match (a, b) {
        (Ok(x), Ok(y)) => x == y,
        (Err(x), Err(y)) => x == y,
        _ => false,
    }
}

pub fn classify(
    feat: &Feat,
    interp: &Result<Vec<u8>, InterpFault>,
    rv: &[(Rv64Mode, Result<Vec<u8>, u64>)],
) -> Verdict {
    // 0. the three rv64 modes must agree among themselves (engine-vs-engine;
    // no interpreter ambiguity involved — always a finding)
    let modes_agree = rv.iter().all(|(_, r)| same_outcome(&rv[0].1, r));
    if !modes_agree {
        // BUG-009 (RDROP pointer) pre-fix shape — post-fix this must never
        // fire; if it does, the BUG-009 regression is back.
        if feat.rdrop_cat {
            if let (Err(c1), Err(c2)) = (rv[0].1.clone(), rv[1].1.clone()) {
                if c1 == 0 && c2 == 0 && rv[2].1.is_ok() {
                    return Verdict::Unexpected("bug009-regressed: Im/Imc Err(0), ImcRemap Ok");
                }
            }
        }
        // P2-C4 S5: explicit audit row (was Unexpected("mode-divergence"))
        return Verdict::ModeSplit;
    }
    match interp {
        Ok(ibytes) => match &rv[0].1 {
            Ok(rbytes) => {
                if rbytes == ibytes {
                    Verdict::AgreeOk
                } else {
                    Verdict::Unexpected("output-mismatch")
                }
            }
            Err(0) => Verdict::Unexpected("rv64-engine-trap(interp-ok)"),
            Err(_) => {
                // BUG-019 family (see the Verdict doc): an interpreter-Ok
                // body whose rv64 triple INVENTS a 71 trap, in a CHOOSE
                // body. FIXED by P2-C3FIX — the row must stay at ZERO
                // (assert_fixed_rows_zero regression watch); any hit is a
                // recurrence and gets minimized immediately. Non-choose
                // bodies keep the Unexpected row.
                if feat.choose {
                    Verdict::PinBug019ChooseStaleNilTag
                } else {
                    Verdict::Unexpected("rv64-trap(interp-ok)")
                }
            }
        },
        Err(ifault) => {
            if ifault.msg.contains("out of gas") {
                return Verdict::Budget;
            }
            let c = ifault.code;
            let nar = narrow_payload(&ifault.msg);
            // --- interpreter FAULT-SITE divergences: rv64 does not fault at
            // that site and CONTINUES; it may then produce Ok OR hit any
            // later trap (its trajectory legitimately diverges after the
            // masked/lazy/silent site). These rows accept any rv64 outcome.
            if c == 90 && ifault.msg.contains("shift overflow") {
                // narrow-width shift (u8 <<) is D5; u64 shift ≥ 64 is D1
                return if nar { Verdict::PinD5NarrowWidth } else { Verdict::PinD1ShiftMask };
            }
            if c == 90 && ifault.msg.contains("clamp check failed") {
                // clamp lo>hi: post-BUG-010-fix rv64 traps 90 at the same
                // site — agree; any other rv64 outcome is a regression
                // (watch row, must stay ZERO in campaigns)
                return match &rv[0].1 {
                    Err(90) => Verdict::AgreeTrap90,
                    _ => Verdict::PinD10ClampLoHi,
                };
            }
            if c != 151 && ifault.msg.contains("Nil") {
                // use of a never-executed `var` slot (Nil-use). Post
                // BUG-012 fix: rv64's Nil guards fault with the
                // interpreter's exact class at the same site — agree rows;
                // any other rv64 outcome is a regression (the row stays as
                // a watch and must be ZERO in every campaign)
                return match &rv[0].1 {
                    Err(90) if c == 90 => Verdict::AgreeTrap90,
                    Err(71) if c == 71 => Verdict::AgreeTrap71,
                    Err(91) if c == 91 => Verdict::AgreeTrap91,
                    // P2-C3: bare-Nil int params (72) and Nil require/throw
                    // values (76) agree at the same class (directed probes)
                    Err(72) if c == 72 => Verdict::AgreeTrap72,
                    Err(76) if c == 76 => Verdict::AgreeTrap76,
                    _ => Verdict::PinD11UninitNil,
                };
            }
            if c == 91 {
                // D11 BYTES flavor: bytes-family op on a never-executed
                // `var` slot — interpreter extract_bytes(Nil) →
                // BytesHandle(91) (canbe.rs:125-133). Post-fix rv64 faults
                // 91 at the same site; any other outcome is a regression
                // (watch row, must stay ZERO in campaigns)
                return match &rv[0].1 {
                    Err(91) => Verdict::AgreeTrap91,
                    _ => Verdict::PinD11UninitNil,
                };
            }
            if c == 90 && nar {
                return Verdict::PinD5NarrowWidth;
            }
            if c == 71 && ifault.msg.contains("cannot cast Bool") {
                // D5 width-model family: Bool rejected by the interpreter's
                // to_uint, coerced to 0/1 by rv64's uniform-u64 model —
                // rv64 may then produce ANY outcome
                return Verdict::PinD5BoolUint;
            }
            if c == 39 {
                // P2-C4 S2 (10^5 run): the D4 mechanism — the interpreter's
                // 1280 B value cap vs rv64's ~40 KiB bump arena — is
                // body-shape-INDEPENDENT: any bytes-growing loop can reach
                // it, not just the churn family's own shape (the old
                // feat.churn gate's premise, falsified by seeds
                // 0xB0DB…+69804/+57821/+80831 whose grow-loops live in
                // other families). The PinD4ChurnBand contract already
                // covers every rv64 outcome ("71 at the arena end, or Ok
                // inside it"), so the gate is dropped and the row stays a
                // pinned-divergence watch.
                return Verdict::PinD4ChurnBand;
            }
            match &rv[0].1 {
                Ok(_) => {
                    // interpreter faults, rv64 continued to success
                    if feat.choose {
                        // the fault fired inside an arm rv64 never executed
                        // (CHOOSE laziness, D9)
                        return Verdict::PinD9ChooseLazy;
                    }
                    if feat.and_chain {
                        // BUG-014 candidate: the fault fired inside an AND/OR
                        // operand that rv64's fault-free-conjunction branch
                        // chain skipped (is_pure_cond unsound for comparisons
                        // with faultable children)
                        return Verdict::PinAndChainSkippedTrap;
                    }
                    Verdict::Unexpected("interp-fault-rv64-ok")
                }
                Err(x) => {
                    // both engines fault at the SAME bytes-bounds site with
                    // the mapped classes (42/72 vs 71) — an agree-family row
                    // regardless of any choose/and_chain feature
                    if matches!((c, *x), (42, 71) | (72, 71)) {
                        return Verdict::PinD2BoundsClass;
                    }
                    // assert/abort (D3): interpreter 151-with-Nil vs exit 1
                    if c == 151 && *x == 1 && ifault.msg.contains("nil") {
                        return Verdict::PinD3AssertAbort;
                    }
                    // require/throw user values agree on the extracted value
                    if matches!(c, 150 | 151) {
                        return match trailing_number(&ifault.msg) {
                            Some(t) if t == *x => Verdict::AgreeUserValue(*x),
                            _ => Verdict::Unexpected("user-value-mismatch"),
                        };
                    }
                    // CHOOSE-lazy / BUG-014 AND-chain watch rows: a fault
                    // rv64 skipped or translated to another class. Post
                    // D9-fix / BUG-014-fix the same-site same-class pairs
                    // (90,90)/(71,71)/(91,91) are plain agreements — these
                    // rows stay as regression watches and must be ZERO in
                    // every campaign.
                    let agree_pair = matches!(
                        (c, *x),
                        (90, 90) | (71, 71) | (91, 91) | (72, 72) | (76, 76) | (85, 85) | (40, 40)
                    );
                    if !agree_pair && feat.choose {
                        return Verdict::PinD9ChooseLazy;
                    }
                    if !agree_pair && feat.and_chain {
                        return Verdict::PinAndChainSkippedTrap;
                    }
                    match (c, *x) {
                        (90, 90) => Verdict::AgreeTrap90,
                        (71, 71) => Verdict::AgreeTrap71,
                        (91, 91) => Verdict::AgreeTrap91,
                        (72, 72) => Verdict::AgreeTrap72,
                        // P3-V2IMPL: the list trap family (85 OOB / 40 cap)
                        (85, 85) => Verdict::AgreeTrap85,
                        (40, 40) => Verdict::AgreeTrap40,
                        (76, 76) => Verdict::AgreeTrap76,
                        _ => Verdict::Unexpected("trap-class-mismatch"),
                    }
                }
            }
        }
    }
}

/* ================================================================== */
/* Section 5: knock driver + stats + minimizer.                        */
/* ================================================================== */

#[derive(Default)]
pub struct CampaignStats {
    pub cases: u64,
    pub knocks: u64,
    pub frontend_rejects: u64,
    pub rv64_rejects_unexpected: u64,
    /// BUG-013 candidate rejects: "value stack not empty at function end" —
    /// the self-referencing XOP vstack leak (ImcRemap only). Pinned, not
    /// unexpected; every occurrence's body must contain a self-ref XOP
    /// (`v op= v`) shape (audit discipline, see 70-c2-generator.md §5).
    pub bug013_rejects: u64,
    /// P2-C3: choose() with int/bytes mixed arms — clean rv64 compile
    /// reject (P2-C3FIX-BUG017: the up-front arm-type check; pre-fix it
    /// was a late "value stack not empty" vstack leak reject); pinned
    /// negative channel
    pub mixed_arm_rejects: u64,
    /// P2-C4 S6: the documented CU128 kernel-subset boundary (doc 30
    /// mapping table + g5_u128_rejected_rv64_compile_only): the SHARED
    /// frontend constant-folds past u64 (e.g. `2u64 + (u64::MAX |
    /// (u64::MAX-1))` → 2^64+1, runtime 90-trap swallowed by the fold),
    /// the interpreter renders the u128 value 16B BE, and the rv64 kernel
    /// (64-bit) rejects all three modes with "opcode CU128 is outside the
    /// translatable kernel subset". Endorsed compile-only family — counted,
    /// NOT unexpected (first surfaced naturally by the S1 10^4 run,
    /// seed 0xB0DB…+5417).
    pub kernel_subset_rejects: u64,
    pub downsized: u64,
    pub depth_downsized: u64,
    pub budget: u64,
    pub verdicts: BTreeMap<String, u64>,
    /// (seed, verdict/reason, FAILING input bytes, n_stmts, depth budget in
    /// force) — the generation knobs and the failing input MUST be replayed
    /// to regenerate and re-observe the exact failure (the campaign
    /// downsizes both knobs deterministically per seed)
    pub unexpected: Vec<(u64, String, Vec<u8>, u64, GenBudget)>,
}

impl CampaignStats {
    /// Rows that must be ZERO after the P2-F2 fixes (BUG-010/011/012/013/
    /// 014 + D9-fix): each is a regression watch. The pinned model-diff
    /// rows (D1/D2/D3/D4/D5 family, incl. bool-uint coercion) legitimately
    /// stay non-zero.
    pub fn assert_fixed_rows_zero(&self, tier: &str) {
        for row in [
            "pin_bug014_and_chain_skip",
            "pin_D9_choose_lazy",
            "pin_D10_clamp_lo_hi",
            "pin_D11_uninit_nil",
            // P2-C3FIX-BUG019: the stale-nil-tag CHOOSE signature is
            // FIXED — the row is a regression watch now (zero post-fix)
            "pin_bug019_choose_stale_nil_tag",
            // P2-C4 S5: explicit engine-vs-engine split audit row (was
            // Unexpected("mode-divergence")) — zero always
            "mode_split",
        ] {
            let n = self.verdicts.get(row).copied().unwrap_or(0);
            assert_eq!(n, 0, "{tier}: {row} must be zero post-P2-F2 fixes (regression)");
        }
        assert_eq!(
            self.bug013_rejects, 0,
            "{tier}: BUG-013 self-ref XOP rejects must be zero post-fix"
        );
        assert_eq!(
            self.rv64_rejects_unexpected, 0,
            "{tier}: rv64 rejects outside the pinned channels must be zero"
        );
        assert_eq!(
            self.frontend_rejects, 0,
            "{tier}: shared frontend must never reject generated bodies"
        );
        assert!(
            self.unexpected.is_empty(),
            "{tier}: unexpected findings must be zero (minimizer list): {:?}",
            self.unexpected.iter().take(3).collect::<Vec<_>>()
        );
    }

    pub fn note(&mut self, v: &Verdict) {
        let k = match v {
            Verdict::AgreeOk => "agree_ok".to_string(),
            Verdict::AgreeTrap90 => "agree_trap_90".to_string(),
            Verdict::AgreeTrap71 => "agree_trap_71".to_string(),
            Verdict::AgreeTrap91 => "agree_trap_91".to_string(),
            Verdict::AgreeTrap72 => "agree_trap_72".to_string(),
            Verdict::AgreeTrap76 => "agree_trap_76".to_string(),
            Verdict::AgreeTrap85 => "agree_trap_85".to_string(),
            Verdict::AgreeTrap40 => "agree_trap_40".to_string(),
            Verdict::AgreeUserValue(v) => format!("agree_user_value({v})"),
            Verdict::PinBug019ChooseStaleNilTag => "pin_bug019_choose_stale_nil_tag".to_string(),
            Verdict::ModeSplit => "mode_split".to_string(),
            Verdict::PinD1ShiftMask => "pin_D1_shift_mask".to_string(),
            Verdict::PinD2BoundsClass => "pin_D2_bounds".to_string(),
            Verdict::PinD3AssertAbort => "pin_D3_assert_abort".to_string(),
            Verdict::PinD4ChurnBand => "pin_D4_churn_band".to_string(),
            Verdict::PinD5NarrowWidth => "pin_D5_narrow_width".to_string(),
            Verdict::PinD5BoolUint => "pin_D5_bool_uint".to_string(),
            Verdict::PinD9ChooseLazy => "pin_D9_choose_lazy".to_string(),
            Verdict::PinD10ClampLoHi => "pin_D10_clamp_lo_hi".to_string(),
            Verdict::PinD11UninitNil => "pin_D11_uninit_nil".to_string(),
            Verdict::PinAndChainSkippedTrap => "pin_bug014_and_chain_skip".to_string(),
            Verdict::Budget => "budget".to_string(),
            Verdict::Unexpected(r) => format!("UNEXPECTED({r})"),
        };
        *self.verdicts.entry(k).or_insert(0) += 1;
    }
}

/// Run one (case, input) through both engines and classify.
pub fn knock_one(rv: &Rv64, it: &Interp, feat: &Feat, input: &[u8]) -> Verdict {
    let ir = it.run(input);
    let mut rr = rv.run(input);
    // budget retry: rv64 out-of-cycles shows as Err(0) with Ok interp
    if ir.is_ok() && rr.iter().any(|(_, r)| matches!(r, Err(0))) {
        rr = rv.run_with_limits(
            input,
            vm::calc::CalcRunLimits { cycles: RV64_CYCLES_RETRY, resource: -1, deadline: None },
        );
    }
    classify(feat, &ir, &rr)
}

/// Generate-compile-knock loop over a seed range. Compile rejects are data:
/// mirror-overflow and depth-err trigger DETERMINISTIC downsizing retries
/// (halve statements / halve depth budgets — both pure functions of the
/// seed); anything else is counted and reported (must stay zero).
pub fn run_campaign(
    n_cases: u64,
    seed0: u64,
    n_stmts: u64,
    verbose: bool,
    minimize_unexpected: usize,
) -> CampaignStats {
    let mut stats = CampaignStats::default();
    for i in 0..n_cases {
        let seed = seed0.wrapping_add(i);
        let mut n = n_stmts;
        let mut budget = GenBudget::default();
        let mut tries = 0u64;
        let good = loop {
            let case = generate_case(seed, n, budget);
            // P2-C3 B1: mixed-arm choose cases are a NEGATIVE channel — the
            // interpreter must accept (and is run on every input); rv64 must
            // reject all three modes with the pinned text; no rv64 knocks.
            if case.feat.mixed_arm {
                let it = match Interp::try_build(&case.body) {
                    Ok(it) => it,
                    Err(e) => {
                        stats.frontend_rejects += 1;
                        stats.unexpected.push((
                            seed,
                            format!("mixed-arm frontend-reject: {e}"),
                            Vec::new(),
                            n,
                            budget,
                        ));
                        break None;
                    }
                };
                match Rv64::try_build(&case.body) {
                    // the mixed-arm vstack leak surfaces as one of THREE
                    // deterministic reject texts depending on where the
                    // choose sits and what follows it (P2-C3 bombard round,
                    // seeds 0xB0DB…+500/+1447): (a) "value stack not empty"
                    // at function end, (b) "leaves values on the stack"
                    // (statement-position depth_err), or (c) "while loop in
                    // expression position" — the leaked entry makes the
                    // NEXT statement's IRWHILE illegal (rv64.rs IRWHILE arm
                    // rejects on non-empty vstack). All three are the same
                    // leak; (c) is the cascade face, still a hard reject.
                    Err(e) if e.contains("choose arms must have the same result type")
                        // P2-C3FIX-BUG017: the up-front arm-type reject; the
                        // three leak texts below are the PRE-fix faces
                        // (defensive — a regression would land here)
                        || e.contains("value stack not empty")
                        || e.contains("leaves values on the stack")
                        || e.contains("while loop in expression position") =>
                    {
                        // run the interpreter over the full input set — the
                        // accepted-side obligation of the two-way assertion
                        for input in case.inputs.iter() {
                            match it.run(input) {
                                Ok(_) | Err(_) => {} // any outcome proves acceptance
                            }
                        }
                        stats.mixed_arm_rejects += 1;
                    }
                    // P2-C4 S2: post-BUG-021 a DEEP mixed-arm body can hit
                    // the 8-value-pool depth_err before any pinned text —
                    // same knob-driven shape as the main loop's depth
                    // retry: halve and regenerate (the case stays
                    // mixed_arm; the arm-type face reappears at a shallower
                    // depth). Seed 0xB0DB…+78219 (shard 2) was the first.
                    Err(e) if e.contains("nesting exceeds") => {
                        tries += 1;
                        if tries > 6 {
                            stats.rv64_rejects_unexpected += 1;
                            stats.unexpected.push((
                                seed,
                                format!("mixed-arm depth stuck: {e}"),
                                Vec::new(),
                                n,
                                budget,
                            ));
                            println!(
                                "[case {seed}] mixed-arm body still over depth after halving budgets"
                            );
                            break None;
                        }
                        budget.halve();
                        continue;
                    }
                    Err(e) => {
                        stats.rv64_rejects_unexpected += 1;
                        stats.unexpected.push((
                            seed,
                            format!("mixed-arm reject text changed: {e}"),
                            Vec::new(),
                            n,
                            budget,
                        ));
                        break None;
                    }
                    Ok(_) => {
                        stats.rv64_rejects_unexpected += 1;
                        stats.unexpected.push((
                            seed,
                            "mixed-arm case LOWERED (expected clean reject)".to_string(),
                            Vec::new(),
                            n,
                            budget,
                        ));
                        break None;
                    }
                }
                stats.cases += 1;
                break None; // negative channel: no knocks
            }
            // P2-C3 B3: bigout cases pack the wide output envelope
            let build = if case.family == "bigout" {
                Rv64::try_build_out(&case.body, 64, 0x7000)
            } else {
                Rv64::try_build(&case.body)
            };
            let rb = build;
            let ib = Interp::try_build(&case.body);
            match (rb, ib) {
                (Ok(rv), Ok(it)) => break Some((rv, it, case)),
                (_, Err(e)) => {
                    // the SHARED front end rejected → generator bug
                    // (checked before the rv64 arms: both channels fail
                    // together on a frontend reject)
                    stats.frontend_rejects += 1;
                    stats.unexpected.push((
                        seed,
                        format!("frontend-reject: {e}"),
                        Vec::new(),
                        n,
                        budget,
                    ));
                    println!("[case {seed}] frontend reject: {first_line}\n{}\n", case.body, first_line = first_line(&e));
                    break None;
                }
                (Err(e), _) if e.contains("calcfunc size overflow") => {
                    tries += 1;
                    stats.downsized += 1;
                    if tries > 6 {
                        println!("[case {seed}] still over mirror budget after downsizing");
                        break None;
                    }
                    n = (n / 2).max(2);
                }
                (Err(e), _) if e.contains("nesting exceeds") => {
                    tries += 1;
                    stats.depth_downsized += 1;
                    if tries > 6 {
                        println!("[case {seed}] still over depth after halving budgets");
                        break None;
                    }
                    budget.halve();
                }
                (Err(e), _) if e.contains("value stack not empty") => {
                    // BUG-013 candidate (pinned-unfixed): the self-referencing
                    // XOP vstack leak rejects every ImcRemap body containing
                    // `v op= v`. Pinned, counted, NOT unexpected. The leak
                    // always persists to function end, so this reject can
                    // never be skipped by downsizing — do not retry.
                    stats.bug013_rejects += 1;
                    println!(
                        "[case {seed}] BUG-013 candidate reject (value stack not empty; \
                         body must contain a self-ref XOP — audit):\n{}\n",
                        case.body
                    );
                    break None;
                }
                (Err(e), _)
                    if e.contains("outside the translatable kernel subset") =>
                {
                    // P2-C4 S6: the CU128 kernel-subset boundary (doc 30
                    // mapping table, g5_u128_rejected_rv64_compile_only):
                    // the shared frontend folds u64 arithmetic past u64
                    // (runtime-trap-swallowing fold), the interpreter
                    // renders the u128 value 16B BE, and the 64-bit rv64
                    // kernel rejects — the documented compile-only family.
                    // Pinned, counted, NOT unexpected; no downsizing retry
                    // (the fold is a property of the body text, not of the
                    // knobs).
                    stats.kernel_subset_rejects += 1;
                    println!(
                        "[case {seed}] CU128 kernel-subset reject (documented compile-only family):\n{}\n",
                        case.body
                    );
                    break None;
                }
                (Err(e), _) => {
                    stats.rv64_rejects_unexpected += 1;
                    stats.unexpected.push((
                        seed,
                        format!("rv64-reject: {e}"),
                        Vec::new(),
                        n,
                        budget,
                    ));
                    println!("[case {seed}] rv64 reject: {first_line}\n{}\n", case.body, first_line = first_line(&e));
                    break None;
                }
            }
        };
        if let Some((rv, it, case)) = good {
            stats.cases += 1;
            for (idx, input) in case.inputs.iter().enumerate() {
                let v = knock_one(&rv, &it, &case.feat, input);
                if matches!(v, Verdict::Budget) {
                    stats.budget += 1;
                }
                if let Verdict::Unexpected(reason) = &v {
                    stats.unexpected.push((
                        seed,
                        format!("{v:?} (input#{idx}, {} B)", input.len()),
                        input.clone(),
                        case.n_stmts,
                        case.budget,
                    ));
                    println!(
                        "[case {seed}] UNEXPECTED {reason} (input#{idx}, {} B):\n{}\n--- input: {input:?}\n",
                        input.len(),
                        case.body
                    );
                }
                stats.note(&v);
                stats.knocks += 1;
            }
        }
        if verbose && (i < 5 || i % 200 == 0) {
            println!("[case {i} seed {seed}] progress: knocks {}", stats.knocks);
        }
    }
    // minimization of the first unexpected cases
    if minimize_unexpected > 0 {
        let mut seen = std::collections::HashSet::new();
        let mut shown = 0;
        let list = stats.unexpected.clone();
        for (seed, reason, input, n, budget) in list {
            if shown >= minimize_unexpected {
                break;
            }
            if !seen.insert(seed) {
                continue;
            }
            shown += 1;
            println!("\n=== UNEXPECTED seed {seed}: {reason} — minimizing ===");
            match minimize_case(seed, n, budget, &input) {
                Some((body, outcome)) => {
                    match &outcome {
                        MinOutcome::Knock(v, obs) => {
                            println!("--- minimal body ({v:?}):\n{body}\n--- observation: {obs}");
                        }
                        o => {
                            println!("--- minimal body ({o:?}):\n{body}");
                        }
                    }
                }
                None => println!("(minimizer could not reproduce)"),
            }
        }
    }
    stats
}

/// Minimization outcome: either a knock verdict (with the raw engine
/// observation) or the compile-reject text (build rejects are findings too
/// — the minimizer must report them).
#[derive(Debug, Clone)]
pub enum MinOutcome {
    Knock(Verdict, String),
    Rv64Reject(String),
    FrontendReject(String),
}

/// Comparable signature of a failure: minimization only accepts trials that
/// preserve the SAME failure (any failure is not enough — deleting a prelude
/// line would otherwise "reproduce" as a symbol error).
pub fn outcome_key(o: &MinOutcome) -> String {
    match o {
        MinOutcome::Knock(v, _) => format!("{v:?}"),
        MinOutcome::Rv64Reject(e) => format!("rx:{}", first_line_of(e)),
        MinOutcome::FrontendReject(e) => format!("fr:{}", first_line_of(e)),
    }
}

pub fn first_line_of(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// Statement-window delta-debugging: delete statement lines while the SAME
/// failure survives; return the minimal repro. The generation knobs
/// (n_stmts, budget) and the failing input must be the ones in force for
/// the failure — the campaign downsizes the knobs deterministically and
/// the mismatch may be input-specific.
pub fn minimize_case(
    seed: u64,
    n_stmts: u64,
    budget: GenBudget,
    input: &[u8],
) -> Option<(String, MinOutcome)> {
    let case = generate_case(seed, n_stmts, budget);
    let mut lines: Vec<String> = case.body.lines().map(|s| s.to_string()).collect();

    fn outcome_of(body: &str, input: &[u8], feat: &Feat) -> Option<MinOutcome> {
        match (Rv64::try_build(body), Interp::try_build(body)) {
            (Ok(rv), Ok(it)) => {
                let ir = it.run(input);
                let mut rr = rv.run(input);
                if ir.is_ok() && rr.iter().any(|(_, r)| matches!(r, Err(0))) {
                    rr = rv.run_with_limits(
                        input,
                        vm::calc::CalcRunLimits {
                            cycles: RV64_CYCLES_RETRY,
                            resource: -1,
                            deadline: None,
                        },
                    );
                }
                let v = classify(feat, &ir, &rr);
                if !v.is_unexpected() {
                    return None;
                }
                Some(MinOutcome::Knock(v, fmt_outcome(&(ir, rr))))
            }
            (Err(e), Err(_)) => Some(MinOutcome::FrontendReject(e)),
            (Err(e), _) => Some(MinOutcome::Rv64Reject(e)),
            (_, Err(e)) => Some(MinOutcome::FrontendReject(e)),
        }
    }

    // the original failure must reproduce before bisection starts
    let want = outcome_of(&case.body, input, &case.feat)?;
    let want_key = outcome_key(&want);
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < lines.len() {
            for win in [6usize, 3, 1] {
                if i + win <= lines.len() {
                    let trial: Vec<String> = lines
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j < i || *j >= i + win)
                        .map(|(_, s)| s.clone())
                        .collect();
                    if let Some(o) = outcome_of(&trial.join("\n"), input, &case.feat) {
                        if outcome_key(&o) == want_key {
                            lines = trial;
                            changed = true;
                            break;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    let body = lines.join("\n");
    let out = outcome_of(&body, input, &case.feat)?;
    (outcome_key(&out) == want_key).then(|| (body, out))
}

/* ================================================================== */
/* Section 6: probes — engine edges observed by reading + running.     */
/* ================================================================== */

pub fn fmt_outcome(
    o: &(Result<Vec<u8>, InterpFault>, Vec<(Rv64Mode, Result<Vec<u8>, u64>)>),
) -> String {
    let (i, r) = o;
    let i_s = match i {
        Ok(b) => format!("Ok({b:?})"),
        Err(f) => format!("Err({}: {})", f.code, f.msg),
    };
    let r_s = r
        .iter()
        .map(|(m, x)| match x {
            Ok(b) => format!("{m:?}=Ok({b:?})"),
            Err(c) => format!("{m:?}=Err({c})"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("interp {i_s} | rv64 {r_s}")
}

pub fn probe(body: &str, input: &[u8]) -> String {
    let it = Interp::try_build(body).expect("probe interp build");
    let rv = Rv64::try_build(body).expect("probe rv64 build");
    fmt_outcome(&(it.run(input), rv.run(input)))
}

/// Two decoded input words prelude (c0 from bytes 0..8, c1 from 8..16).
pub fn dec2() -> String {
    "var c0 = 0u64\nvar i = 0u64\nwhile i < 8 { c0 = c0 * 256 + (byte(data, i) as u64)\ni += 1 }\nvar c1 = 0u64\nvar j = 0u64\nwhile j < 8 { c1 = c1 * 256 + (byte(data, j + 8) as u64)\nj += 1 }\n".to_string()
}

/// One 8-byte BE input word decoder prelude for var `name` at offset `off`.
pub fn word_dec(name: &str, off: u64) -> String {
    format!(
        "var {name} = 0u64\nvar i = 0u64\nwhile i < 8 {{ {name} = {name} * 256 + (byte(data, i + {off}) as u64)\ni += 1 }}\n"
    )
}


pub fn print_coverage(cov: &Coverage, title: &str) {
    println!("--- grammar coverage ({title}): {} productions", cov.0.len());
    for (k, v) in cov.0.iter() {
        println!("    {k}: {v}");
    }
}

/// Generation coverage of `n` cases (generation is pure → identical to what
/// the campaign ran).
pub fn coverage_of(n_cases: u64, seed0: u64, n_stmts: u64, budget: GenBudget) -> Coverage {
    let mut cov = Coverage::default();
    for i in 0..n_cases {
        let seed = seed0.wrapping_add(i);
        let fam_idx = Rng::new(seed ^ 0xF00D).below(FAMILIES.len() as u64) as usize;
        let mut g = Gen::new(seed, FAMILIES[fam_idx], budget);
        let _ = g.program(n_stmts);
        cov.merge(&g.cov);
    }
    cov
}

pub fn campaign_with_coverage(
    n_cases: u64,
    seed0: u64,
    n_stmts: u64,
    verbose: bool,
    minimize: usize,
) -> (CampaignStats, Coverage) {
    let cov = coverage_of(n_cases, seed0, n_stmts, GenBudget::default());
    let stats = run_campaign(n_cases, seed0, n_stmts, verbose, minimize);
    (stats, cov)
}

pub fn env_u64(name: &str, dflt: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(dflt)
}
/// First line of an error message (shared with the test tiers).
pub fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// P2-C3: the negative-suite case tables, factored out so the C3 coverage
/// binary measures the EXACT bodies the c2 negative suites assert on
/// (single source of truth; the c2 tests call these).
pub fn frontend_negative_cases() -> Vec<(&'static str, String)> {
    vec![
("truncated-var", "var x =".into()),
        ("var-name-integer", "var 7 = 3".into()),
        (
            "unclosed-brace",
            "var x = 1u64\nif (x == 1u64) {\nreturn x as bytes".into(),
        ),
        (
            "as-missing-type",
            "var x = 1u64\nvar y = (x) as\nreturn 0u64 as bytes".into(),
        ),
        ("require-no-cond", "require 5u64\nreturn 0u64 as bytes".into()),
        ("prefix-operator", "** 3".into()),
        ("bind-no-name", "bind = 3".into()),
        ("break-outside-loop", "break".into()),
        ("continue-outside-loop", "continue".into()),
        (
            "literal-overflows-u8",
            "var x = 1000u8\nreturn 0u64 as bytes".into(),
        ),
        // P2-C3 B2: literal beyond u64::MAX is a FRONT-END reject (both
        // channels) — the u128 family's only non-CU128 member.
        (
            "literal-overflows-u64",
            "var x = 18446744073709551616u64\nreturn 0u64 as bytes".into(),
        ),
        ("bytecode-unknown-op", "bytecode { TOTALLY_BOGUS_OP }".into()),
        (
            "cast-to-src-form",
            "var a = (byte(data, 0) as u64)\nvar r = cast_to(a, u8)\nreturn (r as u64) as bytes".into(),
        ),
        (
            "unknown-function",
            "var x = frobnicate(1u64)\nreturn 0u64 as bytes".into(),
        ),
        (
            "if-expr-missing-else",
            "var x = 1u64\nvar y = if (x == 1u64) { 2u64 }\nreturn 0u64 as bytes".into(),
        ),
        // macro misuse family
        (
            "macro-unknown-invocation",
            "var x = nosuchmacro(1u64)\nreturn 0u64 as bytes".into(),
        ),
        (
            "macro-unterminated-invocation",
            "var x = 1u64\nvar y = (x + nosuchmacro(2u64\nreturn 0u64 as bytes".into(),
        ),
    ]
}

/// rv64-only reject cases: (name, body, required error fragments, interp-runs).
pub fn rv64_negative_cases() -> Vec<(&'static str, String, Vec<&'static str>, bool)> {
    vec![
(
            "depth-9",
            format!(
                "{}var v8 = (byte(data, 8) as u64)\nvar r = (v0 + (v1 + (v2 + (v3 + (v4 + (v5 + (v6 + (v7 + v8))))))))\nreturn r as bytes",
                (0..8)
                    .map(|k| format!("var v{k} = (byte(data, {k}) as u64)\n"))
                    .collect::<String>()
            ),
            vec!["nesting exceeds"],
            true,
        ),
        (
            "u128-cast",
            "var a = (byte(data, 0) as u64)\nvar r = (a as u128)\nreturn (r as bytes)".into(),
            vec!["CU128"],
            false, // 16 B output > harness output_max; only the reject matters
        ),
        // P2-C3 B2: the u128 reject FAMILY (doc 70 follow-up 3) — every shape the
        // shared front end accepts reaches rv64 lowering as a CU128 constant
        // or cast node and must be rejected naming CU128.
        (
            "u128-fold-add",
            "var a = (byte(data, 0) as u64)\nvar r = (a + (18446744073709551615u64 + 1u64))\nreturn (r as u64) as bytes".into(),
            vec!["CU128"],
            false, // interpreter faults 71 on the u128→u64 return cast
        ),
        (
            "u128-fold-mul",
            "var a = (byte(data, 0) as u64)\nvar r = (a + (4294967296u64 * 4294967296u64))\nreturn (r as u64) as bytes".into(),
            vec!["CU128"],
            false,
        ),
        (
            "u128-fold-pow",
            "var a = (byte(data, 0) as u64)\nvar r = (a + (2u64 ** 64u64))\nreturn (r as u64) as bytes".into(),
            vec!["CU128"],
            false,
        ),
        (
            "u128-nested-arith",
            "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = (((a as u128) + (b as u128)) as u64)\nreturn (r) as bytes".into(),
            vec!["CU128"],
            true,
        ),
        (
            "u128-compare",
            "var a = (byte(data, 0) as u64)\nvar r = (((a as u128) > 5u128) as u64)\nreturn (r) as bytes".into(),
            vec!["CU128"],
            true,
        ),
        (
            "u128-literal-suffix",
            "var r = 170141183460469231731687303715884105727u128\nreturn (r as u64) as bytes".into(),
            vec!["CU128"],
            false,
        ),
        (
            "u128-shift",
            "var a = (byte(data, 0) as u64)\nvar r = (((a as u128) << 70u128) as u64)\nreturn (r) as bytes".into(),
            vec!["CU128"],
            false,
        ),
        // P2-C3 B1: choose() with MIXED int/bytes arms — the generator's
        // negative-channel shape (S_choose_mixed_arm). Interpreter accepts &
        // runs; rv64 cleanly rejects with the pinned text.
        (
            "choose-mixed-arm",
            "var c = (byte(data, 0) as u64)\nvar q = choose(((c % 2u64) == 0u64), 1u64, buf_left(3, data))\nreturn (c) as bytes".into(),
            // P2-C3FIX-BUG017: the up-front arm-type reject (pre-fix the
            // mismatched arm leaked a vstack entry → "value stack not empty")
            vec!["choose arms must have the same result type"],
            true,
        ),
        (
            "mul-mod",
            "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_mod(a, b, c)\nreturn (r as u64) as bytes".into(),
            vec!["MULMOD"],
            true,
        ),
        (
            "add-mod",
            "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = add_mod(a, b, c)\nreturn (r as u64) as bytes".into(),
            vec!["ADDMOD"],
            true,
        ),
        (
            "mul-add",
            "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_add(a, b, c)\nreturn (r as u64) as bytes".into(),
            vec!["MULADD"],
            true,
        ),
        (
            "mul-div",
            "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_div(a, b, c)\nreturn (r as u64) as bytes".into(),
            vec!["MULDIV"],
            true,
        ),
        (
            "type-id",
            "var a = (byte(data, 0) as u64)\nvar r = type_id(a)\nreturn (r as u64) as bytes".into(),
            vec!["TID"],
            true,
        ),
        ("log-op", "log(data, 1)\nreturn 0u64 as bytes".into(), vec!["LOG"], true),
        ("nil-value", "var n = nil\nreturn 0u64 as bytes".into(), vec!["PNIL"], true),
        // P3-V2IMPL (A-25 M1): in-shape list literals LOWER now (creation /
        // append / item_get / clear / length / has_key). The old single
        // "list-literal" reject pin is re-semanticized as the ESCAPE
        // family: every shape where a list value flows out of the admitted
        // ops rejects, naming the consuming opcode. Interpreter behavior
        // is asserted per row (accepts and runs unless noted).
        (
            "list-literal-alias",
            "var l = [1u64, 2u64]\nvar m = l\nreturn 0u64 as bytes".into(),
            vec!["PUT", "outside the translatable kernel subset"],
            true,
        ),
        (
            "list-literal-rebind",
            "var l = [1u64, 2u64]\nl = [3u64]\nreturn 0u64 as bytes".into(),
            vec!["PUT", "outside the translatable kernel subset"],
            true,
        ),
        (
            "list-literal-nested",
            "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes".into(),
            vec!["PACKLIST", "outside the translatable kernel subset"],
            true, // interpreter faults CastBeValueFail(76) at runtime
        ),
        (
            "list-literal-eq",
            "var l = [1u64, 2u64]\nvar r = ((l == l) as u64)\nreturn r as bytes".into(),
            vec!["EQ", "outside the translatable kernel subset"],
            true,
        ),
        (
            "list-literal-ordered-cmp",
            "var l = [1u64, 2u64]\nvar r = ((l < l) as u64)\nreturn r as bytes".into(),
            vec!["LT", "outside the translatable kernel subset"],
            true, // interpreter faults Arithmetic(90) at runtime
        ),
        (
            "list-literal-return",
            "var l = [1u64, 2u64]\nreturn l".into(),
            vec!["RET", "outside the translatable kernel subset"],
            false, // returns a Compo — only the reject matters
        ),
        (
            "list-literal-cast",
            "var l = [1u64, 2u64]\nvar r = (l as bytes)\nreturn r".into(),
            vec!["CBYTES", "outside the translatable kernel subset"],
            true, // interpreter faults CastFail(71) at runtime
        ),
        (
            "list-literal-xop",
            "var l = [1u64, 2u64]\nl += 1u64\nreturn 0u64 as bytes".into(),
            vec!["XOP", "outside the translatable kernel subset"],
            true, // interpreter faults CastFail(71) at runtime
        ),
        (
            "list-literal-choose-arm",
            "var c = (byte(data, 0) as u64)\nvar l = [1u64, 2u64]\nvar r = choose(c == 0u64, l, l)\nreturn 0u64 as bytes".into(),
            vec!["CHOOSE", "outside the translatable kernel subset"],
            true,
        ),
        (
            "list-itemget-bytes-receiver",
            "var r = (data[0] as u64)\nreturn r as bytes".into(),
            // `data[i]` is a RUNTIME CompoOpNotMatch(83) on the interpreter
            // (doc 73 E2/E6) — the kernel subset rejects at compile time;
            // fixed-position bytes reads are `byte(data, i)`
            vec!["ITEMGET", "outside the translatable kernel subset"],
            true, // interpreter faults CompoOpNotMatch(83) at runtime
        ),
        (
            "bytecode-splice",
            "bytecode { P0 P1 }\nreturn 0u64 as bytes".into(),
            vec!["IRBYTECODE"],
            true,
        ),
    ]
}
