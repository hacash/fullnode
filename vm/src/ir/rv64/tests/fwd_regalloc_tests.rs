#![allow(unused_imports)]

/* ==================================================================== */
/* impl/fwd-regalloc: E9 / E3 / E2 candidates (encoding-regalloc survey). */
/*                                                                       */
/* Every behavioral face runs the REAL pipeline: calc interpreter vs all */
/* three RV64 lowering modes (expectations are the interpreter's         */
/* documented faces — fault codes are consensus codes), mirroring the    */
/* guard_elim_tests / ctrl_dse_tests discipline. Structural faces pin    */
/* exact decoded instruction shapes (E9: the guard branch compresses to  */
/* a 2-byte c.beqz in ImcRemap; E3: the banked-left mv disappears for    */
/* commutative ops and stays for non-commutative ones; E2: the forwarded */
/* GET skips its c.ld at depth > 0).                                     */
/*                                                                       */
/* Differential switches: the whole module is also run under             */
/* `RV64_NO_FWD=1` (E2's read side must disable cleanly — behavior       */
/* identical, forward hits disappear) and `RV64_NO_DSE=1` /              */
/* `RV64_INV_CHECK=1` per the suite's standard commands.                 */
/* ==================================================================== */

use super::helpers::*;
use super::super::*;
use crate::calc::decode::{BrKind, RegAluOp};
use crate::calc::{CalcFnObj, CalcRunLimits};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// Pack + run an input-param body and return the whole exec (fault faces
/// assert on the guest exit code).
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

/// Decode a stream keeping each instruction's byte length.
fn decode_len_stream(code: &[u8], profile: u8) -> Vec<(Inst, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile).unwrap();
        out.push((inst, len as usize));
        off += len as usize;
    }
    out
}

/// Decode the RX code bytes of a lowered body for `mode`.
fn decoded_mode(body: &str, mode: Rv64Mode) -> Vec<(Inst, usize)> {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, mode).unwrap();
    let profile = match mode {
        Rv64Mode::Im => 0,
        _ => crate::calc::container::CALC_PROFILE_RV64IMC,
    };
    decode_len_stream(&lo.code, profile)
}

/* ==================================================================== */
/* E9 — nil-guard zero-compare branches borrow a window register          */
/* ==================================================================== */

/// May-Nil slot consumed by a constant-rhs XOP (`x += 1` — the constant
/// absorption path evaluates nothing first): the old-value flag guard fires
/// at an EMPTY stack, so in ImcRemap it borrows vregs[0] (x10, window) and
/// the `beq sc, ZERO` compresses to c.beqz. Semantics: non-Nil path adds,
/// Nil path faults 71.
#[test]
fn e9_xop_const_rhs_empty_stack_guard() {
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nx += 1u64\nreturn (x as u64) as bytes";
    assert_ok(body, &[3, 0], 4, "xop guard non-nil");
    assert_exit(body, &[3, 1], 71, "xop guard nil -> 71");

    // structural: in ImcRemap the guard's zero-compare is a 2-byte branch
    // (c.beqz); a tagged-Nil body without the XOP (plain add of two
    // non-nil values) has no 2B zero-branch here.
    let code = decoded_mode(body, Rv64Mode::ImcRemap);
    let two_byte_zero_branches = code
        .iter()
        .filter(|(i, l)| {
            *l == 2
                && matches!(i, Inst::Branch { kind: BrKind::Eq, rs2: 0, .. })
        })
        .count();
    assert!(two_byte_zero_branches >= 1, "expected a compressed c.beqz guard site");
}

/// Same guard site, non-empty stack context (`t + x` with both operands
/// evaluated): the guard falls back to the dedicated scratch — the value
/// faces are unchanged (the sc1 form is never wrong, only wider).
#[test]
fn e9_binary_pair_guard_faces() {
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nvar t = byte(data, 2)\nvar r = t * 3u64 + x\nreturn (r as u64) as bytes";
    assert_ok(body, &[2, 0, 4], 14, "pair guard non-nil");
    assert_exit(body, &[2, 1, 4], 71, "pair guard nil -> 71");
}

