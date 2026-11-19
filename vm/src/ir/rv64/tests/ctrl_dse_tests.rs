//! C11/C15/C16 (impl/ctrl-dse): targeted differential tests.
//!
//! C11 — pure dead-statement elimination (value-position statements whose
//!        whole subtree is fault-free and side-effect-free emit nothing).
//! C15 — branch_cond OR short-circuit chain (mirror of the AND chain, same
//!        is_pure_cond guard).
//! C16 — `x == K1 || x == K2 || ...` membership-test bitmask.
//!
//! Differential pattern (the guard_elim_tests convention): every body runs
//! through the real pipeline in all three lowering modes; expectations are
//! the interpreter's documented faces. Elimination firing is asserted
//! structurally: a body with an admitted dead statement must lower to the
//! SAME code length as the body without it, and a non-admitted one
//! (HASKEY's 72 face) must stay strictly longer.

use super::helpers::{compile_with_input, run_e2e, run_input_mode};
use super::super::*;
use crate::calc::{CalcFnObj, CalcRunLimits};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// Pack + run an input-param body and return the whole exec (so fault
/// faces assert on the guest exit code, the subset_tests pattern).
fn exec_input(body: &str, input: &[u8], mode: Rv64Mode) -> crate::calc::CalcExec {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, mode)
        .unwrap_or_else(|e| panic!("lower failed ({mode:?}): {e}"));
    let cap = SpaceCap::new(0);
    let container = match mode {
        Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
        _ => lo.pack_segments_profile(
            &cap,
            64,
            5120,
            1280,
            crate::calc::container::CALC_PROFILE_RV64IMC,
        ),
    }
    .unwrap_or_else(|e| panic!("pack failed: {e}"));
    let fno = CalcFnObj::build_v1(container).unwrap();
    fno.run(input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None })
}

fn assert_ok(body: &str, input: &[u8], want: u64, tag: &str) {
    for mode in MODES {
        let out = run_input_mode(body, input, mode);
        assert_eq!(out, want.to_be_bytes().to_vec(), "{tag} mode {mode:?}");
    }
}

fn assert_exit(body: &str, input: &[u8], code: u64, tag: &str) {
    for mode in MODES {
        let e = exec_input(body, input, mode);
        assert!(e.result.is_err(), "{tag} mode {mode:?} should trap {code}");
        assert_eq!(e.exit_code, code, "{tag} mode {mode:?}");
    }
}

/// Per-mode code length of a lowered input-param body (Im vs Imc compress
/// differently, so equality assertions are per mode, never across modes).
fn code_len_mode(body: &str, mode: Rv64Mode) -> usize {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, mode)
        .unwrap_or_else(|e| panic!("lower failed ({mode:?}): {e}"));
    lo.code.len()
}

/// The with-body must cost the SAME bytes as the without-body in every mode.
fn assert_same_len(with: &str, without: &str, tag: &str) {
    for mode in MODES {
        let (lw, lb) = (code_len_mode(with, mode), code_len_mode(without, mode));
        assert_eq!(lw, lb, "{tag} mode {mode:?}");
    }
}

/// The with-body must cost STRICTLY MORE bytes than the without-body.
fn assert_more_len(with: &str, without: &str, tag: &str) {
    let mut more = 0;
    for mode in MODES {
        let (lw, lb) = (code_len_mode(with, mode), code_len_mode(without, mode));
        assert!(lw > lb, "{tag} mode {mode:?}: with {lw} !> without {lb}");
        more += 1;
    }
    assert_eq!(more, MODES.len());
}

/* ==================================================================== */
/* C11 — pure dead-statement elimination                                */
/* ==================================================================== */

