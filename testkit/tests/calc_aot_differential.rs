//! Behavioral differential bombardment: calc AOT (native x86-64 execution)
//! vs the RV64 interpreter — the fourth leg of the three-way alignment
//! campaign.
//!
//! Corpus: every function body + input + expectation is copied VERBATIM from
//! the three_way campaign cases already merged in hacdtestnet
//! (`src/tests/calc_conformance/three_way/g1..g9`; one representative per op
//! family + every trap-face body: overflow / divide-by-zero / out-of-range /
//! cast / nil / capacity / duplicate-key). Expectations are copied from the
//! campaign assertions (`Ok(bytes)` pins exact bytes, `OkInt` pins BE integer
//! values (same width-independent verdict as g8 `expect_be`), `Trap(code)`
//! pins the consensus code); no criteria are invented here.
//!
//! Verdicts (isomorphic to vm's own lockstep convention, the `EngineOut`
//! comparison face in `vm/src/calc/mod.rs`):
//! * **Ok face**: output bytes identical byte-for-byte + all 32 registers +
//!   the full memory image identical + cycles_used identical;
//! * **Failure face (pinned to the code level)**: every trap in the corpus
//!   appears as `CalcAbort::GuestExit(code)` — the fitsh front end encodes
//!   the consensus fault code into the nonzero a0 of the guest HALT, and both
//!   engines' `exit_code` is the engine-level code. This suite pins each
//!   case: matching `CalcAbort2` class + `exit_code` equal case-by-case
//!   (= the consensus codes pinned by the campaign's HVM channel:
//!   90/71/85/72/40/76/80/81/83/91/73/84) + cycles_used identical + full
//!   register/memory state identical. The interpreter's trap message text is
//!   not compared (AOT does not carry it; the `CalcAbort2` docs say "without
//!   the message payload" — non-consensus data). I.e. **the code level is
//!   pinnable, and pinned**.
//! * **OutOfGas / precheck faces**: `calc_aot_out_of_gas_boundary_parity`
//!   pins the gas metering boundary on a shrinking cycles budget (abort
//!   class + cycles_used + full state bit-identical);
//!   `calc_aot_input_overflow_face` pins the input_max precheck
//!   (Trap + cycles 0).
//! * **golden anti-double-error**: cases with hand-computable /
//!   campaign-pinned values also verify the value itself (two engines erring
//!   together get caught by the golden check).
//!
//! Execution shape (mirrors fullnode `dict_digest_three_paths.rs` path-2/3
//! mounting):
//! 1. `compile_rv64im_body_kernel` builds the container (default ImcRemap;
//!    >=20% of cases sweep Im/Imc modes) → decode + verify;
//! 2. **Interpreter**: `CalcFnObj::build_with_host(.., aot_mount=false)`
//!    (path-2 style) + bare `interp::run_lockstep` (deep-state observation);
//! 3. **AOT**: `CalcFnObj::build_with_host(.., aot_mount=true)` (path-3
//!    style; with built-in interp/AOT lockstep, divergence surfaces as
//!    EngineMismatch) + bare `aot::aot_compile` + `run_with_state`
//!    (deep-state observation);
//! 4. Two comparison layers + golden + stats printlns kept in the log.
//!
//! Discipline: this file only; no vm/src/calc code is touched. Any real
//! divergence found gets minimized, pinned as an `#[ignore]` test, and
//! reported.
//!
//! Run:
//!   cargo test -p testkit --offline --features calc-aot --test calc_aot_differential -- --nocapture

#![cfg(all(feature = "calc-aot", target_arch = "x86_64"))]

use std::collections::HashMap;
use std::sync::Arc;

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::{verify_container_with_cap, CalcModule};
use vm::calc::aot::{aot_compile, AotFunc};
use vm::calc::{CalcAbort2, CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap};
use vm::fitshc::{compile_rv64im_body_kernel, Rv64imBodyMeta};
use vm::ir::rv64::Rv64Mode;
use vm::lang::Tokenizer;

/// One cycles budget for the whole suite (far above the largest loop-body cost in the corpus).
const BUDGET: u64 = 50_000_000;

fn cap() -> SpaceCap {
    SpaceCap::new(0)
}

fn limits() -> CalcRunLimits {
    CalcRunLimits { cycles: BUDGET, resource: -1, deadline: None }
}

fn meta(mode: Rv64Mode) -> Rv64imBodyMeta {
    Rv64imBodyMeta { mode, ..Rv64imBodyMeta::default() }
}

/// used as a cache key (Rv64Mode does not implement Hash).
fn mode_key(m: Rv64Mode) -> &'static str {
    match m {
        Rv64Mode::Im => "Im",
        Rv64Mode::Imc => "Imc",
        Rv64Mode::ImcRemap => "ImcRemap",
    }
}

/* ================================================================== */
/* Expectation & case model                                             */
/* ================================================================== */

/// Expectation faces (all copied from hacdtestnet assertions; nothing invented).
enum Want {
    /// The campaign pinned exact output bytes (same as `Obs::Ok(bytes)`).
    OkBytes(Vec<u8>),
    /// The campaign pinned values as BE integers (like g8 `expect_be`: width-independent).
    OkInt(u128),
    /// The campaign pinned traps (`code` = the HVM channel's consensus fault
    /// code; at the calc container engine level, pinned as CalcAbort2::Trap
    /// class + exit_code 0 — see the code-level pinning note in the module header).
    Trap(u32),
    /// The campaign only required three-way agreement (values unpinned) — here interp/AOT full-state must agree.
    Parity,
}

struct Case {
    id: &'static str,
    body: String,
    input: Vec<u8>,
    want: Want,
    /// Cases drawn for the Im/Imc sweep (>=20% coverage).
    sweep3: bool,
}

fn case(id: &'static str, body: impl Into<String>, input: &[u8], want: Want) -> Case {
    Case { id, body: body.into(), input: input.to_vec(), want, sweep3: false }
}

fn sw(mut c: Case) -> Case {
    c.sweep3 = true;
    c
}

fn ok_bytes(b: &[u8]) -> Want {
    Want::OkBytes(b.to_vec())
}

fn ok_int(v: u128) -> Want {
    Want::OkInt(v)
}

fn trap(code: u32) -> Want {
    Want::Trap(code)
}

/* ---------------- Byte construction (hacdtestnet-style helpers) ---------------- */

/// Value → n-byte BE view (same as g1 `be_bytes`).
fn be_n(v: u128, n: usize) -> Vec<u8> {
    let all = v.to_be_bytes();
    all[16 - n..].to_vec()
}

fn be64(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn pattern_byte(i: usize, salt: u32) -> u8 {
    (((i as u32).wrapping_mul(7) + salt) & 0xFF) as u8
}

fn pattern_bytes(n: usize, salt: u32) -> Vec<u8> {
    (0..n).map(|i| pattern_byte(i, salt)).collect()
}

fn pattern_hex(n: usize, salt: u32) -> String {
    (0..n).map(|i| format!("{:02x}", pattern_byte(i, salt))).collect()
}

/// Like g2 `pin_body`: one body carries multiple boundary pins; byte 0 selects the face.
fn pin_body(pins: &[String]) -> String {
    let mut b = String::from("var s = (byte(data, 0) as u64)\nvar r = (0u64) as bytes\n");
    for (i, p) in pins.iter().enumerate() {
        b.push_str(&format!("if (s == {i}u64) {{ var t{i} = {p}\nr = (t{i}) as bytes }}\n"));
    }
    b.push_str("return r");
    b
}

/* ================================================================== */
/* Engine mounting                                                      */
/* ================================================================== */

/// Both engines for one (body, mode): interpreter (bare + CalcFnObj) and
/// AOT (bare + CalcFnObj-mounted).
struct Engines {
    container_bytes: Vec<u8>,
    container: CalcContainer,
    module: CalcModule,
    aot: AotFunc,
}

fn compile_body(body: &str, m: Rv64Mode) -> Vec<u8> {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    compile_rv64im_body_kernel("aot_diff_kernel", tokens, Some("data"), &[], &[], &[], &meta(m), &cap())
        .unwrap_or_else(|e| panic!("body must compile ({m:?}): {e}\n{body}"))
}

fn build_engines(body: &str, m: Rv64Mode) -> Arc<Engines> {
    let container_bytes = compile_body(body, m);
    let (container, _) = CalcContainer::decode(&container_bytes).expect("container decodes");
    let module = verify_container_with_cap(&container, &cap()).expect("verify");
    let aot = aot_compile(&module).expect("aot compiles");
    Arc::new(Engines { container_bytes, container, module, aot })
}

type EnginesCache = HashMap<(&'static str, String), Arc<Engines>>;

fn engines_for(cache: &mut EnginesCache, body: &str, m: Rv64Mode) -> Arc<Engines> {
    let key = (mode_key(m), body.to_string());
    cache.entry(key).or_insert_with(|| build_engines(body, m)).clone()
}

/* ---------------- Observation faces of the two engines ---------------- */

/// One engine's consensus-comparable face (= the dual of vm's `EngineOut`) + full machine state.
struct Face {
    ok: Option<Vec<u8>>,
    abort: Option<CalcAbort2>,
    exit_code: u64,
    cycles_used: u64,
    regs: [u64; 32],
    mem: Vec<u8>,
}

fn interp_face(eng: &Engines, input: &[u8]) -> (Face, Option<String>) {
    let (res, regs, mem) = interp::run_lockstep(
        &eng.module,
        &eng.container,
        input,
        BUDGET,
        None,
    );
    let (face, msg) = match res {
        Ok(r) => (
            Face { ok: Some(r.output), abort: None, exit_code: 0, cycles_used: r.cycles_used, regs, mem },
            None,
        ),
        Err((abort, used)) => match abort {
            CalcAbort::Trap(m) => (
                Face { ok: None, abort: Some(CalcAbort2::Trap), exit_code: 0, cycles_used: used, regs, mem },
                Some(m),
            ),
            CalcAbort::GuestExit(code) => (
                Face { ok: None, abort: Some(CalcAbort2::Trap), exit_code: code, cycles_used: used, regs, mem },
                None,
            ),
            CalcAbort::OutOfGas => (
                Face { ok: None, abort: Some(CalcAbort2::OutOfGas), exit_code: 0, cycles_used: used, regs, mem },
                None,
            ),
            CalcAbort::Deadline => panic!("no deadline in this suite"),
        },
    };
    (face, msg)
}

fn aot_face(eng: &Engines, input: &[u8]) -> Face {
    let (out, regs, mem) = eng.aot.run_with_state(&eng.module, &eng.container, input, BUDGET);
    Face {
        ok: if out.abort.is_none() { Some(out.output.unwrap_or_default()) } else { None },
        abort: out.abort,
        exit_code: out.exit_code,
        cycles_used: out.cycles_used,
        regs,
        mem,
    }
}

/// Production mounting face: the CalcFnObj (path-2 interpreter vs path-3 AOT mount) consensus face.
fn fnobj_pair(
    eng: &Engines,
    input: &[u8],
) -> (vm::calc::CalcExec, vm::calc::CalcExec) {
    let interp_only = CalcFnObj::build(
        CalcContainer::decode(&eng.container_bytes).expect("decode").0,
        &cap(),
        CalcHostConfig::new().with_aot_mount(false),
    )
    .expect("interp-only build");
    let aot_mounted = CalcFnObj::build(
        CalcContainer::decode(&eng.container_bytes).expect("decode").0,
        &cap(),
        CalcHostConfig::new().with_aot_mount(true),
    )
    .expect("aot-mounted build");
    (interp_only.run(input, limits()), aot_mounted.run(input, limits()))
}

/// The comparable part of the faces (state excluded) — used for equality-assertion error reports.
fn face_summary(f: &Face) -> String {
    match (&f.ok, &f.abort) {
        (Some(o), _) => format!("Ok(0x{}) cycles={}", hex::encode(o), f.cycles_used),
        (_, Some(a)) => format!(
            "{a:?} exit_code={} cycles={} 0x{}",
            f.exit_code,
            f.cycles_used,
            hex::encode(&f.mem[..f.mem.len().min(64)])
        ),
        _ => "??".into(),
    }
}

fn assert_faces_agree(case_id: &str, input: &[u8], body: &str, i: &Face, a: &Face) {
    assert_eq!(
        i.abort, a.abort,
        "[{case_id}] abort class mismatch:\n  interp = {}\n  aot    = {}\n  input = 0x{}\n  body =\n{body}",
        face_summary(i), face_summary(a), hex::encode(input)
    );
    assert_eq!(
        i.ok, a.ok,
        "[{case_id}] output bytes mismatch:\n  interp = 0x{}\n  aot    = 0x{}\n  input = 0x{}\n  body =\n{body}",
        i.ok.as_ref().map(|o| hex::encode(o)).unwrap_or_default(),
        a.ok.as_ref().map(|o| hex::encode(o)).unwrap_or_default(),
        hex::encode(input)
    );
    assert_eq!(
        i.exit_code, a.exit_code,
        "[{case_id}] exit_code mismatch (interp {} vs aot {})",
        i.exit_code, a.exit_code
    );
    assert_eq!(
        i.cycles_used, a.cycles_used,
        "[{case_id}] cycles_used mismatch (interp {} vs aot {}) — gas metering drift",
        i.cycles_used, a.cycles_used
    );
    assert_eq!(i.regs, a.regs, "[{case_id}] register file mismatch (full 32-reg state)");
    assert_eq!(i.mem, a.mem, "[{case_id}] memory image mismatch (full stop state)");
}

/* ================================================================== */
/* Corpus (copied verbatim from hacdtestnet three_way g1..g9)           */
/* ================================================================== */

/* ---------------- G1 helpers (same as g1_arith.rs) ---------------- */

#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    U8,
    U64,
}

impl W {
    #[allow(dead_code)]
    fn ty(self) -> &'static str {
        match self {
            W::U8 => "u8",
            W::U64 => "u64",
        }
    }
    fn nbytes(self) -> usize {
        match self {
            W::U8 => 1,
            W::U64 => 8,
        }
    }
    fn bits(self) -> u32 {
        match self {
            W::U8 => 8,
            W::U64 => 64,
        }
    }
    fn max(self) -> u128 {
        (1u128 << self.bits()) - 1
    }
}