/// The borrow kills live forwarding entries naming the borrowed register:
/// after the guard fires (or falls through), later reads of forwarded
/// slots re-load from the frame and see the right values.
#[test]
fn e9_borrow_kills_fwd_entries() {
    // `t = x + 1` leaves fwd_int[t] = Some(vregs[0]) (the stored value's
    // register); the next statement's `x += 5` guard fires at an empty
    // stack, borrows vregs[0] and kills that entry — the final read of t
    // must re-load from the frame (x's borrow never corrupted it).
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nvar t = x + 1u64\nx += 5u64\nvar r = t * 100u64 + x\nreturn (r as u64) as bytes";
    // x=3: t=4, x=8 -> 408 ; x=nil: guard 71 fires at `t = x + 1`
    assert_ok(body, &[3, 0, 0], 408, "borrow-kill face non-nil");
    assert_exit(body, &[3, 1, 0], 71, "borrow-kill face nil");
}

/* ==================================================================== */
/* E3 — pop2_dest banked-Ref left operand: the materializing mv is        */
/* skipped for commutative ops (the caller emits `d = rb OP ra` straight  */
/* out of the bank home); non-commutative ops keep the mv.                */
/* ==================================================================== */

/// Three banked locals so the web promotes a/b/c to register homes; every
/// binary then has a banked-Ref LEFT operand and a banked-Ref right operand
/// (Ref+Ref shape — rb is only ever read in the swapped form).
#[test]
fn e3_commutative_ops_banked_left() {
    let head = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar c = byte(data, 2)\n";
    // ADD MUL XOR OR AND — values
    let body = head.to_string()
        + "var r = a + b\nvar s = a * b\nvar t = a ^ b\nvar u = a | b\nvar v = a & b\nvar w = 0u64\nvar z = 0u64\n"
        + "return (((r as u64) * 1000000u64 + (s as u64) * 100000u64 + (t as u64) * 10000u64 + (u as u64) * 1000u64 + (v as u64) * 100u64 + (w as u64) * 10u64 + z as u64) as u64) as bytes";
    // a=7 b=3: 10, 21, 5, 7, 3
    assert_ok(&body, &[7, 3, 2], 12_147_300, "commutative matrix");

    // EQ / NEQ (the xor+sltiu / xor+sltu chains)
    let body3 = head.to_string()
        + "var r = a == b\nvar s = a != b\nvar t = a == a\nreturn ((r as u64) * 100u64 + (s as u64) * 10u64 + (t as u64)) as bytes";
    assert_ok(&body3, &[7, 3, 0], 11, "eq/neq matrix");
    let body4 = head.to_string()
        + "var r = a == b\nreturn ((r as u64) * 100u64 + (b as u64)) as bytes";
    assert_ok(&body4, &[7, 7, 0], 107, "eq true");
}

/// Result aliasing: the same bank home on BOTH sides of the op, and the
/// result written back into an operand's own home.
#[test]
fn e3_aliased_operands() {
    let head = "var a = byte(data, 0)\nvar b = byte(data, 1)\n";
    let body = head.to_string() + "a = a * a + b\nvar r = a * a\nreturn (r as u64) as bytes";
    // a=3 b=4: a=13, r=169
    assert_ok(&body, &[3, 4], 169, "aliased operands");
}

/// Non-commutative ops MUST keep the materializing mv (operand order is
/// part of the encoding): SUB/LT/DIV values + the underflow face.
#[test]
fn e3_non_commutative_keep_mv() {
    let head = "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar c = byte(data, 2)\n";
    let body = head.to_string()
        + "var r = a - b\nvar s = if a < b { 1u64 } else { 0u64 }\nvar t = a / b\nreturn ((r as u64) * 100u64 + (s as u64) * 10u64 + (t as u64)) as bytes";
    assert_ok(&body, &[7, 3, 0], 402, "sub/lt/div values");
    // checked sub underflow: 3 - 7 faults 90 in every mode (the mv'd
    // two-address form feeds the same bltu face)
    assert_exit(&body, &[3, 7, 0], 90, "sub underflow face");
}