/// A dead statement must cost ZERO bytes: body-with equals body-without.
#[test]
fn c11_dead_statements_cost_zero_bytes() {
    // dead int literal + dead bool literal + dead SIZE(bytes literal)
    let base = "var x = 1u64\nreturn x as bytes";
    let with = "var x = 1u64\n5\n7\ntrue\nfalse\nsize(\"ab\")\nreturn x as bytes";
    assert_same_len(with, base, "dead literals + dead SIZE(lit)");

    // dead folded cast (literal through CU8/CU32/CU64)
    let with_cast = "var x = 1u64\n3 as u8\n9 as u32\n(2 as u8) as u64\nreturn x as bytes";
    assert_same_len(with_cast, base, "dead folded casts");

    // dead constant-answer type test over a pure operand
    let with_is = "var x = 1u64\nx is u64\nx is nil\nx is list\nx is map\nreturn x as bytes";
    assert_same_len(with_is, base, "dead constant type tests");

    // dead nil literal: the tag-area frame layout is a PLAN-level function
    // of nil's mere presence, so the fair comparison carries a nil in the
    // base too (assigned to a dead-store-eliminated var)
    let base_n = "var x = 1u64\nvar e = nil\nreturn x as bytes";
    let with_nil = "var x = 1u64\nvar e = nil\nnil\nreturn x as bytes";
    assert_same_len(with_nil, base_n, "dead nil literal");

    // dead LENGTH over a static list class
    let base_l = "var l = [7u64, 8u64, 9u64]\nvar s = 0u64\nreturn s as bytes";
    let with_len = "var l = [7u64, 8u64, 9u64]\nvar s = 0u64\nlength(l)\nreturn s as bytes";
    assert_same_len(with_len, base_l, "dead LENGTH(static)");
}

/// Dead statements interleaved with live ones: order and results preserved.
#[test]
fn c11_dead_interleaved_live_order() {
    let body = "var x = 0u64\n5\nx = x + 1\nsize(\"ab\")\nx = x * 10\n3 as u8\nx = x + 2\nnil\nx is u64\nreturn (x as u64) as bytes";
    // interpreter: x = 0 + 1 = 1; * 10 = 10; + 2 = 12
    assert_ok(body, &[], 12, "dead interleave");

    // dead statements inside if arms and while bodies (IRLIST arm loop)
    let body2 = "var x = 0u64\nif 1 < 2 {\n7\nx = x + 3\nsize(\"ab\")\n} else {\nx = 9\n}\nvar i = 0u64\nwhile i < 2 {\n4\nx = x + 5\ni += 1\n2 as u8\n}\nreturn (x as u64) as bytes";
    // interpreter: x = 0 + 3 = 3; loop twice: +5 +5 = 13
    assert_ok(body2, &[], 13, "dead in arms/loops");
}

/// A dead statement whose subtree carries a fault face is NOT eliminated:
/// HASKEY keeps its 72 face (strictly more bytes than the body without it).
#[test]
fn c11_haskey_not_eliminated() {
    let base = "var w = [4u64, 5u64]\nreturn 1u64 as bytes";
    let with = "var w = [4u64, 5u64]\nhas_key(w, 1)\nreturn 1u64 as bytes";
    assert_more_len(with, base, "dead HASKEY must still emit (72 face)");

    // dynamic SIZE keeps its 91 face too
    let base2 = "var b = 0x0102\nvar y = 0u64\nreturn y as bytes";
    let with2 = "var b = 0x0102\nvar y = 0u64\nsize(b)\nreturn y as bytes";
    assert_more_len(with2, base2, "dead dynamic SIZE must still emit (91 face)");
}

/// The eliminated statements' fault-free guarantee: a whitelisted dead
/// subtree can never fault on the interpreter either, so the success/fault
/// split is identical with and without them.
#[test]
fn c11_elimination_face_neutral() {
    // dead statements around a 71 face: the fault still fires unchanged
    let body = "var z = 0u64\n5\nsize(\"ab\")\n3 as u8\nreturn (5 / z) as bytes";
    assert_exit(body, &[], 90, "division fault survives dead stmts");
}

/* ==================================================================== */
/* C15 — branch_cond OR short-circuit chain (pure conditions)           */
/* ==================================================================== */

/// `if a || b` with pure operands: both when-shapes, nested chains.
/// Interpreter OR semantics are EAGER (execute.rs: `binop_btw(ops, lgc_or)`,
/// `lgc_or` extracts bool from BOTH operands) — the is_pure_cond guard is
/// what makes skipping the second operand's evaluation face-neutral.
#[test]
fn c15_or_chain_if_both_when_shapes() {
    // if-shape (when_true = false at the chain: jump to else when false)
    let body = "var r = 0u64\nvar a = byte(data, 0)\nvar b = byte(data, 1)\nif a == 1 || b == 2 { r = 10 } else { r = 20 }\nreturn (r as u64) as bytes";
    assert_ok(body, &[1, 0], 10, "if a true");
    assert_ok(body, &[0, 2], 10, "if b true");
    assert_ok(body, &[0, 0], 20, "if both false");
    assert_ok(body, &[1, 2], 10, "if both true");

    // REQ-shape (when_true = true: jump to ok)
    let body2 = "var a = byte(data, 0)\nvar b = byte(data, 1)\nrequire 77, a == 1 || b == 2\nreturn 5u64 as bytes";
    assert_ok(body2, &[1, 0], 5, "req a true");
    assert_ok(body2, &[0, 2], 5, "req b true");
    assert_exit(body2, &[0, 0], 77, "req both false");

    // assert-shape
    let body3 = "var a = byte(data, 0)\nvar b = byte(data, 1)\nassert a == 1 || b == 2\nreturn 6u64 as bytes";
    assert_ok(body3, &[0, 2], 6, "assert b true");
    assert_exit(body3, &[0, 0], 1, "assert both false");
}