/// Assemble a width-W value from `data[off..off+n]` BE (same as g1 `read_lines`).
fn read_lines(v: &str, off: usize, w: W) -> String {
    match w {
        W::U8 => format!("var {v} = byte(data, {off})"),
        W::U64 => format!(
            "var {v} = byte(data, {off}) * 72057594037927936 + byte(data, {o1}) * 281474976710656 + byte(data, {o2}) * 1099511627776 + byte(data, {o3}) * 4294967296 + byte(data, {o4}) * 16777216 + byte(data, {o5}) * 65536 + byte(data, {o6}) * 256 + byte(data, {o7})",
            o1 = off + 1, o2 = off + 2, o3 = off + 3, o4 = off + 4, o5 = off + 5, o6 = off + 6, o7 = off + 7
        ),
    }
}

fn bin_body(w: W, op: &str) -> String {
    format!(
        "{}\n{}\nreturn ((a {op} b) as bytes)",
        read_lines("a", 0, w),
        read_lines("b", w.nbytes(), w)
    )
}

fn fin2_body(w: W, f: &str) -> String {
    format!(
        "{}\n{}\nreturn (({f}(a, b)) as bytes)",
        read_lines("a", 0, w),
        read_lines("b", w.nbytes(), w)
    )
}

fn clamp_body(w: W) -> String {
    let n = w.nbytes();
    format!(
        "{}\n{}\n{}\nreturn ((clamp(a, lo, hi)) as bytes)",
        read_lines("a", 0, w),
        read_lines("lo", n, w),
        read_lines("hi", 2 * n, w)
    )
}

/// (input, want) pairs (same as g1 `ab`/`tri`).
fn ab(w: W, a: u128, b: u128, want: Result<u128, u32>) -> (Vec<u8>, Want) {
    let n = w.nbytes();
    let mut input = be_n(a, n);
    input.extend(be_n(b, n));
    (input, match want {
        Ok(v) => ok_bytes(&be_n(v, n)),
        Err(c) => trap(c),
    })
}

fn tri(w: W, a: u128, lo: u128, hi: u128, want: Result<u128, u32>) -> (Vec<u8>, Want) {
    let n = w.nbytes();
    let mut input = be_n(a, n);
    input.extend(be_n(lo, n));
    input.extend(be_n(hi, n));
    (input, match want {
        Ok(v) => ok_bytes(&be_n(v, n)),
        Err(c) => trap(c),
    })
}

const ARITH: u32 = 90;

fn push_w(v: &mut Vec<Case>, tag: &'static str, body: impl Into<String>, rows: Vec<(Vec<u8>, Want)>) {
    let body = body.into();
    for (i, (input, want)) in rows.into_iter().enumerate() {
        let id: &'static str = Box::leak(format!("{tag}[{i}]").into_boxed_str());
        v.push(case(id, body.clone(), &input, want));
    }
}

fn g1_cases() -> Vec<Case> {
    let mut v = Vec::new();

    // ADD: u8 (narrow-width D5 face) + u64.
    for w in [W::U8, W::U64] {
        let rows = vec![
            ab(w, 0, 0, Ok(0)),
            ab(w, 1, 2, Ok(3)),
            ab(w, w.max(), 0, Ok(w.max())),
            ab(w, w.max(), 1, Err(ARITH)),
        ];
        let (t, b) = ("g1.add", bin_body(w, "+"));
        push_w(&mut v, t, b, rows);
    }
    // SUB / MUL / DIV / MOD（u64）。
    push_w(&mut v, "g1.sub", bin_body(W::U64, "-"), vec![
        ab(W::U64, 5, 3, Ok(2)),
        ab(W::U64, W::U64.max(), W::U64.max(), Ok(0)),
        ab(W::U64, 3, 5, Err(ARITH)),
        ab(W::U64, 0, 1, Err(ARITH)),
    ]);
    push_w(&mut v, "g1.mul", bin_body(W::U64, "*"), vec![
        ab(W::U64, 0, 5, Ok(0)),
        ab(W::U64, 3, 7, Ok(21)),
        ab(W::U64, W::U64.max(), 0, Ok(0)),
        ab(W::U64, W::U64.max(), 2, Err(ARITH)),
    ]);
    push_w(&mut v, "g1.div", bin_body(W::U64, "/"), vec![
        ab(W::U64, 7, 2, Ok(3)),
        ab(W::U64, 0, 5, Ok(0)),
        ab(W::U64, W::U64.max(), 1, Ok(W::U64.max())),
        ab(W::U64, 7, 0, Err(ARITH)),
    ]);
    push_w(&mut v, "g1.mod", bin_body(W::U64, "%"), vec![
        ab(W::U64, 7, 2, Ok(1)),
        ab(W::U64, 7, 3, Ok(1)),
        ab(W::U64, W::U64.max(), 1, Ok(0)),
        ab(W::U64, 7, 0, Err(ARITH)),
    ]);
    // BSHL / BSHR (u64, shift-mask out-of-range face).
    let m64 = W::U64.max();
    push_w(&mut v, "g1.bshl", bin_body(W::U64, "<<"), vec![
        ab(W::U64, 1, 0, Ok(1)),
        ab(W::U64, m64, 1, Ok((m64 << 1) & m64)),
        ab(W::U64, 1, 63, Ok(1u128 << 63)),
        ab(W::U64, 1, 64, Err(ARITH)),
    ]);
    push_w(&mut v, "g1.bshr", bin_body(W::U64, ">>"), vec![
        ab(W::U64, m64, 0, Ok(m64)),
        ab(W::U64, m64, 1, Ok(m64 >> 1)),
        ab(W::U64, m64, 63, Ok(1)),
        ab(W::U64, 1, 64, Err(ARITH)),
    ]);
    // POW (overflow face 3^41 → 90).
    push_w(&mut v, "g1.pow", bin_body(W::U64, "**"), vec![
        ab(W::U64, 0, 0, Ok(1)),
        ab(W::U64, 2, 3, Ok(8)),
        ab(W::U64, 3, 5, Ok(243)),
        ab(W::U64, 3, 41, Err(ARITH)),
    ]);
    // CLAMP（lo>hi → 90）。
    push_w(&mut v, "g1.clamp", clamp_body(W::U64), vec![
        tri(W::U64, 5, 1, 10, Ok(5)),
        tri(W::U64, 0, 1, 10, Ok(1)),
        tri(W::U64, 15, 1, 10, Ok(10)),
        tri(W::U64, 7, 7, 7, Ok(7)),
        tri(W::U64, 5, 10, 1, Err(ARITH)),
    ]);
    // MAX / MIN / ABSDIFF。
    push_w(&mut v, "g1.max", fin2_body(W::U64, "max"), vec![
        ab(W::U64, 5, 3, Ok(5)),
        ab(W::U64, 3, 5, Ok(5)),
        ab(W::U64, m64, 0, Ok(m64)),
    ]);
    push_w(&mut v, "g1.min", fin2_body(W::U64, "min"), vec![
        ab(W::U64, 5, 3, Ok(3)),
        ab(W::U64, m64, 0, Ok(0)),
        ab(W::U64, 0, m64, Ok(0)),
    ]);
    push_w(&mut v, "g1.absdiff", fin2_body(W::U64, "abs_diff"), vec![
        ab(W::U64, 5, 3, Ok(2)),
        ab(W::U64, 3, 5, Ok(2)),
        ab(W::U64, 0, m64, Ok(m64)),
    ]);
    // DIVUP / DIVEXACT。
    push_w(&mut v, "g1.divup", fin2_body(W::U64, "div_up"), vec![
        ab(W::U64, 7, 2, Ok(4)),
        ab(W::U64, 8, 2, Ok(4)),
        ab(W::U64, 0, 5, Ok(0)),
        ab(W::U64, 10, 3, Ok(4)),
        ab(W::U64, 7, 0, Err(ARITH)),
    ]);
    push_w(&mut v, "g1.divexact", fin2_body(W::U64, "div_exact_op"), vec![
        ab(W::U64, 10, 2, Ok(5)),
        ab(W::U64, 9, 3, Ok(3)),
        ab(W::U64, 10, 3, Err(ARITH)),
        ab(W::U64, 7, 0, Err(ARITH)),
    ]);
    // Mixed-width zero extension (g1_mixed_*).
    push_w(&mut v, "g1.mixed.add.u8u64",
        "var a = byte(data, 0)\nvar b = (byte(data, 1) as u64)\nreturn ((a + b) as bytes)", vec![
        (vec![255, 1], ok_int(256)),
        (vec![0, 0], ok_int(0)),
        (vec![200, 55], ok_int(255)),
        (vec![255, 255], ok_int(510)),
    ]);
    push_w(&mut v, "g1.mixed.div.u8u128",
        "var a = byte(data, 0)\nvar b = (byte(data, 1) as u128)\nreturn ((a / b) as bytes)", vec![
        (vec![9, 3], ok_int(3)),
        (vec![255, 1], ok_int(255)),
        (vec![200, 255], ok_int(0)),
        (vec![255, 0], trap(ARITH)),
    ]);
    push_w(&mut v, "g1.mixed.bshl.u8u16",
        "var a = 1u8\nvar b = (byte(data, 0) as u16)\nreturn ((a << b) as bytes)", vec![
        (vec![0], ok_int(1)),
        (vec![9], ok_int(512)),
        (vec![15], ok_int(32768)),
        (vec![16], trap(ARITH)),
        (vec![200], trap(ARITH)),
    ]);
    v
}