/* ---- structural pins (ImcRemap decoded stream) ---- */

/// A 3-distinct-register MUL (`mul d, ra, rb` with d, ra, rb pairwise
/// distinct) can ONLY come from the E3 swapped form — the two-address path
/// always multiplies into the left value's register (rd == rs1).
#[test]
fn e3_structural_swapped_mul_no_mv() {
    let head = "var a = byte(data, 0)\nvar b = byte(data, 1)\n";
    let body = head.to_string() + "var r = a * b\nreturn (r as u64) as bytes";
    let code = decoded_mode(&body, Rv64Mode::ImcRemap);
    let hit = code.iter().enumerate().any(|(i, (inst, _))| match inst {
        Inst::RegAlu { op: RegAluOp::Mul, rd, rs1, rs2 } => {
            rd != rs1 && rd != rs2 && rs1 != rs2
                && !code[..i].iter().rev().take(3).any(|(j, _)| {
                    matches!(j, Inst::RegAlu { op: RegAluOp::Add, rd: mrd, rs1: 0, rs2: m } if *mrd == *rd && *m == *rs1)
                })
        }
        _ => false,
    });
    assert!(hit, "expected a swapped-destination mul with no preceding mv");
}

/// SUB keeps the mv: a c.mv into the destination immediately feeding a
/// two-address SUB (rd == rs1).
#[test]
fn e3_structural_sub_keeps_mv() {
    let head = "var a = byte(data, 0)\nvar b = byte(data, 1)\n";
    let body = head.to_string() + "var r = a - b\nreturn (r as u64) as bytes";
    let code = decoded_mode(&body, Rv64Mode::ImcRemap);
    let hit = code.windows(3).any(|w| match (&w[0].0, &w[2].0) {
        (
            Inst::RegAlu { op: RegAluOp::Add, rd: mrd, rs1: 0, rs2: _ },
            Inst::RegAlu { op: RegAluOp::Sub, rd, rs1, rs2: _ },
        ) => mrd == rd && rd == rs1,
        _ => false,
    });
    assert!(hit, "expected mv(d, bank) feeding a two-address sub");
}

/* ==================================================================== */
/* E2 — forwarded GET re-issue generalized to `r == vregs[j], j <= depth`. */
/* The hit shape: the slot's PUT leaves fwd_int[x] = Some(vregs[1]) (the   */
/* XOP old-value borrow), the next statement materializes a BANKED left    */
/* operand first (push_ref — no pool hand, the entry survives), and reads  */
/* x at depth 1 — the old rule (empty stack only) fell back to c.ld.       */
/* Bank starvation (E2 test scaffolding): 18 slots carry read-count 5 so   */
/* they win every bank home (allocate_bank sorts by slot event count),     */
/* leaving the count-4 XOP slot in its frame cell.                         */
/* ==================================================================== */

fn e2_hit_bodies() -> (String, String) {
    let mut decl = String::new();
    for i in 0..19 {
        decl.push_str(&format!("var a{i} = byte(data, {i})\n"));
    }
    // staggered high-count reads: a_i (i != 17) read 4x -> count 5
    let mut hot = String::new();
    for i in 0..19 {
        if i == 17 {
            continue;
        }
        hot.push_str(&format!("var t{i} = a{i} + a{i} + a{i} + a{i}\n"));
    }
    let base = format!("{decl}{hot}a17 += 300u64\n");
    // A: a17 read at depth 1 (after the banked a0 Ref) -> E2 hit, no c.ld
    let body_a = format!("{base}var r = a0 + a17\nreturn (r as u64) as bytes");
    // B: a17 read at depth 0 (j = 1 > 0 — correctly NOT re-issued) -> c.ld
    let body_b = format!("{base}var r = a17 + a0\nreturn (r as u64) as bytes");
    (body_a, body_b)
}

fn e2_frame_lds(body: &str) -> usize {
    decoded_mode(body, Rv64Mode::ImcRemap)
        .iter()
        .filter(|(i, _)| matches!(i, Inst::Load { op: crate::calc::decode::LoadOp::Ld, .. }))
        .count()
}