#[test]
fn c15_or_chain_nested_and_inverted() {
    // nested ORs
    let body = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar c = byte(data, 2)\nvar r = 0u64\nif a == 1 || b == 2 || c == 3 { r = 100 } else { r = 200 }\nreturn (r as u64) as bytes";
    assert_ok(body, &[1, 0, 0], 100, "nested a");
    assert_ok(body, &[0, 2, 0], 100, "nested b");
    assert_ok(body, &[0, 0, 3], 100, "nested c");
    assert_ok(body, &[0, 0, 0], 200, "nested none");
    assert_ok(body, &[0, 9, 9], 200, "nested wrong keys");

    // NOT wrapping an operand
    let body2 = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = 0u64\nif a != 1 || b != 2 { r = 7 } else { r = 8 }\nreturn (r as u64) as bytes";
    assert_ok(body2, &[1, 2], 8, "not both match");
    assert_ok(body2, &[2, 2], 7, "not a off");
    assert_ok(body2, &[1, 3], 7, "not b off");

    // while-condition OR
    let body3 = "var i = 0u64\nvar n = 0u64\nwhile i < 3 || n < 1 {\nn += 1\ni += 1\n}\nreturn (i * 10 + n) as bytes";
    // interpreter (eager): i=0,n=0 → run (n=1,i=1) → i<3 true → run
    // (n=2,i=2) → i<3 true → run (n=3,i=3) → stop. i=3, n=3.
    assert_ok(body3, &[], 33, "while or");
}

/// An OR chain with an impure operand (a may-Nil slot's comparison) must
/// take the eager path — the same faults the interpreter's eager OR raises.
#[test]
fn c15_impure_or_stays_eager() {
    // x may be Nil: `x > 0` faults 90 on the interpreter (lgc_ord_cmp is
    // uint-only over to_u128(Nil)) — with a true FIRST operand the chain
    // still evaluates the second eagerly on both engines.
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nvar y = byte(data, 2)\nvar r = 0u64\nif x > 0 || y == 3 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    assert_ok(body, &[5, 0, 0], 1, "impure a true");
    assert_ok(body, &[0, 0, 3], 1, "impure b true");
    assert_ok(body, &[0, 0, 0], 2, "impure both false");
    // Nil x faults 90 — the eager path's guard (same as the interpreter)
    assert_exit(body, &[0, 1, 0], 90, "impure nil x faults");
}

/* ==================================================================== */
/* C16 — `x == K1 || x == K2 || ...` membership bitmask                 */
/* ==================================================================== */

/// N = 3..5 arms, hit / miss / boundary keys, shared dynamic x evaluated
/// once (its reads between arms cannot be observed, so hit and miss results
/// must match the interpreter's per-arm eager evaluation).
#[test]
fn c16_membership_bitmask_n3_to_n5() {
    // N=3
    let b3 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 5 || x == 9 || x == 13 { r = 111 } else { r = 222 }\nreturn (r as u64) as bytes";
    assert_ok(b3, &[5], 111, "n3 hit first");
    assert_ok(b3, &[9], 111, "n3 hit middle");
    assert_ok(b3, &[13], 111, "n3 hit last");
    assert_ok(b3, &[0], 222, "n3 miss zero");
    assert_ok(b3, &[6], 222, "n3 miss adjacent");
    assert_ok(b3, &[255], 222, "n3 miss high");
    assert_ok(b3, &[63], 222, "n3 miss 63");
    assert_ok(b3, &[64], 222, "n3 miss 64 (shift-window edge)");

    // N=4 with the literal on the LEFT of some arms
    let b4 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 2 || 7 == x || x == 11 || 19 == x { r = 1 } else { r = 0 }\nreturn ((r + x) as u64) as bytes";
    assert_ok(b4, &[2], 3, "n4 hit 2");
    assert_ok(b4, &[7], 8, "n4 hit 7");
    assert_ok(b4, &[11], 12, "n4 hit 11");
    assert_ok(b4, &[19], 20, "n4 hit 19");
    assert_ok(b4, &[8], 8, "n4 miss 8");
    assert_ok(b4, &[200], 200, "n4 miss 200");

    // N=5, keys spanning the byte domain incl. 0 and 63 (miss 64)
    let b5 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 0 || x == 1 || x == 31 || x == 62 || x == 63 { r = 500 } else { r = 600 }\nreturn (r as u64) as bytes";
    assert_ok(b5, &[0], 500, "n5 hit 0");
    assert_ok(b5, &[1], 500, "n5 hit 1");
    assert_ok(b5, &[31], 500, "n5 hit 31");
    assert_ok(b5, &[62], 500, "n5 hit 62");
    assert_ok(b5, &[63], 500, "n5 hit 63");
    assert_ok(b5, &[64], 600, "n5 miss 64");
    assert_ok(b5, &[128], 600, "n5 miss 128");
    assert_ok(b5, &[255], 600, "n5 miss 255");
}