/* ---------------- G2 (pin_body corpus, same as g2_rowc.rs) ---------------- */

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const BIGM: &str = "1000000000000000000000000000007";

fn g2_cases() -> Vec<Case> {
    let mut v = Vec::new();

    // MULADD / MULSUB narrow/u64 (g2_rowc_muladdsub_narrow_u64).
    let pins: Vec<(String, Want)> = vec![
        ("mul_add(100u8, 2u8, 50u8)".to_string(), ok_int(250)),
        ("mul_sub(100u8, 2u8, 50u8)".to_string(), Want::Parity),
        ("mul_add(200u8, 2u8, 100u8)".to_string(), Want::Parity), // campaign comment claims 90 but no pinned value → Parity
        ("mul_add(200u8, 2u8, 100u16)".to_string(), ok_bytes(&[1, 0xf4])),
        ("mul_sub(200u8, 2u8, 255u8)".to_string(), Want::Parity), // measured Ok(145) (campaign comment typo; value unpinned)
        ("mul_add(65535u16, 65535u16, 1u32)".to_string(), Want::Parity),
        ("mul_sub(4000000000u32, 2u32, 1u32)".to_string(), Want::Parity),
        ("mul_add(3000000000u64, 3u64, 5000000000u64)".to_string(), ok_bytes(&be64(14_000_000_000))),
        ("mul_sub(3000000000u64, 3u64, 1000000000u64)".to_string(), Want::Parity),
        (format!("mul_add({MAX_U64}u64, {MAX_U64}u64, 0u64)"), trap(ARITH)),
        (format!("mul_sub({MAX_U64}u64, {MAX_U64}u64, 1u64)"), trap(ARITH)),
        (format!("mul_add(4294967295u64, 4294967295u64, {MAX_U64}u64)"), trap(ARITH)),
    ];
    let body = pin_body(&pins.iter().map(|(p, _)| p.to_string()).collect::<Vec<_>>());
    for (i, (_, want)) in pins.into_iter().enumerate() {
        v.push(sw(case(Box::leak(format!("g2.muladdsub[{i}]").into_boxed_str()), body.clone(), &[i as u8], want)));
    }

    // MULDIV / MULDIVUP narrow/u64 (g2_rowc_muldiv_narrow_u64).
    let pins: Vec<(String, Want)> = vec![
        ("mul_div(7u32, 5u32, 8u32)".to_string(), ok_bytes(&[0, 0, 0, 4])),
        ("mul_div_up(7u32, 5u32, 8u32)".to_string(), ok_bytes(&[0, 0, 0, 5])),
        ("mul_div(6u32, 4u32, 8u32)".to_string(), Want::Parity),
        ("mul_div(255u8, 255u8, 2u8)".to_string(), Want::Parity),
        ("mul_div(255u8, 255u8, 2u16)".to_string(), Want::Parity),
        ("mul_div_up(255u8, 255u8, 2u16)".to_string(), Want::Parity),
        ("mul_div(200u8, 200u8, 1u8)".to_string(), Want::Parity),
        ("mul_div_up(200u8, 200u8, 1u8)".to_string(), Want::Parity),
        ("mul_div(5u32, 3u32, 0u32)".to_string(), trap(ARITH)),
        ("mul_div_up(5u32, 3u32, 0u32)".to_string(), Want::Parity),
        ("mul_div(4294967295u64, 4294967295u64, 2u64)".to_string(), Want::Parity),
        ("mul_div_up(4294967295u64, 4294967295u64, 2u64)".to_string(), Want::Parity),
        (format!("mul_div({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"), Want::Parity),
        (format!("mul_div_up({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"), Want::Parity),
        ("mul_div(0u64, 18446744073709551615u64, 7u64)".to_string(), ok_bytes(&be64(0))),
        (format!("mul_div({MAX_U64}u64, {MAX_U64}u64, 1u64)"), trap(ARITH)),
        (format!("mul_div_up({MAX_U64}u64, {MAX_U64}u64, 1u64)"), trap(ARITH)),
        (format!("mul_div({MAX_U64}u64, 5u64, 0u64)"), trap(ARITH)),
        ("mul_div(3u64, 2u64, 100u64)".to_string(), ok_bytes(&be64(0))),
        ("mul_div_up(3u64, 2u64, 100u64)".to_string(), ok_bytes(&be64(1))),
        (format!("mul_div({MAX_U64}u64, 5u64, {MAX_U64}u64)"), Want::Parity),
    ];
    let body = pin_body(&pins.iter().map(|(p, _)| p.to_string()).collect::<Vec<_>>());
    for (i, (_, want)) in pins.into_iter().enumerate() {
        v.push(case(Box::leak(format!("g2.muldiv[{i}]").into_boxed_str()), body.clone(), &[i as u8], want));
    }

    // ADDMOD / MULMOD u64（g2_rowc_modarith_u64）。
    let pins: Vec<(String, Want)> = vec![
        ("add_mod(5u64, 7u64, 10u64)".to_string(), ok_bytes(&be64(2))),
        ("mul_mod(11u64, 13u64, 17u64)".to_string(), ok_bytes(&be64(7))),
        (format!("add_mod({MAX_U64}u64, 1u64, {MAX_U64}u64)"), Want::Parity),
        (format!("add_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"), Want::Parity),
        (format!("mul_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"), Want::Parity),
        ("mul_mod(4294967296u64, 4294967296u64, 4294967295u64)".to_string(), Want::Parity),
        ("add_mod(5u64, 3u64, 0u64)".to_string(), trap(ARITH)),
        ("mul_mod(5u64, 3u64, 0u64)".to_string(), trap(ARITH)),
        ("add_mod(5u64, 3u64, 1u64)".to_string(), ok_bytes(&be64(0))),
        ("mul_mod(5u64, 3u64, 1u64)".to_string(), Want::Parity),
        ("add_mod(200u8, 200u8, 256u16)".to_string(), Want::Parity),
        ("mul_mod(250u8, 250u8, 251u8)".to_string(), Want::Parity),
        ("mul_mod(65535u16, 65535u16, 65537u32)".to_string(), Want::Parity),
        ("add_mod(255u8, 4294967296u64, 65537u64)".to_string(), Want::Parity),
        ("mul_mod(65535u16, 4294967295u32, 65537u64)".to_string(), Want::Parity),
    ];
    let body = pin_body(&pins.iter().map(|(p, _)| p.to_string()).collect::<Vec<_>>());
    for (i, (_, want)) in pins.into_iter().enumerate() {
        v.push(sw(case(Box::leak(format!("g2.modarith[{i}]").into_boxed_str()), body.clone(), &[i as u8], want)));
    }

    // ADDMOD / MULMOD u128（g2_rowc_modarith_u128）。
    let pins: Vec<(String, Want)> = vec![
        (format!("add_mod({BIG1}u128, {BIG2}u128, {BIGM}u128)"), Want::Parity),
        (format!("mul_mod({BIG1}u128, {BIG2}u128, {BIGM}u128)"), Want::Parity),
        (format!("add_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128)"), ok_bytes(&vec![0u8; 16])),
        (format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128)"), Want::Parity),
        (format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U64}u128)"), Want::Parity),
        (format!("add_mod({BIG1}u128, {BIG2}u128, 0u128)"), trap(ARITH)),
        (format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, 1u128)"), ok_bytes(&vec![0u8; 16])),
        (format!("mul_mod({BIG1}u128, 12345u64, {BIGM}u128)"), Want::Parity),
        (format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {BIGM}u128)"), Want::Parity),
    ];
    let body = pin_body(&pins.iter().map(|(p, _)| p.to_string()).collect::<Vec<_>>());
    for (i, (_, want)) in pins.into_iter().enumerate() {
        v.push(case(Box::leak(format!("g2.modarith128[{i}]").into_boxed_str()), body.clone(), &[i as u8], want));
    }
    v
}

/* ---------------- G3 (catalog excerpt, same as g3_fin.rs) ---------------- */

