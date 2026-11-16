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
//!      interpreter accepts and runs (S_choose_mixed_arm +
//!      run_campaign mixed_arm routing). Bare value-expression statements
//!      stay frozen (P19a/P19b subset boundary; generated form would land
//!      in the reject family).
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

#[path = "c3lib/mod.rs"]
pub mod c3lib;

use c3lib::*;

use vm::fitshc::{compile_body, CompiledCode};
use vm::ir::rv64::{lower_body_mode_arena, ARENA_SIZE_MAX};
use vm::lang::Tokenizer;
use vm::ValueTy;

/* The test tiers. All test bodies are verbatim from the P1-C2/P2-F2
 * deliverable; the generator/harness/classifier now live in c3lib
 * (extracted by P2-C3 so the coverage instrumentation binary
 * rv64_c3_coverage.rs shares the exact same generator). */
/// D9 — CHOOSE/ternary arm EAGERNESS (FIXED — flipped 2026-11-13, P2-F2):
/// the bytecode channel evaluates ALL THREE operands of a CHOOSE
/// IRNodeTriple eagerly (compile.rs compile_triple handles only
/// IRIF|IRIFR; CHOOSE → eager IRNodeTriple::codegen_into; the
/// interpreter's CHOOSE swaps among three already-evaluated stack values).
/// Pre-fix: rv64 lowered IRIFR|CHOOSE branch-lazily, so a trapping UNTAKEN
/// arm faulted the interpreter only (a consensus-visible trap skipped).
/// FIX: rv64's CHOOSE lowering evaluates cond → yes → no in bytecode stack
/// order, then branch-selects into the cond's positions — the untaken
/// arm's traps now fire exactly where the interpreter fires them. The
/// VALUE-IF (IRIFR) stays branch-lazy: compile_if lowers it lazily on the
/// bytecode side too (two engines agree; see P1d-adjacent value-if probe
/// in c2_probe_engine_edges).
#[test]
fn c2_probe_choose_arm_evaluation() {
    let zeros = vec![0u8; 16];
    let pre = dec2();
    let body_a = format!("{pre}var zr = (c1 - c1)\nvar r = (c0 != 0u64) ? (7u64 / zr) : 42u64\nreturn r as bytes");
    let obs_a = probe(&body_a, &zeros);
    println!("P1a trap-in-yes-arm cond-false: {obs_a}");
    assert!(
        obs_a.contains("interp Err(90:"),
        "interpreter must evaluate the UNTAKEN yes arm eagerly: {obs_a}"
    );
    assert!(obs_a.contains("Im=Err(90)") && obs_a.contains("Imc=Err(90)") && obs_a.contains("ImcRemap=Err(90)"),
        "post-D9-fix: every rv64 mode must fault 90 at the eagerly-evaluated yes arm: {obs_a}");

    let body_b = format!("{pre}var zr = (c1 - c1)\nvar r = (c0 == 0u64) ? 42u64 : (7u64 / zr)\nreturn r as bytes");
    let obs_b = probe(&body_b, &zeros);
    println!("P1b trap-in-no-arm cond-false: {obs_b}");
    assert!(obs_b.contains("interp Err(90:"), "{obs_b}");
    assert!(obs_b.contains("Im=Err(90)") && obs_b.contains("Imc=Err(90)") && obs_b.contains("ImcRemap=Err(90)"),
        "post-D9-fix: every rv64 mode must fault 90 at the eagerly-evaluated no arm: {obs_b}");

    let body_c = format!("{pre}var zr = (c1 - c1)\nvar r = choose((c0 != 0u64), (7u64 / zr), 42u64)\nreturn r as bytes");
    let obs_c = probe(&body_c, &zeros);
    println!("P1c choose() builtin trap-arm: {obs_c}");
    assert!(obs_c.contains("interp Err(90:"), "{obs_c}");
    assert!(obs_c.contains("Im=Err(90)") && obs_c.contains("Imc=Err(90)") && obs_c.contains("ImcRemap=Err(90)"),
        "post-D9-fix: choose() must fault 90 eagerly in every rv64 mode: {obs_c}");

    // contrast: the VALUE-IF (IRIFR) stays branch-lazy on BOTH engines —
    // the untaken arm's trap must NOT fire anywhere
    let body_e = format!("{pre}var zr = (c1 - c1)\nvar r = if (c0 != 0u64) {{ (7u64 / zr) }} else {{ 42u64 }}\nreturn r as bytes");
    let obs_e = probe(&body_e, &zeros);
    println!("P1e value-if lazy on both engines: {obs_e}");
    let want_e = "Ok([0, 0, 0, 0, 0, 0, 0, 42])";
    assert!(obs_e.contains(want_e) && obs_e.matches(want_e).count() == 4,
        "value-if must stay lazy on both engines (interpreter included): {obs_e}");

    // sanity: trap-free arms agree byte-for-byte on all four engines
    let body_d = format!("{pre}var r = (c0 != 0u64) ? 11u64 : 22u64\nreturn r as bytes");
    let obs_d = probe(&body_d, &zeros);
    println!("P1d safe arms: {obs_d}");
    let want = "Ok([0, 0, 0, 0, 0, 0, 0, 22])";
    assert!(obs_d.contains(want) && obs_d.matches(want).count() == 4, "{obs_d}");
}