/// x may-Nil: the 90 face fires ONCE before the mask, at the original
/// first-comparison position — same fault, same code, as the interpreter's
/// first arm evaluation.
#[test]
fn c16_membership_may_nil_x() {
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nvar r = 0u64\nif x == 4 || x == 8 || x == 15 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    assert_ok(body, &[4, 0], 1, "may-nil hit");
    assert_ok(body, &[8, 0], 1, "may-nil hit 2");
    assert_ok(body, &[9, 0], 2, "may-nil miss");
    // Nil x: exactly-one-Nil vs a literal → 90 (the interpreter's
    // lgc_equal on Nil vs U8 value... the EXACTLY-ONE-NIL face the
    // per-arm guards carried; one guard before the mask keeps it)
    assert_exit(body, &[0, 1], 90, "may-nil Nil x faults 90");
}

/// N = 2 must NOT activate the bitmask (falls to the OR chain / eager
/// path) — still correct on every face.
#[test]
fn c16_n2_not_activated() {
    let b2 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 3 || x == 6 { r = 9 } else { r = 4 }\nreturn (r as u64) as bytes";
    assert_ok(b2, &[3], 9, "n2 hit 3");
    assert_ok(b2, &[6], 9, "n2 hit 6");
    assert_ok(b2, &[7], 4, "n2 miss");
    assert_ok(b2, &[0], 4, "n2 miss 0");
}

/// A dynamic (non-slot) x shared by the arms: evaluated once, faults at
/// the original first-arm position only.
#[test]
fn c16_membership_dynamic_x_once() {
    let b = "var d = data\nvar x = byte(d, 0) + byte(d, 1)\nvar r = 0u64\nif x == 10 || x == 20 || x == 30 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    assert_ok(b, &[3, 7], 1, "dyn x hit 10");
    assert_ok(b, &[9, 11], 1, "dyn x hit 20");
    assert_ok(b, &[1, 2], 2, "dyn x miss (3)");
    // byte() OOB: the 39 face fires at the first evaluation, on both engines
    let bo = "var x = byte(data, 0) + byte(data, 5)\nvar r = 0u64\nif x == 10 || x == 20 || x == 30 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    assert_exit(bo, &[1], 71, "dyn x oob face preserved");
}

/// The bitmask must actually shrink N≥3 chains: the masked N=3 chain is not
/// BIGGER than the N=2 chain (which lowers as an eager arm pair), and going
/// from 3 to 5 masked keys costs at most a wider mask immediate — a fraction
/// of the ≥ 3 instructions a per-arm eager chain would add per key.
#[test]
fn c16_bitmask_shrinks_vs_arms() {
    let n2 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 5 || x == 9 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    let n3 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 5 || x == 9 || x == 13 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    // keys kept ≤ 15 so the mask immediate stays in the same li form and
    // the comparison isolates the per-key cost (the wide-mask encoding is
    // exercised functionally by the n5 keys 62/63 in the bitmask test)
    let n5 = "var x = byte(data, 0)\nvar r = 0u64\nif x == 5 || x == 9 || x == 13 || x == 3 || x == 11 { r = 1 } else { r = 2 }\nreturn (r as u64) as bytes";
    for mode in MODES {
        let (l2, l3, l5) = (
            code_len_mode(n2, mode),
            code_len_mode(n3, mode),
            code_len_mode(n5, mode),
        );
        assert!(l3 <= l2, "n3 mask {mode:?}: {l3} > n2 chain {l2}");
        assert!(
            l5 <= l3 + 2,
            "n5 mask {mode:?}: {l5} grew more than 2B over n3 {l3}"
        );
    }
}