fn g3_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let fin = |v: &mut Vec<Case>, id: &'static str, body: &str, want: Want| {
        v.push(case(id, body, &[], want));
    };
    // FIN2
    fin(&mut v, "g3.fin2.div_exact.exact", "return div_exact(8u64, 2u64) as bytes", ok_int(4));
    fin(&mut v, "g3.fin2.div_exact.inexact", "return div_exact(7u64, 2u64) as bytes", trap(90));
    fin(&mut v, "g3.fin2.div_floor", "return div_floor(7u64, 2u64) as bytes", ok_int(3));
    fin(&mut v, "g3.fin2.div_half_up", "return div_half_up(5u64, 2u64) as bytes", ok_int(3));
    fin(&mut v, "g3.fin2.sat_add", "return sat_add(200u8, 100u8) as bytes", ok_bytes(&[255]));
    fin(&mut v, "g3.fin2.sat_sub", "return sat_sub(100u8, 200u8) as bytes", ok_bytes(&[0]));
    fin(&mut v, "g3.fin2.div.zero", "return div_floor(7u64, 0u64) as bytes", trap(90));
    // FIN3
    fin(&mut v, "g3.fin3.mul_div_exact.exact", "return mul_div_exact(10u64, 4u64, 5u64) as bytes", ok_int(8));
    fin(&mut v, "g3.fin3.mul_div_exact.inexact", "return mul_div_exact(10u64, 4u64, 3u64) as bytes", trap(90));
    fin(&mut v, "g3.fin3.mul_div_floor", "return mul_div_floor(10u64, 4u64, 3u64) as bytes", ok_int(13));
    fin(&mut v, "g3.fin3.mul_div_ceil", "return mul_div_ceil(10u64, 4u64, 3u64) as bytes", ok_int(14));
    fin(&mut v, "g3.fin3.scaled_add_floor", "return scaled_add_floor(100u64, 3u64, 2u64) as bytes", ok_int(250));
    fin(&mut v, "g3.fin3.scaled_sub_floor", "return scaled_sub_floor(100u64, 1u64, 2u64) as bytes", ok_int(50));
    fin(&mut v, "g3.fin3.u64_mad", "return u64_mad(100u64, 3u64, 10u64) as bytes", ok_int(130));
    fin(&mut v, "g3.fin3.u64_mad.u8-ovf", "return u64_mad(250u8, 3u64, 10u64) as bytes", trap(90));
    fin(&mut v, "g3.fin3.u64_mad.u16", "return u64_mad(5u16, 3u64, 10u64) as bytes", ok_bytes(&be_n(35, 2)));
    // FIN4
    fin(&mut v, "g3.fin4.mul_sub_div.neg", "return mul_sub_div_floor(5u64, 4u64, 100u64, 9u64) as bytes", trap(90));
    // FINP3
    fin(&mut v, "g3.finp3.abs_diff_lte.true", "return (abs_diff_lte(5u64, 9u64, 4u64) as u64) as bytes", ok_int(1));
    fin(&mut v, "g3.finp3.abs_diff_lte.false", "return (abs_diff_lte(5u64, 9u64, 3u64) as u64) as bytes", ok_int(0));
    // FINPOW3 (rpow = (x/z)^y * z, iterated multiplication half-up)
    fin(&mut v, "g3.finpow3.scaled-square", "return rpow_half_up(3u64, 2u64, 10u64) as bytes", ok_int(1));
    fin(&mut v, "g3.finpow3.half-up-per-mul", "return rpow_half_up(105u64, 2u64, 100u64) as bytes", ok_int(110));
    fin(&mut v, "g3.finpow3.scale-zero", "return rpow_half_up(5u64, 3u64, 0u64) as bytes", trap(90));
    fin(&mut v, "g3.finpow3.zero-base", "return rpow_half_up(0u64, 5u64, 10u64) as bytes", ok_int(0));
    v
}

/* ---------------- G4 (same as g4_bytes.rs) ---------------- */

fn g4_cases() -> Vec<Case> {
    let mut v = Vec::new();

    // omnibus: 12 bytes ops in one body (three-mode sweep).
    let body = format!(
        "var h = buf_left(3, data)\nvar t = buf_right(2, data)\nvar ld = buf_left_drop(1, data)\nvar rd = buf_right_drop(1, data)\nvar c = buf_cut(data, 1, 3)\nvar n = size(c)\nvar lit = \"K7\"\nvar big = 0x{}\nvar emp = \"\"\nvar b0 = byte(data, 0)\nreturn h ++ t ++ ld ++ rd ++ c ++ lit ++ big ++ emp ++ (n as bytes) ++ (b0 as bytes)",
        pattern_hex(300, 0xA3)
    );
    let input: Vec<u8> = (10u8..20u8).collect();
    let expected: Vec<u8> = [
        input[0..3].to_vec(),
        input[8..10].to_vec(),
        input[1..10].to_vec(),
        input[0..9].to_vec(),
        input[1..4].to_vec(),
        b"K7".to_vec(),
        pattern_bytes(300, 0xA3),
        vec![0, 3],
        vec![10],
    ]
    .concat();
    v.push(sw(case("g4.omni", body, &input, ok_bytes(&expected))));

    // Boundary slices.
    const SLB: &str = "var f = buf_cut(data, 0, 1)\nvar l = buf_cut(data, byte(data, 0), 1)\nvar z = buf_left(0, data)\nvar all = buf_cut(data, 0, size(data))\nvar k = buf_right(0, data)\nvar d0 = buf_left_drop(0, data)\nvar d9 = buf_right_drop(0, data)\nreturn f ++ l ++ z ++ all ++ k ++ d0 ++ d9";
    v.push(sw(case("g4.slb.3b", SLB, &[2, 5, 6], ok_bytes(&[2, 6, 2, 5, 6, 2, 5, 6, 2, 5, 6]))));
    v.push(case("g4.slb.1b", SLB, &[0], ok_bytes(&[0, 0, 0, 0, 0])));

    // The three BYTE out-of-range faces (the 42/71 channel divergence face is at the calc engine level = same container → class agreement).
    const BYTEOOB: &str = "var v = byte(data, 42)\nvar w = byte(data, 90)\nreturn (v as bytes) ++ (w as bytes)";
    v.push(case("g4.byte-oob.small", BYTEOOB, &[1, 2], trap(71)));
    let input90 = pattern_bytes(90, 0x11);
    v.push(case("g4.byte-oob.end", BYTEOOB, &input90, trap(71)));
    let input100 = pattern_bytes(100, 0x22);
    v.push(sw(case("g4.byte-oob.ok", BYTEOOB, &input100,
        ok_bytes(&[pattern_byte(42, 0x22), pattern_byte(90, 0x22)]))));

    // CUT out-of-range / end-aligned.
    const CUT7: &str = "var c = buf_cut(data, 7, 4)\nreturn c";
    v.push(case("g4.cut-oob", CUT7, &pattern_bytes(10, 0x33), trap(71)));
    v.push(case("g4.cut-ok", CUT7, &pattern_bytes(11, 0x33),
        ok_bytes(&pattern_bytes(11, 0x33)[7..11])));

    // DROP out-of-range / drop-all.
    const DROP11: &str = "var a = buf_left_drop(11, data)\nvar b = buf_right_drop(11, data)\nreturn a ++ b";
    v.push(case("g4.drop-oob", DROP11, &pattern_bytes(10, 0x44), trap(71)));
    v.push(case("g4.drop-all", DROP11, &pattern_bytes(11, 0x44), ok_bytes(&[])));

    // Literal tri-state (PNBUF/PBUF/PBUFL).
    let body = format!(
        "var e = \"\"\nvar s = \"K7\"\nvar l = 0x{}\nreturn s ++ l ++ e ++ s ++ (l ++ s)",
        pattern_hex(300, 0x5A)
    );
    let expected: Vec<u8> = [
        b"K7".to_vec(),
        pattern_bytes(300, 0x5A),
        b"K7".to_vec(),
        pattern_bytes(300, 0x5A),
        b"K7".to_vec(),
    ]
    .concat();
    v.push(case("g4.lit.empty-input", body.clone(), &[], ok_bytes(&expected)));
    v.push(case("g4.lit.small-input", body, &[1, 2], ok_bytes(&expected)));

    // Concatenation over-wide (900B output).
    v.push(case("g4.wide", "var a = buf_cut(data, 0, 300)\nreturn a ++ a ++ a",
        &pattern_bytes(480, 0x77),
        ok_bytes(&[pattern_bytes(300, 0x77), pattern_bytes(300, 0x77), pattern_bytes(300, 0x77)].concat())));

    // Byte comparison (EQ/NEQ).
    const CMP: &str = "var eq = (buf_left(2, data) == buf_left(2, data))\nvar ne = (buf_left(1, data) != buf_right(1, data))\nvar eq2 = (data == buf_cut(data, 0, size(data)))\nreturn ((eq as u64) as bytes) ++ ((ne as u64) as bytes) ++ ((eq2 as u64) as bytes)";
    let ones8 = vec![0u8, 0, 0, 0, 0, 0, 0, 1];
    let ones8b = ones8.clone();
    let zeros8 = vec![0u8; 8];
    v.push(sw(case("g4.cmp.true", CMP, &[5, 6, 7, 8],
        ok_bytes(&[ones8.clone(), ones8.clone(), ones8.clone()].concat()))));
    v.push(case("g4.cmp.false", CMP, &[5, 5],
        ok_bytes(&[ones8b.clone(), zeros8, ones8b].concat())));

    // The full data-read family (dynamic byte/cut/size).
    const READ: &str = "var i = byte(data, byte(data, 0))\nvar c = buf_cut(data, 1, byte(data, byte(data, 1)))\nvar s = size(data)\nvar l = size(buf_cut(data, 0, byte(data, 2)))\nreturn (i as bytes) ++ c ++ (s as bytes) ++ (l as bytes)";
    v.push(sw(case("g4.read", READ, &[0, 2, 2, 9, 8, 7], ok_bytes(&[0, 2, 2, 0, 6, 0, 2]))));
    v.push(case("g4.read.zerow", READ, &[0, 0, 2], ok_bytes(&[0, 0, 3, 0, 2])));

    // nil bytes faces: size(nil) = Ok 0 (aligned semantics); byte(nil,0) = 91; nil as bytes = 71.
    const NILSZ: &str = "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = size(local_x(1))\nreturn (r as bytes)";
    v.push(case("g4.nil.size", NILSZ, &[0], ok_bytes(&[0, 0])));
    v.push(case("g4.nil.size.taken", NILSZ, &[1, 5, 6], ok_bytes(&[0, 2])));
    const NILBYTE: &str = "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = byte(local_x(1), 0)\nreturn (r as bytes)";
    v.push(case("g4.nil.byte", NILBYTE, &[0], trap(91)));
    v.push(case("g4.nil.byte.taken", NILBYTE, &[1, 5, 6], ok_bytes(&[1])));
    const NILCB: &str = "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = (local_x(1) as bytes)\nreturn r";
    v.push(case("g4.nil.cbytes", NILCB, &[0], trap(71)));
    v.push(case("g4.nil.cbytes.taken", NILCB, &[1, 5, 6], ok_bytes(&[1, 5])));

    // CHOOSE bytes arm (eager).
    const CHB: &str = "var r = if byte(data, 0) == 1u64 { buf_left(1, data) } else { buf_right(1, data) }\nreturn r";
    v.push(sw(case("g4.choose.true", CHB, &[1, 9], ok_bytes(&[1]))));
    v.push(case("g4.choose.false", CHB, &[0, 9], ok_bytes(&[9])));
    v.push(case("g4.choose.1b", CHB, &[1], ok_bytes(&[1])));
    v
}