/// The C1 mapping table rows as executable comparator assertions — every
/// pinned divergence family exercised through the classifier, including the
/// new D10 row and the widened any-rv64-outcome semantics.
#[test]
fn c2_probe_pinned_divergence_rows() {
    // D1: shift ≥ 64 — interpreter Arithmetic(90), rv64 masks to 63 (Ok).
    let body = "var c0 = (byte(data, 0) as u64)\nvar r = (c0 << 64u64)\nreturn r as bytes";
    let it = Interp::try_build(body).unwrap();
    let rv = Rv64::try_build(body).unwrap();
    let v = knock_one(&rv, &it, &Feat::default(), &[1u8]);
    assert_eq!(v, Verdict::PinD1ShiftMask, "D1 row (90 vs Ok)");

    // D1 widened: masked shift followed by a LATER rv64 trap (the interpreter
    // died at the shift; rv64 continues into `require 5000000` → exit
    // 5000000). Any rv64 outcome is the pinned semantics.
    let body_ls =
        format!("{}var r = ((c0 << 100u64) != 0u64)\nrequire 5000000u64, r\nreturn 1u64 as bytes", word_dec("c0", 0));
    let it_ls = Interp::try_build(&body_ls).unwrap();
    let rv_ls = Rv64::try_build(&body_ls).unwrap();
    let v_ls = knock_one(&rv_ls, &it_ls, &Feat::default(), &[1u8; 8]);
    assert_eq!(v_ls, Verdict::PinD1ShiftMask, "D1 widened row (90 vs later-trap)");

    // D2: byte OOB — empty input (42 vs 71) and index ≥ 2¹⁶ (72 vs 71).
    let body = "var r = (byte(data, 0) as u64)\nreturn r as bytes";
    let it = Interp::try_build(body).unwrap();
    let rv = Rv64::try_build(body).unwrap();
    let v = knock_one(&rv, &it, &Feat::default(), &[]);
    assert_eq!(v, Verdict::PinD2BoundsClass, "D2 empty-input row");
    let body2 = format!("{}var r = (byte(data, c0) as u64)\nreturn r as bytes", dec2());
    let it2 = Interp::try_build(&body2).unwrap();
    let rv2 = Rv64::try_build(&body2).unwrap();
    let v2 = knock_one(&rv2, &it2, &Feat::default(), &[0xFFu8; 16]);
    assert_eq!(v2, Verdict::PinD2BoundsClass, "D2 huge-index row (72 vs 71)");

    // D3: assert failure / abort — 151(Nil) vs exit 1.
    let body = "assert (1u64 == 2u64)\nreturn 1u64 as bytes";
    let it = Interp::try_build(body).unwrap();
    let rv = Rv64::try_build(body).unwrap();
    let v = knock_one(&rv, &it, &Feat::default(), &[0u8; 4]);
    assert_eq!(v, Verdict::PinD3AssertAbort, "D3 assert row");
    // NOTE: bare `abort` followed by a return is a FRONTEND reject
    // ("unreachable code after terminal statement") — guarded abort instead.
    let body_ab = "if (1u64 == 1u64) { abort }\nreturn 1u64 as bytes";
    let it_ab = Interp::try_build(body_ab).unwrap();
    let rv_ab = Rv64::try_build(body_ab).unwrap();
    let v_ab = knock_one(&rv_ab, &it_ab, &Feat::default(), &[0u8; 4]);
    assert_eq!(v_ab, Verdict::PinD3AssertAbort, "D3 abort row");

    // D4: churn beyond BOTH budgets — 39 vs 71 (60 × 200 B).
    let feat_ch = Feat { churn: true, rdrop_cat: true, ..Feat::default() };
    let body_ch = "var b = buf_left(200, data)\nvar i = 0u64\nwhile i < 60 { b = b ++ buf_left(200, data)\ni += 1 }\nreturn (size(b) as u64) as bytes";
    let it_ch = Interp::try_build(body_ch).unwrap();
    let rv_ch = Rv64::try_build(body_ch).unwrap();
    let big: Vec<u8> = (0..256).map(|i| (i % 251) as u8).collect();
    let v_ch = knock_one(&rv_ch, &it_ch, &feat_ch, &big);
    assert_eq!(v_ch, Verdict::PinD4ChurnBand, "D4 row (39 vs 71)");

    // D4 band: churn past the interpreter's 1280 B value cap but inside the
    // rv64 arena — 39 vs Ok (threshold-band extension, documented).
    let body_band = "var b = buf_left(200, data)\nvar i = 0u64\nwhile i < 10 { b = b ++ buf_left(200, data)\ni += 1 }\nreturn (size(b) as u64) as bytes";
    let it_band = Interp::try_build(body_band).unwrap();
    let rv_band = Rv64::try_build(body_band).unwrap();
    let v_band = knock_one(&rv_band, &it_band, &feat_ch, &big);
    assert_eq!(v_band, Verdict::PinD4ChurnBand, "D4 band row (39 vs Ok)");

    // D5: narrow-width arithmetic — u8×u8 faults on the interpreter
    // (common-width u8, payload U8(..)) while rv64 computes in u64.
    let body_n = "var r = ((byte(data, 0) * byte(data, 1)) as u64)\nreturn r as bytes";
    let it_n = Interp::try_build(body_n).unwrap();
    let rv_n = Rv64::try_build(body_n).unwrap();
    let v_n = knock_one(&rv_n, &it_n, &Feat::default(), &[255u8, 255, 0, 0]);
    assert_eq!(v_n, Verdict::PinD5NarrowWidth, "D5 row (interp 90 U8-payload vs rv64 Ok)");

    // D10 (FIXED, P2-F2): clamp(x, lo, hi) with lo > hi — interpreter
    // Arithmetic(90) "clamp check failed" (instruction.rs clamp_checked
    // rejects lo>hi); pre-fix rv64 silently computed max(min(x,hi),lo).
    // Post-BUG-010-fix the row classifies as a plain 90-agreement.
    let body_c10 =
        format!("{}var r = clamp(c0, 100u64, 10u64)\nreturn r as bytes", word_dec("c0", 0));
    let it_c10 = Interp::try_build(&body_c10).unwrap();
    let rv_c10 = Rv64::try_build(&body_c10).unwrap();
    let v_c10 = knock_one(&rv_c10, &it_c10, &Feat::default(), &[7u8; 16]);
    assert_eq!(v_c10, Verdict::AgreeTrap90, "clamp lo>hi now agrees (both 90)");

    // agree rows: overflow / div-zero / cast-overflow / require / throw.
    let of = format!("{pre_word}var r = (a + 1u64)\nreturn r as bytes", pre_word = word_dec("a", 0));
    let it_o = Interp::try_build(&of).unwrap();
    let rv_o = Rv64::try_build(&of).unwrap();
    let mut mx = vec![0xFFu8; 8];
    mx.extend_from_slice(&[0; 8]);
    assert_eq!(knock_one(&rv_o, &it_o, &Feat::default(), &mx), Verdict::AgreeTrap90);
    let dz = format!("{pre}var zr = (c1 - c1)\nvar r = (7u64 / zr)\nreturn r as bytes", pre = dec2());
    let it_d = Interp::try_build(&dz).unwrap();
    let rv_d = Rv64::try_build(&dz).unwrap();
    assert_eq!(knock_one(&rv_d, &it_d, &Feat::default(), &[0u8; 16]), Verdict::AgreeTrap90);
    let cf = format!("{pre_word}var r = ((a as u8) as u64)\nreturn r as bytes", pre_word = word_dec("a", 0));
    let it_c = Interp::try_build(&cf).unwrap();
    let rv_c = Rv64::try_build(&cf).unwrap();
    assert_eq!(knock_one(&rv_c, &it_c, &Feat::default(), &[255u8; 8]), Verdict::AgreeTrap71);
    let rq = "require 42u64, (1u64 == 2u64)\nreturn 1u64 as bytes";
    let it_r = Interp::try_build(rq).unwrap();
    let rv_r = Rv64::try_build(rq).unwrap();
    assert_eq!(knock_one(&rv_r, &it_r, &Feat::default(), &[]), Verdict::AgreeUserValue(42));
    let th = "throw 18446744073709551615u64";
    let it_t = Interp::try_build(th).unwrap();
    let rv_t = Rv64::try_build(th).unwrap();
    assert_eq!(
        knock_one(&rv_t, &it_t, &Feat::default(), &[]),
        Verdict::AgreeUserValue(u64::MAX)
    );
}