/// Structural: the depth-1 read re-issues the forwarded register (one fewer
/// frame load than the depth-0 variant). Skipped under RV64_NO_FWD — the
/// differential there is correctness, not byte counts.
#[test]
fn e2_forwarded_get_at_depth_hits() {
    let (body_a, body_b) = e2_hit_bodies();
    // lower once so the env switch is latched into FWD_DISABLED
    let _ = decoded_mode(&body_a, Rv64Mode::ImcRemap);
    if FWD_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("RV64_NO_FWD active — structural hit pin skipped");
        return;
    }
    let la = e2_frame_lds(&body_a);
    let lb = e2_frame_lds(&body_b);
    assert_eq!(la, lb - 1, "depth-1 forwarded GET must skip exactly one c.ld");
}

/// Semantic faces of the hit shape in all three modes: the re-issued
/// register carries the post-XOP value (the borrow's kill keeps it honest).
#[test]
fn e2_forwarded_get_value_faces() {
    let (body_a, body_b) = e2_hit_bodies();
    let mut input = vec![0u8; 19];
    input[0] = 40;
    input[17] = 9;
    // r = a0 + (a17 + 300) = 349; body B is the same value, operand-swapped
    for (tag, body) in [("A", &body_a), ("B", &body_b)] {
        for mode in MODES {
            let out = run_input_mode(body, &input, mode);
            assert_eq!(out, 349u64.to_be_bytes().to_vec(), "{tag} mode {mode:?}");
        }
    }
    // the fault face rides through: a17 may-Nil -> the XOP old-value guard
    // faults 71 before anything else
    let nil_base = {
        let (a, _) = e2_hit_bodies();
        a.replacen(
            "var a17 = byte(data, 17)\n",
            "var a17 = byte(data, 17)\nif byte(data, 18) == 1u64 { a17 = nil }\n",
            1,
        )
    };
    let mut nil_in = vec![0u8; 19];
    nil_in[18] = 1;
    assert_exit(&nil_base, &nil_in, 71, "may-nil XOP slot faults 71");
}

/// Regression face for the E2 companion kill: a fwd entry naming the
/// register an XOP overwrites (fr hit or empty-stack borrow) must die —
/// the generalized read side can re-issue vregs[1] at depth >= 1, so a
/// stale entry there would feed the WRONG value to the consumer. The
/// interpreter truth is z = 5, x = 5 + b in every mode.
#[test]
fn e2_stale_fwd_entries_killed() {
    for b in [100u64, 7] {
        let body = "var k = 5u64\nvar x = k\nvar b = byte(data, 0)\nx += b\nvar z = k\nreturn ((z as u64) * 1000u64 + (x as u64)) as bytes";
        let input = vec![b as u8];
        for mode in MODES {
            let out = run_input_mode(body, &input, mode);
            let want = (5 * 1000 + 5 + b) as u64;
            assert_eq!(out, want.to_be_bytes().to_vec(), "stale fwd probe mode {mode:?}");
        }
    }
}

/// place/join barrier: after an if/else the fwd entries die wholesale, so
/// the reads take the frame path and still see the right arm's value.
#[test]
fn e2_join_barrier_faces() {
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = 7u64 } else { x = 9u64 }\nvar s = x\nvar r = s * 10u64 + x\nreturn (r as u64) as bytes";
    assert_ok(&body, &[0, 0], 99, "join else arm");
    assert_ok(&body, &[0, 1], 77, "join then arm");
}

/// u128 and bytes slots keep their own read rules (no single-register
/// forwarding; the byte-pair rule is untouched) — plain value faces.
#[test]
fn e2_u128_and_bytes_slots_unaffected() {
    let u = "var x = 1u128\nx += byte(data, 0) as u128\nvar y = x * 2u128\nreturn (y as u64) as bytes";
    assert_ok(u, &[5], 12, "u128 slot read after XOP");
    let b = "var s = size(data)\nvar r = 0u64\nvar i = 0u64\nwhile i < s { r += byte(data, i); i += 1 }\nreturn (r as u64) as bytes";
    assert_ok(b, &[1, 2, 3], 6, "bytes-shaped loop reads");
}