/* ---------------- G5 (same as g5_control.rs) ---------------- */

fn g5_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let c5 = |v: &mut Vec<Case>, id: &'static str, body: &str, input: &[u8], want: Want, sweep: bool| {
        let c = case(id, body, input, want);
        v.push(if sweep { sw(c) } else { c });
    };

    c5(&mut v, "g5.if.stmt[5]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 10u64) {\ns = a * 2u64\n}\nif (a > 20u64) {\ns = s + 1u64\n} else {\ns = s + 2u64\n}\nreturn (s as u64) as bytes", &[5], ok_int(2), true);
    c5(&mut v, "g5.if.stmt[15]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 10u64) {\ns = a * 2u64\n}\nif (a > 20u64) {\ns = s + 1u64\n} else {\ns = s + 2u64\n}\nreturn (s as u64) as bytes", &[15], ok_int(32), true);
    c5(&mut v, "g5.if.stmt[25]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 10u64) {\ns = a * 2u64\n}\nif (a > 20u64) {\ns = s + 1u64\n} else {\ns = s + 2u64\n}\nreturn (s as u64) as bytes", &[25], ok_int(51), true);
    c5(&mut v, "g5.if.value[5]", "var a = (byte(data, 0) as u64)\nvar r = if (a > 10u64) { a * 2u64 } else { a + 1u64 }\nreturn (r as u64) as bytes", &[5], ok_int(6), false);
    c5(&mut v, "g5.if.value[15]", "var a = (byte(data, 0) as u64)\nvar r = if (a > 10u64) { a * 2u64 } else { a + 1u64 }\nreturn (r as u64) as bytes", &[15], ok_int(30), false);
    c5(&mut v, "g5.if.nest[0]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 1u64) {\nif (a > 2u64) {\ns += 1u64\n}\n}\nif (a > 3u64) {\nif (a > 4u64) {\nif (a > 5u64) {\nif (a > 6u64) {\ns += 2u64\n}\n}\n}\n}\nif (a > 7u64) {\nif (a > 8u64) {\nif (a > 9u64) {\nif (a > 10u64) {\nif (a > 11u64) {\nif (a > 12u64) {\nif (a > 13u64) {\nif (a > 14u64) {\ns += 4u64\n}\n}\n}\n}\n}\n}\n}\n}\nreturn (s as u64) as bytes", &[0], ok_int(0), false);
    c5(&mut v, "g5.if.nest[3]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 1u64) {\nif (a > 2u64) {\ns += 1u64\n}\n}\nif (a > 3u64) {\nif (a > 4u64) {\nif (a > 5u64) {\nif (a > 6u64) {\ns += 2u64\n}\n}\n}\n}\nif (a > 7u64) {\nif (a > 8u64) {\nif (a > 9u64) {\nif (a > 10u64) {\nif (a > 11u64) {\nif (a > 12u64) {\nif (a > 13u64) {\nif (a > 14u64) {\ns += 4u64\n}\n}\n}\n}\n}\n}\n}\n}\nreturn (s as u64) as bytes", &[3], ok_int(1), false);
    c5(&mut v, "g5.if.nest[15]", "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 1u64) {\nif (a > 2u64) {\ns += 1u64\n}\n}\nif (a > 3u64) {\nif (a > 4u64) {\nif (a > 5u64) {\nif (a > 6u64) {\ns += 2u64\n}\n}\n}\n}\nif (a > 7u64) {\nif (a > 8u64) {\nif (a > 9u64) {\nif (a > 10u64) {\nif (a > 11u64) {\nif (a > 12u64) {\nif (a > 13u64) {\nif (a > 14u64) {\ns += 4u64\n}\n}\n}\n}\n}\n}\n}\n}\nreturn (s as u64) as bytes", &[15], ok_int(7), false);
    c5(&mut v, "g5.choose.ternary[5]", "var a = (byte(data, 0) as u64)\nvar r = (a > 10u64) ? (a * 2u64) : (a + 7u64)\nreturn (r as u64) as bytes", &[5], ok_int(12), true);
    c5(&mut v, "g5.choose.ternary[15]", "var a = (byte(data, 0) as u64)\nvar r = (a > 10u64) ? (a * 2u64) : (a + 7u64)\nreturn (r as u64) as bytes", &[15], ok_int(30), true);
    c5(&mut v, "g5.choose.builtin[5]", "var a = (byte(data, 0) as u64)\nvar r = choose((a > 10u64), (a * 3u64), (a + 5u64))\nreturn (r as u64) as bytes", &[5], ok_int(10), false);
    c5(&mut v, "g5.choose.eager[0]", "var a = (byte(data, 0) as u64)\nvar z = (a - a)\nvar r = choose((a > 0u64), (7u64 / z), 42u64)\nreturn (r as u64) as bytes", &[0], trap(90), false);
    c5(&mut v, "g5.choose.eager[9]", "var a = (byte(data, 0) as u64)\nvar z = (a - a)\nvar r = choose((a > 0u64), (7u64 / z), 42u64)\nreturn (r as u64) as bytes", &[9], trap(90), false);
    c5(&mut v, "g5.while.break[0]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\nif (i == 4u64) {\nbreak\n}\ns += i\ni += 1u64\n}\nreturn (s as u64) as bytes", &[0], ok_int(0), true);
    c5(&mut v, "g5.while.break[5]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\nif (i == 4u64) {\nbreak\n}\ns += i\ni += 1u64\n}\nreturn (s as u64) as bytes", &[5], ok_int(6), true);
    c5(&mut v, "g5.while.break[255]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\nif (i == 4u64) {\nbreak\n}\ns += i\ni += 1u64\n}\nreturn (s as u64) as bytes", &[255], ok_int(6), true);
    c5(&mut v, "g5.while.cont[4]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif ((i % 2u64) == 0u64) {\ncontinue\n}\ns += i\n}\nreturn (s as u64) as bytes", &[4], ok_int(4), false);
    c5(&mut v, "g5.while.cont[5]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif ((i % 2u64) == 0u64) {\ncontinue\n}\ns += i\n}\nreturn (s as u64) as bytes", &[5], ok_int(9), false);
    c5(&mut v, "g5.while.trap[0]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif (i == 3u64) {\ns += (9u64 / (i - i))\n}\ns += 1u64\n}\nreturn (s as u64) as bytes", &[0], ok_int(0), false);
    c5(&mut v, "g5.while.trap[3]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif (i == 3u64) {\ns += (9u64 / (i - i))\n}\ns += 1u64\n}\nreturn (s as u64) as bytes", &[3], trap(90), false);
    c5(&mut v, "g5.while.breakonly[7]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (n == n) {\nif (i >= n) {\nbreak\n}\ni += 1u64\n}\nreturn (i as u64) as bytes", &[7], ok_int(7), false);
    c5(&mut v, "g5.while.count[0]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (i != n) {\ni += 1u64\n}\nreturn (i as u64) as bytes", &[0], ok_int(0), false);
    c5(&mut v, "g5.while.count[255]", "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (i != n) {\ni += 1u64\n}\nreturn (i as u64) as bytes", &[255], ok_int(255), true);
    v
}

/* ---------------- G6 (same as g6_types.rs) ---------------- */

const A_V1: &str = "RtqvBaudCALQXgZRiRUT8kuYCsjxPFk8P"; // version 1, payload 0x11

fn addr_v1() -> Vec<u8> {
    let mut v = vec![1u8];
    v.extend([0x11; 20]);
    v
}