/// Candidate-bug repro pins (PINNED-UNFIXED, 89-bug-db convention): each
/// assertion records TODAY'S exact observed behavior for a minimized repro.
/// When the backend is fixed, flip the marked assertions to the correct
/// semantics (the flip is documented inline per case).
#[test]
fn c2_probe_candidate_bug_pins() {
    // ---- BUG-010 regression pin (FIXED — flipped 2026-11-13, P2-F2) ----
    // Minimal repro: `clamp(c0, 100u64, 10u64)`, input c0 = 7.
    // Pre-fix (2026-11-13, cd53275): interpreter Arithmetic(90)
    // "arithmetic clamp check failed with U64(7), U64(100) and U64(10)"
    // (interpreter/instruction.rs `clamp_checked`: `if lo > hi { Err }`),
    // while rv64 lowered CLAMP as an unchecked min/max chain and returned
    // 100 (= lo) in ALL THREE modes.
    // FIX: the CLAMP lowering emits `bltu hi, lo → fault_arith` before the
    // min/max chain (rv64.rs CLAMP arm). Post-fix pin: all modes Err(90),
    // and the lo<=hi control region still agrees bit-for-bit.
    let body = format!("{}var r = clamp(c0, 100u64, 10u64)\nreturn r as bytes", word_dec("c0", 0));
    let it = Interp::try_build(&body).unwrap();
    let rv = Rv64::try_build(&body).unwrap();
    let ir = it.run(&[7u8; 16]);
    let rr = rv.run(&[7u8; 16]);
    let interp_fault = ir.as_ref().expect_err("interpreter must fault clamp lo>hi with 90");
    assert_eq!(interp_fault.code, 90);
    assert!(
        interp_fault.msg.contains("clamp check failed"),
        "interpreter clamp message: {}",
        interp_fault.msg
    );
    for (mode, r) in &rr {
        assert_eq!(
            r.as_ref().err(),
            Some(&90u64),
            "BUG-014-style regression: rv64 {mode:?} must trap 90 on clamp lo>hi"
        );
    }
    // control: well-ordered bounds still clamp identically on all engines
    let body_ok = format!("{}var r = clamp(c0, 10u64, 100u64)\nreturn r as bytes", word_dec("c0", 0));
    let it_ok = Interp::try_build(&body_ok).unwrap();
    let rv_ok = Rv64::try_build(&body_ok).unwrap();
    for inp in [0u8; 16].chunks(8).chain([5u8; 16].chunks(8)).chain([0xFFu8; 16].chunks(8)) {
        let full: Vec<u8> = inp.repeat(2);
        let want = it_ok.run(&full).expect("control clamp run");
        for (mode, r) in rv_ok.run(&full) {
            assert_eq!(r.as_ref().expect("control clamp run"), &want, "{mode:?}");
        }
    }
    println!("BUG-010 fixed pin: clamp lo>hi traps 90 in all modes; ordered bounds agree");

    // ---- BUG-011 regression pin (FIXED — flipped 2026-11-13, P2-F2) ----
    // Minimal repro: two bytes vars, `var r = if (c0 != 0) { a } else { b }`,
    // return r. Pre-fix (2026-11-13, cd53275): the lowerer ACCEPTED the body
    // in all three modes and MISCOMPILED it three ways — Im=Err(71),
    // Imc=Err(71), ImcRemap=Err(0) — while the interpreter returned the
    // taken arm [7, 7]. Root cause: the value-if join re-pushed the arm
    // locations in `.rev()` order, swapping the (ptr, len) pair; and
    // take_arm_values' push/pop-per-value materialization aliased a Ref
    // bytes pair onto one register (len clobbered ptr).
    // FIX: take_arm_values materializes each Ref into its OWN consecutive
    // pool position; the join re-pushes bottom-first. Post-fix pin: all
    // three rv64 modes return the taken arm, byte-for-byte with the
    // interpreter, on both condition outcomes.
    let pre = dec2();
    let body17 = format!(
        "{pre}var a = buf_left(2, data)\nvar b2 = buf_right(2, data)\nvar r = if (c0 != 0u64) {{ a }} else {{ b2 }}\nreturn r"
    );
    let it17 = Interp::try_build(&body17).unwrap();
    let rv17 = Rv64::try_build(&body17).unwrap();
    for (name, input) in [("taken-then", [7u8; 16]), ("taken-else", [0u8; 16])] {
        let ir17 = it17.run(&input).expect("interpreter returns the taken arm");
        for (mode, r) in rv17.run(&input) {
            assert_eq!(
                r.as_ref().expect("rv64 bytes value-if must succeed"),
                &ir17,
                "BUG-011 regression: {mode:?} ({name})"
            );
        }
    }
    // bytes value-if on BANKED bytes slots (ImcRemap): arms are Refs, which
    // is the materialization path the old form clobbered
    let body_bank = format!(
        "{pre}var a = buf_left(2, data)\nvar b2 = buf_right(2, data)\nvar c = buf_left(1, data)\nvar d = buf_right(1, data)\nvar e = buf_left(3, data)\nvar f = buf_right(3, data)\nvar r = if (c0 != 0u64) {{ a }} else {{ b2 }}\nvar r2 = if (c1 != 0u64) {{ c }} else {{ d }}\nvar r3 = if (c0 != 0u64) {{ e }} else {{ f }}\nreturn r ++ r2 ++ r3"
    );
    let it_b = Interp::try_build(&body_bank).unwrap();
    let rv_b = Rv64::try_build(&body_bank).unwrap();
    for (name, input) in [("all-then", [7u8; 16]), ("all-else", [0u8; 16])] {
        let want = it_b.run(&input).expect("banked interp run");
        for (mode, r) in rv_b.run(&input) {
            assert_eq!(r.as_ref().expect("banked bytes value-if"), &want, "{mode:?} ({name})");
        }
    }
    println!("BUG-011 fixed pin: bytes value-if returns the taken arm in all modes (incl. banked Ref arms)");

    // ---- BUG-012 regression pin (FIXED — flipped 2026-11-13, P2-F2) ----
    // Minimal repro: `var t9 = 5u64` lives inside an if-arm that is NOT
    // taken; `t9` is read after the arm. Pre-fix (2026-11-13, cd53275):
    // interpreter — the HVM slot still holds Nil; `t9 == 5u64` faults
    // Arithmetic(90) "cannot compare different types U64(5) and Nil";
    // rv64 — the prologue zero-filled the slot (needs_zero machinery),
    // `0 == 5` was false, and the body returned 0 in all modes (a
    // value-level divergence, not just a fault class).
    // FIX (P2-F2): flow-sensitive may-Nil analysis (rv64.rs nil_plan) +
    // per-slot init flags + per-value tag bytes + consumer guards with
    // the interpreter's exact fault classes. Post-fix pin: the uninit
    // read faults 90 in every rv64 mode, and the taken-arm contrast
    // still agrees bit-for-bit.
    let pre_q = word_dec("q", 0);
    let body20 = format!(
        "{pre_q}var r = 0u64\nif (q == 0u64) {{\nvar t9 = 5u64\n}}\nr = ((t9 == 5u64) as u64)\nreturn r as bytes"
    );
    let it20 = Interp::try_build(&body20).unwrap();
    let rv20 = Rv64::try_build(&body20).unwrap();
    // arm skipped (q != 0): interpreter Nil-fault — rv64 must fault 90 too
    let ir20 = it20.run(&[1u8; 8]);
    let rr20 = rv20.run(&[1u8; 8]);
    let f20 = ir20.as_ref().expect_err("interpreter must Nil-fault on the uninit read");
    assert_eq!(f20.code, 90);
    assert!(f20.msg.contains("Nil"), "uninit-use message: {}", f20.msg);
    for (mode, r) in &rr20 {
        assert_eq!(
            r.as_ref().err(),
            Some(&90u64),
            "BUG-012 regression: rv64 {mode:?} must Nil-fault the uninit compare with 90"
        );
    }
    println!("BUG-012 fixed pin (uninit int slot): interp Err(90 …Nil) = rv64 all-modes Err(90)");
    // ---- BUG-012 bytes flavor (FIXED; was the D11 row extension) ----
    // Minimal repro: a bytes `var` declared inside an UNTAKEN if-arm, then
    // read by a bytes-family op. Pre-fix: interpreter BytesHandle(91)
    // (canbe.rs:125-133 extract_bytes_with_error_code(Nil), reached from
    // operand.rs cutbyte/cutleft/…/concat) vs rv64 zero-filled empty bytes
    // faulting CastFail(71). Post-fix: rv64 faults 91 at the same site.
    let body_b = format!(
        "{pre_q}if (q == 12345u64) {{\nvar s1 = buf_left(2, data)\n}}\nvar r = (byte(s1, 0) as u64)\nreturn (r) as bytes"
    );
    let it_b = Interp::try_build(&body_b).unwrap();
    let rv_b = Rv64::try_build(&body_b).unwrap();
    // arm skipped (q != 12345): interpreter 91, rv64 91
    let ir_b = it_b.run(&[1u8; 8]);
    let f_b = ir_b.as_ref().expect_err("interpreter must fault byte(Nil-slot) with 91");
    assert_eq!(f_b.code, 91, "uninit-bytes fault class: {f_b:?}");
    assert!(f_b.msg.contains("BytesHandle"), "{f_b:?}");
    for (mode, r) in &rv_b.run(&[1u8; 8]) {
        assert_eq!(
            r.as_ref().err(),
            Some(&91u64),
            "BUG-012 regression: rv64 {mode:?} must fault the uninit bytes use with 91"
        );
    }
    // contrast: arm TAKEN (q == 12345 → input bytes 0..8 = 0x0000000000003039)
    // — both engines agree on byte(s1,0) = data[0] = 0
    let taken_input: Vec<u8> = vec![0, 0, 0, 0, 0, 0, 0x30, 0x39];
    let ir_b2 = it_b.run(&taken_input);
    assert_eq!(ir_b2.as_ref().expect("taken-arm run"), &[0u8; 8]);
    for (mode, r) in &rv_b.run(&taken_input) {
        assert_eq!(r.as_ref().expect("taken-arm run"), &[0u8; 8], "{mode:?}");
    }
    println!("BUG-012 fixed pin (uninit bytes slot): interp Err(91) = rv64 all-modes Err(91)");
    // definite-assignment refinement: a fully-assigned flow (unconditional
    // decl + both diamond arms storing non-Nil) needs NO Nil machinery —
    // the read after the join agrees everywhere and never faults. (A
    // both-arms *declaration* diamond is not writable in fitsh: the
    // frontend binds each symbol once.)
    let body_da = format!(
        "{pre_q}var r = 0u64\nvar t8 = 0u64\nif (q == 0u64) {{\nt8 = 5u64\n}} else {{\nt8 = 6u64\n}}\nr = ((t8 >= 5u64) as u64)\nreturn r as bytes"
    );
    let it_da = Interp::try_build(&body_da).unwrap();
    let rv_da = Rv64::try_build(&body_da).unwrap();
    for inp in [[1u8; 8], [0u8; 8]] {
        let want: Vec<u8> = it_da.run(&inp).expect("diamond interp run").to_vec();
        assert_eq!(want, 1u64.to_be_bytes());
        for (mode, r) in rv_da.run(&inp) {
            assert_eq!(r.as_ref().expect("diamond rv64 run"), &want, "{mode:?}");
        }
    }
    // Nil propagation through assignment: `u8v = t9` then use — the
    // interpreter faults at the USE of the contaminated slot (90)
    let body_prop = format!(
        "{pre_q}var r = 0u64\nvar u8v = 3u64\nif (q == 0u64) {{\nvar t9 = 5u64\n}}\nu8v = t9\nr = ((u8v == 5u64) as u64)\nreturn r as bytes"
    );
    let it_p = Interp::try_build(&body_prop).unwrap();
    let rv_p = Rv64::try_build(&body_prop).unwrap();
    let ir_p = it_p.run(&[1u8; 8]);
    let fp = ir_p.as_ref().expect_err("contaminated use must fault");
    assert_eq!(fp.code, 90, "{fp:?}");
    for (mode, r) in rv_p.run(&[1u8; 8]) {
        assert_eq!(
            r.as_ref().err(),
            Some(&90u64),
            "BUG-012 propagation: {mode:?} must fault at the contaminated use"
        );
    }
    let taken: Vec<u8> = it_p.run(&[0u8; 8]).expect("propagation taken arm").to_vec();
    assert_eq!(taken, 1u64.to_be_bytes());
    for (mode, r) in rv_p.run(&[0u8; 8]) {
        assert_eq!(r.as_ref().expect("propagation taken arm"), &taken, "{mode:?}");
    }
    println!("BUG-012 definite-assignment diamond + Nil-through-assignment propagation: all engines agree");
    // contrast: arm TAKEN (q == 0) — both engines agree on 1
    let ir20b = it20.run(&[0u8; 8]);
    let rr20b = rv20.run(&[0u8; 8]);
    assert_eq!(ir20b.as_ref().expect("taken-arm run"), &[0, 0, 0, 0, 0, 0, 0, 1]);
    for (mode, r) in &rr20b {
        assert_eq!(r.as_ref().expect("taken-arm run"), &[0, 0, 0, 0, 0, 0, 0, 1], "{mode:?}");
    }
}