fn g6_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let c6 = |v: &mut Vec<Case>, id: &'static str, body: &str, input: &[u8], want: Want, sweep: bool| {
        let c = case(id, body, input, want);
        v.push(if sweep { sw(c) } else { c });
    };

    c6(&mut v, "g6.is_pred", "var a = (byte(data, 0) as u64)\nvar r = 0u64\nif (a is nil) { r += 1u64 }\nif (a is u64) { r += 2u64 }\nif (a is u8) { r += 4u64 }\nif (a is bool) { r += 8u64 }\nif (a is bytes) { r += 16u64 }\nif (a is address) { r += 32u64 }\nif (a is list) { r += 64u64 }\nif (a is map) { r += 128u64 }\nvar nb = ((data is not bytes) as u64)\nvar lb = ((data is list) as u64)\nvar mb = ((data is map) as u64)\nreturn (r + nb * 256u64 + lb * 512u64 + mb * 1024u64) as bytes", &[7], ok_int(2), true);

    const TIDW: &str = "var w = (byte(data, 0) % 5u64)\nvar b = byte(data, 1)\nvar t = 0u8\nif (w == 0u64) { t = type_id(b as u8) }\nif (w == 1u64) { t = type_id(b as u16) }\nif (w == 2u64) { t = type_id(b as u32) }\nif (w == 3u64) { t = type_id(b as u64) }\nif (w == 4u64) { t = type_id(b as u128) }\nreturn ((t as u64) + (type_id(data) as u64) * 16u64 + (type_id(b < b) as u64) * 256u64) as bytes";
    for w in 0u8..5 {
        let want = (w as u128 + 2) + 8 * 16 + 256;
        c6(&mut v, Box::leak(format!("g6.tid.w{w}").into_boxed_str()), TIDW, &[w, 9], ok_int(want), w < 2);
    }

    const LOGIC: &str = "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = 0u64\nif (a == b) { r += 1u64 }\nif (a != b) { r += 2u64 }\nif (a < b) { r += 4u64 }\nif (a > b) { r += 8u64 }\nif (a <= b) { r += 16u64 }\nif (a >= b) { r += 32u64 }\nif (a > 1u64) && (b > 1u64) { r += 64u64 }\nif (a == 0u64) || (b == 0u64) { r += 128u64 }\nvar q = (!(a > b))\nif q { r += 256u64 }\nreturn (r) as bytes";
    for (input, want) in [([5u8, 9], 342u128), ([5, 5], 369), ([0, 9], 406), ([255, 1], 42)] {
        c6(&mut v, Box::leak(format!("g6.logic[{}-{}]", input[0], input[1]).into_boxed_str()), LOGIC, &input, ok_int(want), true);
    }

    const LOGIC8: &str = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = 0u64\nif (a == b) { r += 1u64 }\nif (a < b) { r += 2u64 }\nif (a > b) { r += 4u64 }\nreturn (r) as bytes";
    c6(&mut v, "g6.logic8.eq", LOGIC8, &[3, 3], ok_int(1), false);
    c6(&mut v, "g6.logic8.lt", LOGIC8, &[3, 4], ok_int(2), false);

    const NILCMP: &str = "var q = (byte(data, 0) as u64)\nif (q == 1u64) { var n = 5u64 }\nvar r = (n == 5u64)\nreturn (r as u64) as bytes";
    c6(&mut v, "g6.nilcmp.eq", NILCMP, &[0], trap(90), false);
    c6(&mut v, "g6.nilcmp.taken", NILCMP, &[1], ok_int(1), false);

    const ASBOOL: &str = "var a = (byte(data, 0) as u64)\nreturn ((a as bool) as u64) as bytes";
    for (input, want) in [(&[0u8][..], 0u128), (&[1], 1), (&[255], 1)] {
        c6(&mut v, Box::leak(format!("g6.asbool.u64[{}]", input[0]).into_boxed_str()), ASBOOL, input, ok_int(want), true);
    }
    const ASBOOLB: &str = "return ((data as bool) as u64) as bytes";
    c6(&mut v, "g6.asbool.bytes.empty", ASBOOLB, &[], ok_int(0), false);
    c6(&mut v, "g6.asbool.bytes.zeros", ASBOOLB, &[0, 0, 0], ok_int(0), false);
    c6(&mut v, "g6.asbool.bytes.nonzero", ASBOOLB, &[0, 0, 1], ok_int(1), false);

    const ASU8: &str = "return (data as u8) as bytes";
    c6(&mut v, "g6.asu8.fit", ASU8, &[5], ok_bytes(&[5]), true);
    c6(&mut v, "g6.asu8.lead-zero", ASU8, &[0, 5], ok_bytes(&[5]), false);
    c6(&mut v, "g6.asu8.ovf", ASU8, &[1, 5], trap(71), false);
    const ASU64: &str = "return (data as u64) as bytes";
    c6(&mut v, "g6.asu64.fit", ASU64, &[0, 0, 0, 0, 0, 0, 0, 9], ok_bytes(&[0, 0, 0, 0, 0, 0, 0, 9]), false);
    c6(&mut v, "g6.asu64.ovf", ASU64, &[1u8; 9], trap(71), false);

    const MULTI: &str = "var x = byte(data, 0)\nvar r = ((x as u8) as bytes) ++ ((x as u16) as bytes) ++ ((x as u32) as bytes) ++ ((x as u64) as bytes) ++ ((x as u128) as bytes) ++ ((x < x) as bytes)\nreturn data ++ r";
    {
        let input = [0x12u8];
        let x = input[0];
        let mut want = input.to_vec();
        want.push(x);
        want.extend((x as u16).to_be_bytes());
        want.extend((x as u32).to_be_bytes());
        want.extend((x as u64).to_be_bytes());
        want.extend((x as u128).to_be_bytes());
        want.push(0);
        c6(&mut v, "g6.multiwidth", MULTI, &input, ok_bytes(&want), true);
    }

    const ASADDR: &str = "var a = (data as address)\nreturn (a as bytes)";
    c6(&mut v, "g6.asaddr.v1", ASADDR, &addr_v1(), ok_bytes(&addr_v1()), false);
    let mut bad_ver = vec![2u8];
    bad_ver.extend([0x33; 20]);
    c6(&mut v, "g6.asaddr.ver2", ASADDR, &bad_ver, trap(71), false);
    c6(&mut v, "g6.asaddr.short", ASADDR, &[1, 0x11], trap(71), false);
    c6(&mut v, "g6.asaddr.long", ASADDR, &[1u8; 22], trap(71), false);

    const CAST8: &str = "var a = (byte(data, 0) as u64)\nvar r = ((a * 100u64) as u8)\nreturn (r as u64) as bytes";
    c6(&mut v, "g6.cast8.ovf", CAST8, &[9], trap(71), false);
    c6(&mut v, "g6.cast8.fit", CAST8, &[2], ok_int(200), false);

    const NILCB: &str = "var q = (byte(data, 0) as u64)\nif (q == 1u64) { var n = 5u64 }\nreturn (n) as bytes";
    c6(&mut v, "g6.nilcbytes", NILCB, &[0], trap(71), false);
    c6(&mut v, "g6.nilcbytes.taken", NILCB, &[1], ok_int(5), false);

    c6(&mut v, "g6.addr.literal", &format!("var a = {A_V1}\nreturn (a as bytes)"), &[], ok_bytes(&addr_v1()), false);
    c6(&mut v, "g6.addr.eq.same", &format!("var r = ({A_V1} == {A_V1})\nreturn ((r as u64)) as bytes"), &[], ok_int(1), false);
    c6(&mut v, "g6.addr.eq.len20", &format!("var r = (data == {A_V1})\nreturn ((r as u64)) as bytes"), &[1u8; 20], trap(90), false);
    v
}

/* ---------------- G7 (same as g7_list.rs) ---------------- */

fn g7_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let c7 = |v: &mut Vec<Case>, id: &'static str, body: &str, want: Want, sweep: bool| {
        let c = case(id, body, &[0u8], want);
        v.push(if sweep { sw(c) } else { c });
    };

    c7(&mut v, "g7.create", "var l = [1u64, 2u64, 3u64]\nvar i = (byte(data, 0) as u64) % 3u64\nvar r = l[i] + (length(l) as u64)\nreturn (r as u64) as bytes", ok_int(4), true);
    c7(&mut v, "g7.append.frame", "var l = []\nvar i = 0u64\nwhile i < 5u64 {\nappend(l, i * 11u64)\ni += 1u64\n}\nvar r = (length(l) as u64) * 100u64 + l[0] + l[4]\nreturn (r as u64) as bytes", ok_int(544), false);
    c7(&mut v, "g7.clear", "var l = [1u64, 2u64, 3u64]\nclear(l)\nvar l0 = (length(l) as u64)\nappend(l, 9u64)\nappend(l, 8u64)\nvar r = l0 * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes", ok_int(2), false);
    c7(&mut v, "g7.itemset", "var l = [1u64, 2u64, 3u64]\nl[1] = 9u64\nitem_set(l, 2u64, 7u64)\nvar r = (l[0] * 10u64 + l[1]) * 10u64 + l[2]\nreturn (r as u64) as bytes", ok_int(197), false);
    c7(&mut v, "g7.haskey", "var w = [4u64, 5u64]\nappend(w, 6u64)\nvar r = (has_key(w, 1u64) as u64) * 10u64 + (has_key(w, 9u64) as u64)\nreturn (r as u64) as bytes", ok_int(10), false);
    c7(&mut v, "g7.oob.read", "var l = [1u64, 2u64]\nvar i = (byte(data, 0) as u64) + 2u64\nvar r = l[i]\nreturn (r as u64) as bytes", trap(85), false);
    v.push(case("g7.idx72.itemget", "var l = [1u64, 2u64]\nvar i = (byte(data, 0) as u64) * 4294967296u64\nvar r = l[i]\nreturn (r as u64) as bytes", &[1u8], trap(72)));
    c7(&mut v, "g7.cap.append129", "var l = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(l, i)\ni += 1u64\n}\nappend(l, 5u64)\nreturn 0u64 as bytes", trap(40), false);
    c7(&mut v, "g7.cap.exact128", "var l = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(l, i)\ni += 1u64\n}\nreturn (length(l) as u64) as bytes", ok_int(128), false);
    c7(&mut v, "g7.nested.literal", "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes", trap(76), false);
    c7(&mut v, "g7.nested.append", "var a = [1u64]\nvar b = [2u64]\nappend(a, b)\nreturn 0u64 as bytes", trap(76), false);
    c7(&mut v, "g7.take.first-last", "var l = [10u64, 20u64, 30u64]\nvar a = (take_first(l) as u64)\nvar b = (take_last(l) as u64)\nvar r = a * 100u64 + b\nreturn (r as u64) as bytes", ok_int(1030), false);
    c7(&mut v, "g7.take.empty", "var l = []\nvar a = (take_first(l) as u64)\nreturn (a as u64) as bytes", trap(81), false);
    c7(&mut v, "g7.take.clone.iso", "var l = [1u64, 2u64]\nvar m = clone(l)\nvar a = (take_first(m) as u64)\nvar r = (length(l) as u64) * 10u64 + (length(m) as u64)\nreturn (r as u64) as bytes", ok_int(21), false);
    c7(&mut v, "g7.insert.middle", "var l = [1u64, 2u64, 3u64]\ninsert(l, 1u64, 99u64)\nvar r = ((l[0] * 1000u64 + l[1]) * 1000u64 + l[2]) * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes", ok_int(10990024), false);
    c7(&mut v, "g7.insert.pastlen", "var l = [1u64, 2u64]\ninsert(l, 3u64, 9u64)\nreturn 0u64 as bytes", trap(81), false);
    c7(&mut v, "g7.remove.middle", "var l = [1u64, 2u64, 3u64, 4u64]\nremove(l, 1u64)\nvar r = ((l[0] * 100u64 + l[1]) * 100u64 + l[2]) * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes", ok_int(103043), false);
    c7(&mut v, "g7.merge.basic", "var a = [1u64, 2u64]\nvar b = [3u64, 4u64]\nmerge(a, b)\nvar r = ((length(a) as u64) * 100u64 + a[2]) * 100u64 + a[3]\nreturn (r as u64) as bytes", ok_int(40304), false);
    c7(&mut v, "g7.merge.samerc", "var a = [1u64, 2u64]\nmerge(a, a)\nreturn 0u64 as bytes", trap(80), false);
    c7(&mut v, "g7.keys.on-list", "var l = [1u64, 2u64]\nvar m = keys(l)\nreturn 0u64 as bytes", trap(83), false);
    c7(&mut v, "g7.remove.to-empty", "var l = [4u64]\nremove(l, 0u64)\nvar a = (take_first(l) as u64)\nreturn (a as u64) as bytes", trap(81), false);
    v
}

/* ---------------- G8 (same as g8_map.rs, input [1,2,3,4]) ---------------- */