/// BUG-013 regression pin (FIXED — flipped 2026-11-13, P2-F2).
/// Minimal repro: `zr /= zr` (self-referencing XOP) — pre-fix (2026-11-13,
/// HEAD 90244dc): the ImcRemap lowering leaked ONE vstack entry per
/// self-ref XOP (rv64.rs XOP promoted arm, the `VLoc::Ref(v) if v == rw`
/// materialization called push_vreg() and never popped), so EVERY ImcRemap
/// body containing the shape was rejected with "value stack not empty at
/// function end" — while Im and Imc lowered AND ran the same body. Found
/// by the first bombardment (1 hit in 1199 cases; minimalized to the
/// 3-line body below).
/// FIX: the materialization now pops the entry it pushed (the value
/// survives in the temp register, consumed by the emit_arith below).
/// Post-fix pin: all three modes lower the body; the compiled container
/// agrees with the interpreter on both inputs (0/0 → trap 90; nonzero →
/// quotient 1).
#[test]
fn c2_probe_bug013_xop_self_leak() {
    let body = "var c0 = (byte(data, 0) as u64)\nvar zr = (c0 - c0)\nzr /= zr\nreturn (zr) as bytes";
    // lowering: all three modes must succeed (ImcRemap previously rejected)
    let (ir, _) = frontend(body).expect("frontend");
    for &mode in MODES.iter() {
        let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX);
        let lo = lo.unwrap_or_else(|e| panic!("BUG-013 regression: {mode:?} must lower self-ref XOP: {e}"));
        assert!(lo.code.len() > 0, "{mode:?} must emit code");
    }
    // behavior: 0/0 → div-by-zero trap 90 on every engine; nonzero c0 → zr=1
    let it = Interp::try_build(body).expect("interp build");
    let rv = Rv64::try_build(body).expect("rv64 build (all modes)");
    let f = it.run(&[9u8; 16]).expect_err("interpreter: zr/zr = 0/0 faults 90");
    assert_eq!(f.code, 90, "interp fault class: {f:?}");
    for (mode, r) in rv.run(&[9u8; 16]) {
        assert_eq!(r.as_ref().err(), Some(&90u64), "BUG-013: {mode:?} must fault 90 on 0/0");
    }
    // contrast: non-self XOP and guarded rhs build+run fine in ALL modes
    for (name, ok_body) in [
        ("guarded-rhs", "var c0 = 0u64\nvar zr = (c0 - c0)\nzr /= (zr | 1u64)\nreturn (zr) as bytes"),
        ("two-var", "var a = (byte(data, 0) as u64)\nvar b = a\na /= b\nreturn (a) as bytes"),
    ] {
        match Rv64::try_build(ok_body) {
            Ok(_) => println!("BUG-013 contrast {name}: all modes build+pack"),
            Err(e) => panic!("BUG-013 contrast {name} must build in all modes: {e}"),
        }
    }
    println!("BUG-013 fixed pin: self-ref XOP lowers in all three modes and faults 90 on 0/0 like the interpreter");
}

/// BUG-014 regression pin (FIXED — flipped 2026-11-13, P2-F2).
/// Minimal repro: `if ((c0 == 8u64)) && ((byte(data, 25) > byte(data, 0))) { abort }`
/// with a 24-byte input whose first 8 bytes decode to u64::MAX. Pre-fix
/// (2026-11-13, HEAD 90244dc): interpreter StackError(42) "read buf byte
/// overflow" (the bytecode channel evaluates AND operands eagerly —
/// compile.rs:64-68, node.rs IRNodeDouble::codegen_into — so byte(data,25)
/// faults), while ALL THREE rv64 modes returned Err(90): the fault-free-
/// conjunction branch chain (branch_cond AND arm, gated by is_pure_cond)
/// saw is_pure_cond(top)==true (GT was marked pure WITHOUT recursing into
/// its faultable byte() children), short-circuited past the byte OOB, and
/// then trapped at the subsequent MUL overflow — a consensus-visible trap
/// skipped by an optimization (trap-timing deviation, red-line class).
/// FIX: is_pure_cond recurses into comparison/NOT/AND/OR operands
/// (rv64.rs is_pure_cond); a conjunction whose operand can fault now
/// lowers EAGER (both operands evaluated), matching the bytecode channel.
/// Post-fix pin: all rv64 modes fault at the byte site with 71 (the C1 D2
/// row: interpreter 42, rv64 71 — same-site class mapping).
#[test]
fn c2_probe_bug014_and_chain_skipped_trap() {
    let pre = word_dec("c0", 0);
    let body = format!(
        "{pre}if ((c0 == 8u64)) && (((byte(data, 25) as u64)) > ((byte(data, 0) as u64))) {{ abort }}\nvar r = ((c0 * 2000000000000000000u64) as u64)\nreturn (r) as bytes"
    );
    let input = vec![0xFFu8; 24]; // c0 = u64::MAX; len 24 < index 25 → OOB
    let it = Interp::try_build(&body).expect("interp build");
    let ir = it.run(&input);
    let f = ir.as_ref().expect_err("interpreter must fault byte(data,25) eagerly");
    assert_eq!(f.code, 42, "interpreter fault class: {f:?}");
    assert!(f.msg.contains("read buf byte overflow"), "{f:?}");
    let rv = Rv64::try_build(&body).expect("rv64 build (all modes)");
    for (mode, r) in rv.run(&input) {
        assert_eq!(
            r.as_ref().err(),
            Some(&71u64),
            "BUG-014 regression: {mode:?} must fault at the eagerly-evaluated byte site (71)"
        );
    }
    // control: a genuinely fault-free conjunction STILL takes the
    // short-circuit branch chain (a legal optimization — no trap to move)
    // and agrees with the interpreter bit-for-bit.
    let body_ctl = format!(
        "{pre}if ((c0 == 8u64)) && ((1u64 == 1u64)) {{ abort }}\nreturn ((c0 % 7u64) as u64) as bytes"
    );
    let it_c = Interp::try_build(&body_ctl).expect("ctl interp build");
    let rv_c = Rv64::try_build(&body_ctl).expect("ctl rv64 build");
    let ir_c = it_c.run(&input).expect("ctl interp run");
    for (mode, r) in rv_c.run(&input) {
        assert_eq!(r.as_ref().expect("ctl rv64 run"), &ir_c, "{mode:?} pure-AND control");
    }
    println!("BUG-014 fixed pin: eager AND faults at the byte site — interp Err(42) vs rv64 all-modes Err(71) (D2 row)");
}

/// Edge probes that pin subset-boundary decisions (B-family extensions) and
/// watch shapes whose final disposition belongs to the backend owner.
#[test]
fn c2_probe_engine_edges() {
    let input = vec![7u8, 9, 1, 2, 3, 4, 5, 6];
    let pre = dec2();

    // P2: assignment to the `data` param slot. PINNED subset boundary:
    // the interpreter accepts and runs param-slot assignment; rv64 rejects
    // at LOWER time with a clean message ("the input parameter cannot be
    // assigned") — a B-family compile-reject row, not a crash.
    let body_p2 = "data = buf_right(1, data)\nreturn (byte(data, 0) as u64) as bytes";
    let bi2 = Interp::try_build(body_p2);
    let br2 = Rv64::try_build(body_p2);
    assert!(bi2.is_ok(), "P2: interpreter must accept data-slot assignment");
    let e2 = br2.err().expect("P2: rv64 must reject data-slot assignment (pinned)");
    assert!(e2.contains("input parameter cannot be assigned"), "P2 message: {e2}");
    println!("P2 data-slot reassignment: interp accepts+runs; rv64 rejects ({}) — pinned subset boundary", first_line(&e2));

    // P3: bytes == bytes comparison. PINNED subset boundary: interpreter
    // accepts (content equality in HVM), rv64 rejects cleanly.
    let body_p3 = "var p = buf_left(2, data)\nvar q = buf_left(2, data)\nvar r = ((p == q) as u64)\nreturn r as bytes";
    let bi3 = Interp::try_build(body_p3);
    let br3 = Rv64::try_build(body_p3);
    assert!(bi3.is_ok(), "P3: interpreter must accept bytes==bytes");
    let e3 = br3.err().expect("P3: rv64 must reject bytes==bytes (pinned)");
    assert!(e3.contains("expected an int value"), "P3 message: {e3}");
    println!("P3 bytes==bytes: interp accepts; rv64 rejects ({}) — pinned subset boundary", first_line(&e3));

    // P4: mixed-type ternary arms (u64 vs bytes). WATCH (BUG-011 family):
    // in debug builds the lowerer's `value-if arms must share registers`
    // debug_assert (rv64.rs:4086) panics; in release builds the assert is
    // compiled out and the body lowers. Any of the three shapes below is
    // "today" — the probe records which one, and only a shape OUTSIDE the
    // recorded set fails the gate.
    let body_p4 = format!("{pre}var r = (c0 != 0u64) ? 1u64 : data\nreturn r");
    assert!(Interp::try_build(&body_p4).is_ok(), "P4: interpreter must accept mixed arms");
    let built4 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Rv64::try_build(&body_p4)
    }));
    match built4 {
        Err(_) => println!("P4 mixed-type CHOOSE arms: rv64 lower PANICS (debug_assert) — recorded"),
        Ok(Err(e)) => println!("P4 mixed-type CHOOSE arms: rv64 rejects cleanly ({}) — recorded", first_line(&e)),
        Ok(Ok(rv)) => {
            let it = Interp::try_build(&body_p4).unwrap();
            let obs = fmt_outcome(&(it.run(&input), rv.run(&input)));
            println!("P4 mixed-type CHOOSE arms (release lower path): {obs} — recorded");
        }
    }

    // P5: require evaluation order with a trapping condition (agreed row).
    let body_p5 = format!("{pre}var zr = (c1 - c1)\nrequire 5u64, ((7u64 / zr) == 1u64)\nreturn 1u64 as bytes");
    if let (Ok(it), Ok(rv)) = (Interp::try_build(&body_p5), Rv64::try_build(&body_p5)) {
        let obs = fmt_outcome(&(it.run(&vec![0u8; 16]), rv.run(&vec![0u8; 16])));
        println!("P5 require ev-ok cond-trap: {obs}");
        assert!(
            obs.contains("interp Err(90:") && obs.contains("Err(90)"),
            "require order must agree: {obs}"
        );
    }

    // P15: clamp lo>hi — informational (the classified pin lives in
    // c2_probe_candidate_bug_pins / D10).
    let body_p15 = format!("{pre}var r = clamp(c0, 100u64, 10u64)\nreturn r as bytes");
    if let (Ok(it), Ok(rv)) = (Interp::try_build(&body_p15), Rv64::try_build(&body_p15)) {
        let obs = fmt_outcome(&(it.run(&vec![7u8; 16]), rv.run(&vec![7u8; 16])));
        println!("P15 clamp lo>hi: {obs}");
    }

    // P16: bytes value as a truth condition. Subset boundary: rv64 rejects
    // ("expected an int value, got bytes"); record the interpreter's
    // content-truthiness behavior for the record.
    let body_p16 = "var b = buf_left(3, data)\nif b { return 1u64 as bytes }\nreturn 2u64 as bytes";
    match (Interp::try_build(body_p16), Rv64::try_build(body_p16)) {
        (Ok(it), Ok(rv)) => {
            let zeros = vec![0u8, 0, 0, 9];
            let obs = fmt_outcome(&(it.run(&zeros), rv.run(&zeros)));
            println!("P16 bytes-as-cond (all-zero 3B slice): {obs}");
        }
        (bi, br) => {
            println!(
                "P16 bytes-as-cond: interp built={} rv64 reject={:?}",
                bi.is_ok(),
                br.err().map(|e| first_line(&e).to_string())
            );
            if let Ok(it) = bi {
                let zeros = vec![0u8, 0, 0, 9];
                println!("P16 bytes-as-cond interpreter-only run: {:?}", it.run(&zeros));
            }
        }
    }

    // P6: literal narrowing overflow at parse time (`1000u64 as u8`).
    let body_p6 = "var r = 1000u64 as u8\nreturn (r as u64) as bytes";
    println!(
        "P6 literal 1000u64 as u8: interp {:?} rv64 {:?}",
        Interp::try_build(body_p6).map(|_| ()).err(),
        Rv64::try_build(body_p6).map(|_| ()).err().map(|e| first_line(&e).to_string())
    );

    // P18: `data ++ data` (cat of the param descriptor).
    let body_p18 = "var d = data ++ data\nreturn (byte(d, 8) as u64) as bytes";
    match (Interp::try_build(body_p18), Rv64::try_build(body_p18)) {
        (Ok(it), Ok(rv)) => {
            let obs = fmt_outcome(&(it.run(&input), rv.run(&input)));
            println!("P18 data++data: {obs}");
        }
        (bi, br) => {
            println!(
                "P18 data++data: build rejected — interp built={} rv64 reject={:?}",
                bi.is_ok(),
                br.err().map(|e| first_line(&e).to_string())
            );
        }
    }

    // P19: bare value-expression statement (leftover operand-stack value).
    // FLIPPED (2026-11-13, P2-F2 D9-fix): pre-fix, the statement-position
    // CHOOSE leaked its vstack entries (the arm arm lowering pushed both
    // arms' compile-time values), so any later `while` was rejected with
    // "while loop in expression position" and a fall-off-the-end body with
    // "value stack not empty at function end" — pinned subset boundaries.
    // The eager-CHOOSE lowering (eval + drop through the default statement
    // arm) pushes exactly ONE result and pops it: no leak, so both bodies
    // now lower AND agree with the interpreter (which tolerates/discards
    // the leftover operand-stack value).
    let body_p19 = "var q = 0u64\nvar w = 0u64\nchoose((1u64 == 1u64), 5u64, 6u64)\nwhile w < 1 { w += 1 }\nreturn q as bytes";
    let it19 = Interp::try_build(body_p19).expect("P19: interpreter must accept the leftover-value body");
    let rv19 = Rv64::try_build(body_p19).expect("P19a post-D9-fix: rv64 must lower bare choose + while");
    let want19 = it19.run(&[]).expect("P19 interp run");
    for (mode, r) in rv19.run(&[]) {
        assert_eq!(r.as_ref().expect("P19a rv64 run"), &want19, "P19a {mode:?}");
    }
    println!("P19a bare choose stmt + later while: both engines lower and agree — pin flipped");
    // P19b: fall-off-the-end variant — the interpreter's end path DISCARDS
    // the leftover (returns empty bytes, same as the Nil boundary); rv64's
    // fallthrough exit also emits an empty output.
    let body_p19b = "choose((1u64 == 1u64), 5u64, 6u64)";
    let it19b = Interp::try_build(body_p19b).expect("P19b: interpreter must accept");
    let got19b = it19b.run(&[]).expect("P19b: interpreter returns empty on end");
    assert_eq!(got19b, Vec::<u8>::new(), "P19b: leftover must be discarded on end");
    let rv19b = Rv64::try_build(body_p19b).expect("P19b post-D9-fix: rv64 must lower fall-off-end");
    for (mode, r) in rv19b.run(&[]) {
        assert_eq!(r.as_ref().expect("P19b rv64 run"), &Vec::<u8>::new(), "P19b {mode:?}");
    }
    println!("P19b bare choose stmt fall-off-end: both engines return empty — pin flipped");
}

/* ================================================================== */
/* Section 7: negative suites (frontend-reject + rv64-compile-reject). */
/* ================================================================== */

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

#[test]
fn c2_negative_frontend_rejects() {
    // Both channels must reject at the shared front end.
    let cases = frontend_negative_cases();
    for (name, body) in cases {
        let err = Interp::try_build(&body)
            .err()
            .unwrap_or_else(|| panic!("negative '{name}' must be frontend-rejected"));
        let err_rv = Rv64::try_build(&body)
            .err()
            .unwrap_or_else(|| panic!("negative '{name}' must also fail rv64-side"));
        println!("NEG-FR {name}: interp/{}", first_line(&err_rv));
        let _ = err;
    }
}

/// rv64-only reject: interpreter builds (and runs), rv64 rejects all 3
/// modes with a message naming the offending opcode.
#[test]
fn c2_negative_rv64_compile_rejects() {
    let cases = rv64_negative_cases();
    // NOTE (pinned 2026-11-13): `cast_to(a, u8)` and `sqrt(a)` are NOT
    // source-callable — the shared front end rejects them outright, so the
    // CTO/SQRT opcode-level rv64 rejections are only reachable via
    // `bytecode{}` splices. Observed as NEG-FR, not NEG-RX.
    for (name, body, fragments, interp_runs) in cases {
        let it = Interp::try_build(&body)
            .unwrap_or_else(|e| panic!("negative '{name}': front end must accept, got: {e}"));
        if interp_runs {
            // Ok OR a runtime fault both prove acceptance (the harness only
            // panics on BUILD failure)
            let r = it.run(&[]);
            println!("NEG-RX {name}: interp accepted, run → {:?}", r.map(|b| b.len()));
        }
        let err = Rv64::try_build(&body)
            .err()
            .unwrap_or_else(|| panic!("negative '{name}': rv64 must reject"));
        assert!(
            fragments.iter().any(|f| err.contains(f)),
            "negative '{name}': rv64 error {:?} must mention {fragments:?}",
            first_line(&err)
        );
        println!("NEG-RX {name}: rv64 rejected ({})", first_line(&err));
    }
}