fn g8_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let input = [1u8, 2, 3, 4];
    let c8 = |v: &mut Vec<Case>, id: &'static str, body: &str, want: Want, sweep: bool| {
        let c = case(id, body, &input, want);
        v.push(if sweep { sw(c) } else { c });
    };

    c8(&mut v, "g8.newmap.len", "var m = new_map()\nreturn length(m) as bytes", ok_int(0), false);
    c8(&mut v, "g8.map1.len", "var m = map{1u64 : 2u64}\nreturn length(m) as bytes", ok_int(1), true);
    c8(&mut v, "g8.insert.itemget", "var m = new_map()\ninsert(m, 7u64, 40u64)\nreturn (m[7u64] as u64) as bytes", ok_int(40), true);
    c8(&mut v, "g8.overwrite", "var m = map{1u64 : 2u64}\ninsert(m, 1u64, 9u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes", ok_int(91), false);
    c8(&mut v, "g8.zero.key", "var m = new_map()\ninsert(m, 0u64, 5u64)\nreturn (m[0u64] as u64) as bytes", ok_int(5), false);
    c8(&mut v, "g8.itemset.overwrite", "var m = map{1u64 : 2u64, 2u64 : 3u64}\nitem_set(m, 2u64, 9u64)\nreturn ((m[2u64] as u64) * 10u64 + length(m)) as bytes", ok_int(92), false);
    c8(&mut v, "g8.index.assign", "var m = map{1u64 : 2u64}\nm[1u64] = 9u64\nreturn (m[1u64] as u64) as bytes", ok_int(9), false);
    c8(&mut v, "g8.keys.sorted", "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 100u64 + (ks[2] as u64)) as bytes", ok_int(103), true);
    c8(&mut v, "g8.values.order", "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar vs = values(m)\nreturn ((vs[0] as u64) * 100u64 + (vs[1] as u64)) as bytes", ok_int(1020), false);
    c8(&mut v, "g8.remove.reinsert", "var m = map{1u64 : 10u64, 2u64 : 20u64}\nremove(m, 1u64)\ninsert(m, 3u64, 30u64)\nreturn ((m[2u64] as u64) * 100u64 + (m[3u64] as u64)) as bytes", ok_int(2030), false);
    c8(&mut v, "g8.merge.values", "var a = map{1u64 : 2u64}\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn ((a[1u64] as u64) * 10u64 + (a[3u64] as u64)) as bytes", ok_int(24), true);
    c8(&mut v, "g8.alias.b", "var a = new_map()\nvar b = a\ninsert(b, 1u64, 2u64)\nreturn (a[1u64] as u64) as bytes", ok_int(2), false);
    c8(&mut v, "g8.missing.get", "var m = map{1u64 : 2u64}\nvar r = m[9u64]\nreturn r as bytes", trap(85), false);
    c8(&mut v, "g8.missing.remove", "var m = map{1u64 : 2u64}\nremove(m, 9u64)\nreturn 0u64 as bytes", trap(85), false);
    c8(&mut v, "g8.missing.set", "var m = map{1u64 : 2u64}\nm[9u64] = 3u64\nreturn 0u64 as bytes", trap(85), false);
    c8(&mut v, "g8.boolkey", "var m = new_map()\ninsert(m, (1u64 == 1u64), 5u64)\nreturn 0u64 as bytes", trap(73), false);
    c8(&mut v, "g8.emptybyteskey", "var m = new_map()\nvar e = buf_left(0, data)\ninsert(m, e, 5u64)\nreturn 0u64 as bytes", trap(73), false);
    c8(&mut v, "g8.dup.pack", "var m = map{1u64 : 2u64, 1u64 : 3u64}\nreturn length(m) as bytes", trap(84), false);
    c8(&mut v, "g8.dup.merge", "var a = map{1u64 : 2u64}\nvar b = map{1u64 : 3u64}\nmerge(a, b)\nreturn length(a) as bytes", trap(84), false);
    c8(&mut v, "g8.selfmerge", "var a = map{1u64 : 2u64}\nmerge(a, a)\nreturn length(a) as bytes", trap(80), false);
    c8(&mut v, "g8.capacity", "var m = new_map()\nvar i = 0u64\nwhile i < 129u64 { insert(m, i, i)\ni += 1 }\nreturn length(m) as bytes", trap(40), false);
    c8(&mut v, "g8.canon.collide", "var m = new_map()\ninsert(m, 1u64, 5u64)\ninsert(m, buf_left(1, data), 6u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes", ok_int(61), false);
    c8(&mut v, "g8.canon.zeroext", "var m = new_map()\ninsert(m, 65536u32, 9u64)\nreturn (m[65536u64] as u64) as bytes", ok_int(9), false);
    v.push(case("g8.byteskey.sort", "var m = new_map()\ninsert(m, buf_left(2, data), 1u64)\ninsert(m, buf_right(1, data), 2u64)\nvar ks = keys(m)\nreturn ks[0]", &[7u8, 9, 3, 4], ok_bytes(&[4])));
    v.push(case("g8.bytesvalue.itemget", "var m = new_map()\ninsert(m, 1u64, buf_left(3, data))\nreturn m[1u64]", &[7u8, 9, 3, 4], ok_bytes(&[7, 9, 3])));
    c8(&mut v, "g8.loop.sum", "var m = new_map()\nvar i = 0u64\nwhile i < 10u64 { insert(m, i, i)\ni += 1u64 }\nvar j = 0u64\nvar s = 0u64\nwhile j < 10u64 { s += m[j]\nj += 1u64 }\nreturn s as bytes", ok_int(45), true);
    v
}

/* ---------------- G9 (same as g9_assign.rs) ---------------- */

fn g9_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let c9 = |v: &mut Vec<Case>, id: &'static str, body: &str, input: &[u8], want: Want, sweep: bool| {
        let c = case(id, body, input, want);
        v.push(if sweep { sw(c) } else { c });
    };

    c9(&mut v, "g9.byte_reads", "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar d = (byte(data, 3) as u64)\nreturn (((a * 256u64 + b) * 256u64 + c) * 256u64 + d) as bytes", &[7, 9, 11, 13], ok_bytes(&hex_bytes("0000000007090b0d")), true);
    c9(&mut v, "g9.slice_family", "var a = (byte(data, 0) as u64)\nvar c1 = buf_cut(data, 1u64, 2u64)\nvar c2 = buf_cut(data, 0u64, 1u64)\nvar l = buf_left(2, data)\nvar r = buf_right(2, data)\nvar ld = buf_left_drop(1, data)\nvar rd = buf_right_drop(1, data)\nvar r1 = (a * 1000u64 + (size(c1) as u64)) * 1000u64 + (size(c2) as u64)\nvar r2 = ((size(l) as u64) * 1000u64 + (size(r) as u64)) * 1000u64 + (size(ld) as u64) * 1000u64 + (size(rd) as u64)\nreturn (r1 * 1000000u64 + r2) as bytes", &[7, 9, 11, 13], ok_int(7_002_003_005_003), false);
    c9(&mut v, "g9.data_rebind", "data = buf_right(1, data)\nreturn (byte(data, 0) as u64) as bytes", &[7, 9, 1, 2], ok_bytes(&be64(2)), false);
    c9(&mut v, "g9.rebind.both", "var a = (byte(data, 0) as u64)\ndata = 0x0506\nvar b = (byte(data, 0) as u64)\nreturn (((a * 100u64) + b) as u64) as bytes", &[7, 9, 1, 2], ok_int(705), false);
    c9(&mut v, "g9.rebind.twice", "data = buf_left(2, data)\ndata = data ++ 0xff\nreturn data", &[1, 2, 3, 4], ok_bytes(&[1, 2, 0xff]), false);

    let big = "ab".repeat(300);
    let buf_pbuf = format!(
        "var e = \"\"\nvar s = \"abcdefgh\"\nvar big = 0x{big}\nvar b = \"aaaaaaaa\"\nvar o = (byte(data, 0) as u64) % 8u64\nvar x = buf_set(b, o, 17u8)\nvar r1 = ((size(e) as u64) * 1000000u64 + (size(s) as u64)) * 1000u64 + (size(big) as u64)\nvar r2 = (byte(x, o) as u64) * 10u64 + (byte(b, o) as u64)\nreturn (r1 * 1000u64 + r2) as bytes"
    );
    c9(&mut v, "g9.buf_pbuf", &buf_pbuf, &[3, 1, 2, 4], ok_int(8_300_267), false);

    c9(&mut v, "g9.bufset.res", "var b = \"abcdefgh\"\nvar x = buf_set(b, 2u64, 85u8)\nvar r = (byte(x, 2) as u64) * 10u64 + (byte(b, 2) as u64)\nreturn (r as u64) as bytes", &[0, 0, 0, 0], ok_int(85 * 10 + 99), false);
    c9(&mut v, "g9.compound.rhsfirst", "var x $70 = (byte(data, 0) as u64)\nx += (x * 10u64)\nreturn x as bytes", &[5, 0, 0, 0], ok_int(55), false);
    c9(&mut v, "g9.compound.u128.div", "var x $64 = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx /= 3u128\nx -= 1u128\nreturn x as bytes", &[3, 0, 0, 0], ok_bytes(&hex_bytes("0000000000000000ffffffffffffffff")), false);
    c9(&mut v, "g9.compound.u128.add", "var x $64 = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx += 5u128\nreturn x as bytes", &[1, 0, 0, 0], ok_bytes(&hex_bytes("00000000000000010000000000000005")), false);

    c9(&mut v, "g9.compound.div0", "var x $64 = (byte(data, 0) as u64)\nx /= 0u64\nreturn x as bytes", &[1, 2, 3, 4], trap(90), false);
    c9(&mut v, "g9.compound.addovf", "var x $64 = 18446744073709551615u64\nx += 1u64\nreturn x as bytes", &[1, 2, 3, 4], trap(90), false);
    c9(&mut v, "g9.compound.subunder", "var x $64 = ((byte(data, 0) as u64) % 3u64)\nx -= 10u64\nreturn x as bytes", &[1, 2, 3, 4], trap(90), false);
    c9(&mut v, "g9.compound.mulovf", "var x $64 = 18446744073709551615u64\nx *= 2u64\nreturn x as bytes", &[1, 2, 3, 4], trap(90), false);
    c9(&mut v, "g9.itemset.oob", "var l = [7u64, 8u64]\nvar i = (byte(data, 0) as u64) % 4u64\nl[i] = 5u64\nvar r = l[0] + l[1]\nreturn (r as u64) as bytes", &[2, 0, 0, 0], trap(85), false);
    c9(&mut v, "g9.itemset.u32", "var l = [7u64, 8u64]\nvar i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\nl[i] = 5u64\nreturn 0u64 as bytes", &[0, 0, 0, 0], trap(72), false);
    c9(&mut v, "g9.itemset.rhstrap", "var l = [7u64, 8u64]\nl[0] += (1u64 / (byte(data, 1) - byte(data, 1)))\nreturn 0u64 as bytes", &[1, 5, 0, 0], trap(90), false);
    c9(&mut v, "g9.itemset.addovf", "var l = [18446744073709551615u64, 2u64]\nl[0] += (byte(data, 0) as u64) + 1u64\nvar r = l[0] + l[1]\nreturn (r as u64) as bytes", &[255, 0, 0, 0], trap(90), false);
    c9(&mut v, "g9.bufset.nil", "var q = 0u64\nif q == 1u64 { var t = \"ab\" }\nbuf_set(t, 0u64, 5u8)\nreturn 0u64 as bytes", &[1, 2, 3, 4], trap(91), false);
    c9(&mut v, "g9.map.missing.set", "var m = map{1u64 : 2u64}\nm[9u64] = 3u64\nreturn 0u64 as bytes", &[1, 2, 3, 4], trap(85), false);
    c9(&mut v, "g9.map.missing.get", "var m = map{1u64 : 2u64}\nvar r = m[9u64]\nreturn r as bytes", &[1, 2, 3, 4], trap(85), false);
    v
}