#[test]
fn c2_debug_bisect_body() {
    // TEMPORARY: prefix-bisect a body from stdin file ( bombard repro)
    let path = std::env::var("GEN_BISECT_FILE").unwrap_or_default();
    if path.is_empty() {
        return;
    }
    // minimal-shape probes for the ternary-in-comparison leak
    for (name, body) in [
        ("xop-div-self", "var c0 = (byte(data, 0) as u64)\nvar zr = (c0 - c0)\nzr /= zr\nreturn (zr) as bytes"),
        ("xop-div-guarded-self", "var c0 = (byte(data, 0) as u64)\nvar zr = (c0 - c0)\nzr /= (zr | 1u64)\nreturn (zr) as bytes"),
        ("xop-add-self", "var a = (byte(data, 0) as u64)\na += a\nreturn (a) as bytes"),
        ("xop-sub-self", "var a = (byte(data, 0) as u64)\na -= a\nreturn (a) as bytes"),
        ("xop-mul-self", "var a = (byte(data, 0) as u64)\na *= a\nreturn (a) as bytes"),
        ("xop-div-self-plain", "var a = (byte(data, 0) as u64)\na /= a\nreturn (a) as bytes"),
        ("xop-div-two-var", "var a = (byte(data, 0) as u64)\nvar b = a\na /= b\nreturn (a) as bytes"),
    ] {
        let rv = Rv64::try_build(body);
        let interp = Interp::try_build(body);
        let run = match (&interp, &rv) {
            (Ok(it), Ok(r)) => {
                let rs = r
                    .run(&[7u8; 8])
                    .iter()
                    .map(|(m, x)| match x {
                        Ok(b) => format!("{m:?}=Ok({}B)", b.len()),
                        Err(c) => format!("{m:?}=Err({c})"),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(" run interp={:?} rv {rs}", it.run(&[7u8; 8]).map(|b| b.len()))
            }
            _ => String::new(),
        };
        println!(
            "{name}: interp build={} rv64 build={}{}",
            interp.is_ok(),
            rv.as_ref().map(|_| "ok").err().map(|e| first_line(e)).unwrap_or("ok"),
            run
        );
    }
    let body = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    for k in 1..=lines.len() {
        let mut prefix = lines[..k].to_vec();
        prefix.push("return (acc) as bytes");
        let b = prefix.join("\n");
        let err = Rv64::try_build(&b).err().map(|e| first_line(&e).to_string());
        println!("prefix {k:2}: {}", err.unwrap_or_else(|| "ok".into()));
    }
    let err = Rv64::try_build(&body).err().map(|e| first_line(&e).to_string());
    println!("full   : {}", err.unwrap_or_else(|| "ok".into()));
}

/* ================================================================== */
/* Section 8: default tiers — smoke, campaign, determinism.            */
/* ================================================================== */




#[test]
fn c2_generator_smoke() {
    // 24 cases × full input set: runnable, knockable, zero Unexpected.
    let seed0 = 0x5100_0000_0000_0001;
    let (stats, cov) = campaign_with_coverage(24, seed0, 8, false, 0);
    println!(
        "SMOKE: cases {} knocks {} budget {} frontend_rejects {} rv64_rejects {} bug013_rejects {} downsized {} depth_downsized {}",
        stats.cases, stats.knocks, stats.budget, stats.frontend_rejects,
        stats.rv64_rejects_unexpected, stats.bug013_rejects, stats.downsized, stats.depth_downsized
    );
    for (k, v) in stats.verdicts.iter() {
        println!("    {k}: {v}");
    }
    print_coverage(&cov, "smoke");
    assert_eq!(stats.frontend_rejects, 0, "generator produced a frontend-rejected program");
    assert_eq!(stats.rv64_rejects_unexpected, 0, "generator produced an rv64-rejected program");
    let unexpected = stats.verdicts.iter().filter(|(k, _)| k.starts_with("UNEXPECTED")).count();
    assert_eq!(unexpected, 0, "smoke corpus must be clean");
    // P2-C3: the composition assertion is family-wide — the reflow from
    // the constraint-⑥ unfreeze (E_bytes_if_arm rewiring) moved the OK-path
    // cases out of this 24-seed window, but the agree FAMILY (ok/trap90/
    // trap71/91/user_value) is what "smoke must exercise the agree path"
    // means: the four engines agreeing on some knock outcome.
    let agree_rows = stats
        .verdicts
        .keys()
        .filter(|k| k.starts_with("agree"))
        .map(|k| stats.verdicts[k])
        .sum::<u64>();
    assert!(agree_rows > 0, "smoke must exercise the agree path (any agree family row)");
    stats.assert_fixed_rows_zero("smoke");
}

#[test]
fn c2_default_campaign_150() {
    // The stable regression tier: 150 cases, all families, full input set.
    let seed0 = 0xC2DE_0000_0000_0001;
    let t0 = std::time::Instant::now();
    let (stats, cov) = campaign_with_coverage(150, seed0, 10, false, 0);
    println!(
        "CAMPAIGN-150: {} cases / {} knocks in {:?} — budget {} frontend {} rv64 {} bug013 {} downsized {} depth_downsized {}",
        stats.cases, stats.knocks, t0.elapsed(), stats.budget,
        stats.frontend_rejects, stats.rv64_rejects_unexpected, stats.bug013_rejects,
        stats.downsized, stats.depth_downsized
    );
    for (k, v) in stats.verdicts.iter() {
        println!("    {k}: {v}");
    }
    print_coverage(&cov, "campaign-150");
    assert_eq!(stats.frontend_rejects, 0);
    assert_eq!(stats.rv64_rejects_unexpected, 0);
    let unexpected = stats.verdicts.iter().filter(|(k, _)| k.starts_with("UNEXPECTED")).count();
    assert_eq!(unexpected, 0, "campaign must be clean");
    stats.assert_fixed_rows_zero("campaign-150");
}

#[test]
fn c2_determinism_quick() {
    // same seed → same program text; same program → byte-identical
    // bytecode + containers across two independent compiles.
    let seed0 = 0xDE7E_0000_0000_0001;
    for i in 0..10 {
        let seed = seed0 + i;
        let a = generate_case(seed, 8, GenBudget::default());
        let b = generate_case(seed, 8, GenBudget::default());
        assert_eq!(a.body, b.body, "seed {seed} regenerated different text");
        assert_eq!(a.inputs, b.inputs, "seed {seed} regenerated different inputs");
        assert_eq!(a.feat, b.feat, "seed {seed} regenerated different features");
    }
    for i in 0..10 {
        let seed = seed0 + 100 + i;
        let case = generate_case(seed, 8, GenBudget::default());
        let b1 = Rv64::try_build(&case.body);
        let b2 = Rv64::try_build(&case.body);
        assert_eq!(
            b1.is_ok(),
            b2.is_ok(),
            "seed {seed}: build success must be deterministic"
        );
        let (r1, r2) = match (b1, b2) {
            (Ok(x), Ok(y)) => (x, y),
            _ => {
                println!("DET seed {seed}: case not buildable (generator edge), skipped for byte-compare");
                continue;
            }
        };
        for ((m1, c1), (m2, c2)) in r1.containers.iter().zip(r2.containers.iter()) {
            assert_eq!(m1, m2);
            assert_eq!(c1, c2, "seed {seed} mode {m1:?} containers differ across compiles");
        }
        let tokens = Tokenizer::new(case.body.as_bytes()).parse().expect("tok");
        let (_, c1, _) = compile_body(
            tokens,
            vec![("data".to_string(), ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .expect("cb 1");
        let CompiledCode::Bytecode(b1) = c1 else { panic!("bytecode 1") };
        let tokens = Tokenizer::new(case.body.as_bytes()).parse().expect("tok");
        let (_, c2, _) = compile_body(
            tokens,
            vec![("data".to_string(), ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .expect("cb 2");
        let CompiledCode::Bytecode(b2) = c2 else { panic!("bytecode 2") };
        assert_eq!(b1, b2, "seed {seed} bytecode differs across compiles");
    }
}

/* ================================================================== */
/* Section 9: ignored tiers — mass bombardment, mutation, determinism. */
/* Run (release):                                                      */
/*   cargo test -p vm --release --test rv64_gen_corpus c2_bombard_mass -- --ignored --nocapture */
/* Knobs: GEN_CORPUS_N (1500), GEN_CORPUS_SEED0, GEN_CORPUS_STMTS (10),*/
/*        GEN_CORPUS_SAVE (dir to persist unexpected cases).           */
/* ================================================================== */


#[test]
#[ignore]
fn c2_bombard_mass() {
    let n = env_u64("GEN_CORPUS_N", 1500);
    let seed0 = env_u64("GEN_CORPUS_SEED0", 0xB0DB_0000_0000_0001);
    let stmts = env_u64("GEN_CORPUS_STMTS", 10);
    let save = std::env::var("GEN_CORPUS_SAVE").ok();
    let t0 = std::time::Instant::now();
    let (stats, cov) = campaign_with_coverage(n, seed0, stmts, true, 8);
    let dt = t0.elapsed();
    println!("\n==================== BOMBARD SUMMARY ====================");
    println!(
        "cases {} knocks {} ({:.2} inputs/case) in {:?} ({:.1} knocks/s)",
        stats.cases,
        stats.knocks,
        stats.knocks as f64 / stats.cases.max(1) as f64,
        dt,
        stats.knocks as f64 / dt.as_secs_f64().max(1e-9)
    );
    let mut agree = 0u64;
    let mut pinned = 0u64;
    for (k, v) in stats.verdicts.iter() {
        println!("    {k}: {v}");
        if k.starts_with("agree") {
            agree += v;
        } else if k.starts_with("pin_") {
            pinned += v;
        }
    }
    println!("agree {agree} / pinned-divergence {pinned} / budget {}", stats.budget);
    println!(
        "frontend rejects {} rv64 rejects {} (bug013 pinned rejects {} kernel-subset pinned rejects {}) mirror-downsized {} depth-downsized {}",
        stats.frontend_rejects, stats.rv64_rejects_unexpected, stats.bug013_rejects,
        stats.kernel_subset_rejects, stats.downsized, stats.depth_downsized
    );
    print_coverage(&cov, "bombard");
    if let Some(dir) = save {
        let _ = std::fs::create_dir_all(&dir);
        for (i, (seed, reason, input, n, budget)) in stats.unexpected.iter().take(40).enumerate() {
            let case = generate_case(*seed, *n, *budget);
            let _ = std::fs::write(
                format!("{dir}/unexpected_{i:02}_seed{seed}.fitsh"),
                format!("// {reason}\n// input: {input:?}\n{}", case.body),
            );
        }
    }
    let unexpected = stats.verdicts.iter().filter(|(k, _)| k.starts_with("UNEXPECTED")).count();
    assert_eq!(unexpected, 0, "bombard found unexpected divergences (see minimizations)");
    stats.assert_fixed_rows_zero("bombard");
}

#[test]
#[ignore]
fn c2_mutation_pass() {
    // Statement-level mutations of generated programs: delete a random
    // statement, duplicate one, or perturb a numeric literal. Every mutant
    // must classify clean (agree or pinned); Unexpected = finding.
    let n = env_u64("GEN_MUT_N", 200);
    let seed0 = env_u64("GEN_MUT_SEED0", 0x6E17_0000_0000_0001);
    let mut rng = Rng::new(seed0);
    let mut stats = CampaignStats::default();
    let mut mutants = 0u64;
    while mutants < n {
        let seed = seed0.wrapping_add(rng.below(4000));
        let case = generate_case(seed, 10, GenBudget::default());
        let mut lines: Vec<String> = case.body.lines().map(|s| s.to_string()).collect();
        if lines.len() < 4 {
            continue;
        }
        let kind = rng.below(3);
        match kind {
            0 => {
                // delete a random non-decoder line
                let i = rng.range(2, lines.len() as u64 - 1) as usize;
                lines.remove(i);
            }
            1 => {
                // duplicate a random line
                let i = rng.below(lines.len() as u64) as usize;
                let l = lines[i].clone();
                lines.insert(i, l);
            }
            _ => {
                // perturb a number literal in a random line
                let i = rng.below(lines.len() as u64) as usize;
                let l = &mut lines[i];
                if let Some(pos) = l.find(|c: char| c.is_ascii_digit()) {
                    let d = (rng.below(10) as u8 + b'0') as char;
                    l.replace_range(pos..pos + 1, &d.to_string());
                }
            }
        }
        let body = lines.join("\n");
        // P2-C3: mutants can reach lowerer shapes the generator never emits
        // — e.g. the BUG-017 facet-(c) vstack index underflow (bytes-first
        // mixed CHOOSE) that PANICKED in debug and release pre-fix. (First
        // observed 2026-11-15: mutant of seed 0x6E17… era hit rv64.rs
        // CHOOSE-arm index underflow through a perturbed literal.)
        // P2-C3FIX-BUG017: the up-front arm-type check removed the panic
        // face — PANICKED is expected 0 now; the catch_unwind stays as a
        // defensive recorder for any future lowerer edge.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (Rv64::try_build(&body), Interp::try_build(&body))
        }));
        let (rv, it) = match built {
            Ok((Ok(r), Ok(i))) => (r, i),
            Err(_) => {
                stats.frontend_rejects += 1;
                println!("[mut of seed {seed}] compile PANICKED (documented lowerer edge, BUG-017 facet c):\n{body}\n");
                mutants += 1;
                continue;
            }
            Ok((Err(e), _)) | Ok((_, Err(e))) => {
                // mutants may legitimately fail to compile (e.g. broken
                // decode prelude → unused-var is fine, syntax break isn't):
                // count as compile-rejected mutant, not a failure
                stats.frontend_rejects += 1;
                println!("[mut of seed {seed}] compile rejected: {}", first_line(&e));
                mutants += 1;
                continue;
            }
        };
        for (idx, input) in case.inputs.iter().enumerate().take(3) {
            let v = knock_one(&rv, &it, &case.feat, input);
            if let Verdict::Unexpected(reason) = &v {
                stats.unexpected.push((
                    seed,
                    format!("{v:?} (mut-kind {kind} input#{idx})"),
                    input.clone(),
                    case.n_stmts,
                    case.budget,
                ));
                println!("[mut of seed {seed}] UNEXPECTED {reason} (kind {kind}):\n{body}\n");
            }
            stats.note(&v);
            stats.knocks += 1;
        }
        mutants += 1;
    }
    println!(
        "MUTATION: {mutants} mutants / {} knocks / {} compile-rejected",
        stats.knocks, stats.frontend_rejects
    );
    for (k, v) in stats.verdicts.iter() {
        println!("    {k}: {v}");
    }
    let unexpected = stats.verdicts.iter().filter(|(k, _)| k.starts_with("UNEXPECTED")).count();
    assert_eq!(unexpected, 0, "mutation pass found unexpected divergences");
}

/// D9 audit (mechanical): for a large seed range, `feat.choose` must hold
/// EXACTLY when the body contains a CHOOSE-node shape (ternary `? :` or the
/// `choose(` builtin) — the value-if shape (IRIFR, lazy on both engines)
/// must never set it. This bounds the PinD9ChooseLazy attribution: a D9
/// verdict can only fire on a body with a real CHOOSE node. (Residual
/// ambiguity — an interp fault outside the CHOOSE arm of a choose-bearing
/// body — is covered by the audit discipline in 70-c2-generator.md §5.)
#[test]
#[ignore]
fn c2_audit_choose_flags() {
    let n = env_u64("GEN_AUDIT_N", 2000);
    let seed0 = env_u64("GEN_AUDIT_SEED0", 0xB0DB_0000_0000_0001);
    let mut with_choose = 0u64;
    let mut with_mixed = 0u64;
    for i in 0..n {
        let seed = seed0.wrapping_add(i);
        let case = generate_case(seed, 10, GenBudget::default());
        let has_ternary = case.body.contains('?');
        let has_choose_call = case.body.contains("choose(");
        if case.feat.choose {
            with_choose += 1;
            assert!(
                has_ternary || has_choose_call,
                "seed {seed}: feat.choose but no CHOOSE shape:\n{}",
                case.body
            );
        } else if case.feat.mixed_arm {
            // P2-C3 B1: S_choose_mixed_arm bodies contain `choose(` BY
            // CONSTRUCTION (the negative channel — rv64 rejects the body,
            // no rv64 execution, so the classifier's choose-laziness rows
            // never apply and feat.choose stays false). Counted, exempted
            // from the biconditional.
            with_mixed += 1;
            assert!(has_choose_call, "seed {seed}: mixed_arm without choose( text");
        } else {
            assert!(
                !(has_ternary || has_choose_call),
                "seed {seed}: CHOOSE shape present but feat.choose not set:\n{}",
                case.body
            );
        }
    }
    println!(
        "AUDIT-CHOOSE: {n} seeds, {with_choose} with CHOOSE shapes, {with_mixed} mixed-arm (exempt), flags exact"
    );
}

#[test]
#[ignore]
fn c2_determinism_50() {
    // N seeds (P2-C4: GEN_DETERMINISM_N knob, default 50 unchanged):
    // same-seed regeneration byte-equal + same-program recompile
    // byte-equal (bytecode + all three rv64 containers).
    let seed0 = env_u64("GEN_DET_SEED0", 0xDE71_0000_0000_0001);
    let n = env_u64("GEN_DETERMINISM_N", 50);
    for i in 0..n {
        let seed = seed0 + i;
        let a = generate_case(seed, 10, GenBudget::default());
        let b = generate_case(seed, 10, GenBudget::default());
        assert_eq!(a.body, b.body, "seed {seed} regenerated different text");
        assert_eq!(a.inputs, b.inputs, "seed {seed} regenerated different inputs");
        assert_eq!(a.feat, b.feat, "seed {seed} regenerated different features");
        let r1 = Rv64::try_build(&a.body);
        let r2 = Rv64::try_build(&a.body);
        // mirror-overflow bodies (bigbody family over the 16383 B budget) are
        // not buildable without the campaign's downsizing retry — the pinned
        // fact there is that build success itself is deterministic
        assert_eq!(r1.is_ok(), r2.is_ok(), "seed {seed}: build success not deterministic");
        let (r1, r2) = match (r1, r2) {
            (Ok(x), Ok(y)) => (x, y),
            _ => {
                println!("DET seed {seed}: not buildable (mirror budget), byte-compare skipped");
                continue;
            }
        };
        assert_eq!(r1.containers, r2.containers, "seed {seed} containers differ");
    }
    println!("DETERMINISM: all {n} seeds regenerate + recompile byte-identical");
}

/// P2-C4 S4: cross-PROCESS determinism — phase 1 dumps the generated
/// corpus artifacts (body text + per-mode packed container bytes) to a
/// directory; phase 2 runs in a FRESH process, regenerates + recompiles
/// the same seeds and byte-compares against the dump. This excludes
/// in-process memoization false positives by construction (two separate
/// `cargo test` invocations are two separate OS processes).
/// Env: GEN_DET_CROSS_PHASE ("1" dump / "2" verify; unset = no-op),
/// GEN_DET_CROSS_N (default 128 bodies), GEN_DET_CROSS_SEED0
/// (default 0xDE7C_0000_0000_0001), GEN_DET_CROSS_DIR (default
/// /tmp/c4_det_cross). Defaults keep every existing tier unchanged.
#[test]
#[ignore]
fn c2_determinism_cross_process() {
    let phase = std::env::var("GEN_DET_CROSS_PHASE").unwrap_or_default();
    if phase != "1" && phase != "2" {
        println!("GEN_DET_CROSS_PHASE unset: cross-process determinism disabled (default off)");
        return;
    }
    let n = env_u64("GEN_DET_CROSS_N", 128);
    let seed0 = env_u64("GEN_DET_CROSS_SEED0", 0xDE7C_0000_0000_0001);
    let dir = std::env::var("GEN_DET_CROSS_DIR").unwrap_or_else(|_| "/tmp/c4_det_cross".into());
    std::fs::create_dir_all(&dir).expect("cross-process dir");
    // buildable-flag manifest shared by both phases (mirror-overflow bodies
    // are consistently unbuildable without downsizing — compare the flag)
    let mut buildable = Vec::new();
    for i in 0..n {
        let seed = seed0.wrapping_add(i);
        let case = generate_case(seed, 10, GenBudget::default());
        let built = Rv64::try_build(&case.body);
        buildable.push((seed, built.is_ok()));
        match built {
            Ok(rv) => {
                let mut blob = Vec::new();
                // containers follow c3lib MODES order: Im, Imc, ImcRemap
                for (idx, (_m, c)) in rv.containers.iter().enumerate() {
                    blob.extend_from_slice(&(idx as u8).to_le_bytes());
                    blob.extend_from_slice(&(c.len() as u64).to_le_bytes());
                    blob.extend_from_slice(c);
                }
                std::fs::write(format!("{dir}/{seed}.body"), &case.body).expect("write body");
                std::fs::write(format!("{dir}/{seed}.bin"), &blob).expect("write containers");
            }
            Err(_) => {
                let _ = std::fs::remove_file(format!("{dir}/{seed}.body"));
                let _ = std::fs::remove_file(format!("{dir}/{seed}.bin"));
            }
        }
    }
    let manifest: String = buildable
        .iter()
        .map(|(seed, ok)| format!("{seed} {}
", if *ok { 1 } else { 0 }))
        .collect();
    std::fs::write(format!("{dir}/manifest.txt"), &manifest).expect("write manifest");
    if phase == "1" {
        println!(
            "CROSS-PROCESS PHASE 1: dumped {n} seeds to {dir} (buildable {})",
            buildable.iter().filter(|(_, ok)| *ok).count()
        );
        return;
    }
    // phase 2: fresh process — REGENERATE from the seeds (no reuse of the
    // dumped bodies) and byte-compare text + all three container encodings
    let manifest_disk =
        std::fs::read_to_string(format!("{dir}/manifest.txt")).expect("read manifest");
    assert_eq!(manifest, manifest_disk, "cross-process: buildable flags differ");
    for (seed, ok) in buildable.iter() {
        let case = generate_case(*seed, 10, GenBudget::default());
        if *ok {
            let body_disk =
                std::fs::read_to_string(format!("{dir}/{seed}.body")).expect("read body");
            assert_eq!(case.body, body_disk, "seed {seed}: body text differs across processes");
            let bin_disk = std::fs::read(format!("{dir}/{seed}.bin")).expect("read containers");
            let rv = Rv64::try_build(&case.body).expect("phase-2 rebuild must succeed");
            let mut blob = Vec::new();
            for (idx, (_m, c)) in rv.containers.iter().enumerate() {
                blob.extend_from_slice(&(idx as u8).to_le_bytes());
                blob.extend_from_slice(&(c.len() as u64).to_le_bytes());
                blob.extend_from_slice(c);
            }
            assert_eq!(blob, bin_disk, "seed {seed}: container bytes differ across processes");
        }
    }
    println!("CROSS-PROCESS PHASE 2: {n} seeds regenerated + recompiled byte-identical in a fresh process");
}



/// P2-C3 B1 — constraint ⑥ unfreeze probe: ≥200 randomized BYTES×BYTES
/// value-if arms through the full four-way knock. The value-position `if`
/// (IRIFR) bytes-arm path is the BUG-011-fixed take_arm_values/join; this
/// probe is the gate that admitted E_bytes_if_arm into the main grammar
/// (zero Unexpected required). The CHOOSE-eager bytes-arm shape
/// (ternary/choose() with bytes arms) is deliberately NOT probed here — it
/// miscompiles (BUG-017 candidate) and is pinned in rv64_c3_coverage.rs.
#[test]
fn c2_probe_bytes_if_arms() {
    let seed0 = 0xB1A6_0000_0000_0001u64;
    let n = 240u64;
    let mut stats = CampaignStats::default();
    let mut bytes_arm_cases = 0u64;
    for i in 0..n {
        let seed = seed0.wrapping_add(i);
        let mut rng = Rng::new(seed ^ 0xB5E5);
        // determinstic mini-body: decoders, a bytes pool, a value-if over
        // bytes arms, then a random consumer (byte/size/++/return)
        let mut body = String::new();
        body.push_str(&dec2());
        let nvars = 1 + rng.below(3);
        for v in 0..nvars {
            let len = 2 + rng.below(6) as usize;
            body.push_str(&format!("var s{v} = buf_left({len}, data)\n"));
        }
        let arms = 2 + rng.below(4) as usize;
        let mut arms_txt = Vec::new();
        for _ in 0..arms {
            let a = rng.below(4);
            let txt = match a {
                0 => format!("buf_left({}, data)", 2 + rng.below(6)),
                1 => format!("buf_right({}, data)", 2 + rng.below(6)),
                2 => {{
                    let l = 1 + rng.below(4) as usize;
                    let chars: Vec<char> = "XYZQ".chars().collect();
                    format!("\"{}\"", (0..l).map(|_| chars[rng.below(4) as usize]).collect::<String>())
                }}
                _ => {
                    let v = rng.below(nvars);
                    format!("s{v}")
                }
            };
            arms_txt.push(txt);
        }
        let cond = match rng.below(3) {
            0 => "((c0 % 2u64) == 0u64)".to_string(),
            1 => "(c1 > 100u64)".to_string(),
            _ => "((c0 + c1) != 7u64)".to_string(),
        };
        // nested value-if chains: if (c) { if (c2) { a } else { b } } else { d }
        fn build_if(
            rng: &mut Rng,
            cond: &str,
            depth: u64,
            pool: &mut Vec<String>,
        ) -> String {
            if depth > 0 && rng.chance(40) {
                let x = build_if(rng, cond, depth - 1, pool);
                let y = build_if(rng, cond, depth - 1, pool);
                format!("if ({cond}) {{ {x} }} else {{ {y} }}")
            } else {
                let a = pool.pop().unwrap_or_else(|| "buf_left(2, data)".to_string());
                let b = pool.pop().unwrap_or_else(|| "\"z\"".to_string());
                format!("if ({cond}) {{ {a} }} else {{ {b} }}")
            }
        }
        let vi = build_if(&mut rng, &cond, 2, &mut arms_txt);
        let tail = match rng.below(4) {
            0 => format!("var r = ({vi})\nreturn (byte(r, 0) as u64) as bytes"),
            1 => format!("var r = ({vi})\nreturn (size(r) as u64) as bytes"),
            2 => format!("var r = ({vi} ++ \"t\")\nreturn (size(r) as u64) as bytes"),
            _ => format!("return ({vi})"),
        };
        body.push_str(&tail);
        let mut feat = Feat::default();
        feat.bytes_if_arm = true;
        match (Rv64::try_build(&body), Interp::try_build(&body)) {
            (Ok(rv), Ok(it)) => {
                bytes_arm_cases += 1;
                for (idx, input) in inputs_for(seed, "normal").iter().enumerate() {
                    let v = knock_one(&rv, &it, &feat, input);
                    if let Verdict::Unexpected(r) = &v {
                        stats.unexpected.push((
                            seed,
                            format!("{v:?} (input#{idx})"),
                            input.clone(),
                            8,
                            GenBudget::default(),
                        ));
                        println!("[bytes-if seed {seed}] UNEXPECTED {r}:\n{body}\n");
                    }
                    stats.note(&v);
                    stats.knocks += 1;
                }
            }
            (Err(e), _) => panic!("[bytes-if seed {seed}] rv64 rejected a bytes-arm body: {e}\n{body}"),
            (_, Err(e)) => panic!("[bytes-if seed {seed}] frontend rejected: {e}\n{body}"),
        }
    }
    println!(
        "BYTES-IF PROBE: {bytes_arm_cases} cases / {} knocks, zero-unexpected gate",
        stats.knocks
    );
    for (k, v) in stats.verdicts.iter() {
        println!("    {k}: {v}");
    }
    let unexpected = stats.verdicts.iter().filter(|(k, _)| k.starts_with("UNEXPECTED")).count();
    assert_eq!(unexpected, 0, "bytes-arm probe found unexpected divergences");
    assert!(stats.knocks > 1000, "probe must knock a real corpus");
    stats.assert_fixed_rows_zero("bytes-if-probe");
}