fn corpus() -> Vec<Case> {
    let mut v = Vec::new();
    v.extend(g1_cases());
    v.extend(g2_cases());
    v.extend(g3_cases());
    v.extend(g4_cases());
    v.extend(g5_cases());
    v.extend(g6_cases());
    v.extend(g7_cases());
    v.extend(g8_cases());
    v.extend(g9_cases());
    v
}

/* ================================================================== */
/* Main differential suite                                              */
/* ================================================================== */

#[test]
fn calc_aot_differential_suite() {
    let cases = corpus();
    let mut cache: EnginesCache = HashMap::new();
    let mut n_exec = 0usize;
    let mut n_agree = 0usize;
    let mut n_golden = 0usize;
    let mut n_traps = 0usize;
    let mut trap_rows: Vec<String> = Vec::new();
    let mut bodies: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for c in &cases {
        let mut modes = vec![Rv64Mode::ImcRemap];
        if c.sweep3 {
            modes.push(Rv64Mode::Im);
            modes.push(Rv64Mode::Imc);
        }
        for m in modes {
            let eng = engines_for(&mut cache, &c.body, m);
            bodies.insert(format!("{m:?}:{}", c.body));
            let (i, _msg) = interp_face(&eng, &c.input);
            let a = aot_face(&eng, &c.input);
            assert_faces_agree(c.id, &c.input, &c.body, &i, &a);
            n_exec += 1;
            n_agree += 1;

            // Production mounting face: path-2 (interpreter CalcFnObj) vs path-3 (AOT mount).
            let (li, la) = fnobj_pair(&eng, &c.input);
            assert_eq!(
                li.result, la.result,
                "[{} mode {m:?}] CalcFnObj consensus surface mismatch (interp vs AOT-mounted): {:?} vs {:?}",
                c.id, li.result, la.result
            );
            assert_eq!(li.cycles_used, la.cycles_used, "[{} mode {m:?}] CalcFnObj cycles mismatch", c.id);
            assert_eq!(li.exit_code, la.exit_code, "[{} mode {m:?}] CalcFnObj exit_code mismatch", c.id);

            // golden (guards against both engines erring together).
            match &c.want {
                Want::OkBytes(b) => {
                    assert_eq!(
                        i.ok.as_deref(),
                        Some(b.as_slice()),
                        "[{} mode {m:?}] golden bytes drifted: interp=0x{} want=0x{}",
                        c.id,
                        i.ok.as_ref().map(|o| hex::encode(o)).unwrap_or_default(),
                        hex::encode(b)
                    );
                    n_golden += 1;
                }
                Want::OkInt(v) => {
                    let o = i.ok.as_ref().expect("[{c.id}] golden expects Ok output");
                    let got = o.iter().fold(0u128, |acc, x| (acc << 8) | *x as u128);
                    assert_eq!(got, *v, "[{} mode {m:?}] golden BE-int drifted", c.id);
                    n_golden += 1;
                }
                Want::Trap(code) => {
                    assert!(
                        matches!(i.abort, Some(CalcAbort2::Trap)),
                        "[{} mode {:?}] golden expects a Trap-class abort, got {}",
                        c.id, m, face_summary(&i)
                    );
                    // Code-level pinning: fitsh encodes the consensus fault code
                    // into the nonzero a0 of the guest HALT; both engines' exit_code
                    // is the engine-level code — equal case-by-case to the consensus
                    // codes pinned by the campaign's HVM channel (see the module header).
                    assert_eq!(
                        i.exit_code, *code as u64,
                        "[{} mode {:?}] engine-level fault code drifted (interp exit_code {} vs campaign consensus {})",
                        c.id, m, i.exit_code, code
                    );
                    assert_eq!(
                        a.exit_code, *code as u64,
                        "[{} mode {:?}] engine-level fault code drifted (aot exit_code {} vs campaign consensus {})",
                        c.id, m, a.exit_code, code
                    );
                    trap_rows.push(format!(
                        "  {:<30} code={:<3} cycles={:<6} exit_code(interp)=exit_code(aot)={}",
                        c.id, code, i.cycles_used, i.exit_code
                    ));
                    n_traps += 1;
                }
                Want::Parity => {}
            }
        }
    }

    // ---- stats record (println) ----
    let sweep_bodies = cases.iter().filter(|c| c.sweep3).count();
    let unique_bodies: std::collections::HashSet<&String> = cases.iter().map(|c| &c.body).collect();
    println!("\n== calc AOT differential report ==");
    println!("corpus unique function bodies: {}", unique_bodies.len());
    println!("corpus cases (body x input): {} (of which Im/Imc sweep3: {})", cases.len(), sweep_bodies);
    println!("compiled (body x mode) images: {}", bodies.len());
    println!("executions (case x mode): {n_exec}");
    println!("interp-AOT agreements: {n_agree} (mismatches: {})", n_exec - n_agree);
    println!("golden value checks: {n_golden}");
    println!("trap-face executions: {n_traps} (pinned to CODE level: CalcAbort2::Trap class + exit_code == campaign consensus fault code, on both engines, + cycles + full state)");
    println!("trap-face table (code-level pin: engine exit_code == consensus fault code per case):");
    for r in &trap_rows {
        println!("{r}");
    }
}

/* ================================================================== */
/* Dedicated failure face: gas metering boundary (OutOfGas), code-level agreement */
/* ================================================================== */

/// Sweep the OutOfGas boundary on a shrinking budget: at every budget point
/// both engines' (abort class, cycles_used, full machine state) must be
/// bit-identical — the only engine-level failure face carrying a "code"
/// (OutOfGas); the gas metering boundary is pinned down to the cycles level.
#[test]
fn calc_aot_out_of_gas_boundary_parity() {
    let body = "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (i != n) {\ni += 1u64\n}\nreturn (i as u64) as bytes";
    let input = [255u8];
    let eng = build_engines(body, Rv64Mode::ImcRemap);

    let (full, _) = interp_face(&eng, &input);
    let full_cycles = full.cycles_used;
    assert!(full.abort.is_none(), "full-budget run must succeed");

    for budget in [full_cycles, full_cycles - 1, full_cycles / 2, 500, 100, 10, 3, 1, 0] {
        let (ires, aregs, amem) = interp::run_lockstep(&eng.module, &eng.container, &input, budget, None);
        let (aout, aregs2, amem2) = eng.aot.run_with_state(&eng.module, &eng.container, &input, budget);
        let icls = match &ires {
            Ok(_) => None,
            Err((a, _)) => Some(match a {
                CalcAbort::Trap(_) => CalcAbort2::Trap,
                CalcAbort::GuestExit(_) => CalcAbort2::Trap,
                CalcAbort::OutOfGas => CalcAbort2::OutOfGas,
                CalcAbort::Deadline => panic!("no deadline"),
            }),
        };
        let icycles = match &ires {
            Ok(r) => r.cycles_used,
            Err((_, u)) => *u,
        };
        assert_eq!(
            icls, aout.abort,
            "budget {budget}: abort class mismatch (interp {icls:?} vs aot {:?})",
            aout.abort
        );
        assert_eq!(
            icycles, aout.cycles_used,
            "budget {budget}: cycles_used mismatch (interp {icycles} vs aot {}) — OutOfGas boundary gas drift",
            aout.cycles_used
        );
        assert_eq!(aregs, aregs2, "budget {budget}: register file mismatch at stop point");
        assert_eq!(amem, amem2, "budget {budget}: memory image mismatch at stop point");
        if icls == Some(CalcAbort2::OutOfGas) {
            println!("out-of-gas face: budget={budget} cycles_used={icycles} (interp == aot, class + gas + state pinned)");
        }
    }
}

/// input_max precheck face: input above the declared cap → both engines Trap,
/// cycles 0, empty state; the CalcFnObj precheck path also Traps.
#[test]
fn calc_aot_input_overflow_face() {
    let body = "return (byte(data, 0) as u64) as bytes";
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let m = Rv64imBodyMeta { input_max: 4, ..Rv64imBodyMeta::default() };
    let bytes = compile_rv64im_body_kernel("aot_diff_small_in", tokens, Some("data"), &[], &[], &[], &m, &cap())
        .expect("compiles");
    let (container, _) = CalcContainer::decode(&bytes).expect("decodes");
    let module = verify_container_with_cap(&container, &cap()).expect("verify");
    let aot = aot_compile(&module).expect("aot compiles");
    let input = [0u8; 8];

    let (ires, iregs, imem) = interp::run_lockstep(&module, &container, &input, BUDGET, None);
    let (aout, aregs, amem) = aot.run_with_state(&module, &container, &input, BUDGET);
    let err = ires.expect_err("interp must reject oversized input");
    assert!(matches!(err.0, CalcAbort::Trap(_)));
    assert_eq!(err.1, 0, "interp oversized-input cycles must be 0");
    assert_eq!(aout.abort, Some(CalcAbort2::Trap), "aot must reject oversized input");
    assert_eq!(aout.cycles_used, 0, "aot oversized-input cycles must be 0");
    assert_eq!(iregs, aregs, "register state must agree (zeroed)");
    assert_eq!(imem, amem, "memory state must agree (empty)");

    // CalcFnObj precheck path.
    let fno = CalcFnObj::build(
        CalcContainer::decode(&bytes).expect("decodes").0,
        &cap(),
        CalcHostConfig::new().with_aot_mount(true),
    )
    .expect("build");
    let ex = fno.run(&input, limits());
    assert_eq!(ex.result, Err(CalcAbort2::Trap), "preflight must Trap");
    assert_eq!(ex.cycles_used, 0, "preflight charges no cycles");
}
