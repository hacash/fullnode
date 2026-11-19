#![allow(unused_imports)]

/* ==================================================================== */
/* C1+C2 (rv64-size-min): width-provable guard elimination.              */
/*                                                                       */
/* C1 — CU8/CU16/CU32 of an int operand whose static width already fits  */
/*      the target skips the `li_k(limit)+bltu` range guard (the width   */
/*      invariant: every runtime value of width w is < 2^w, so the      */
/*      guard is unreachable; u8→CU8/16/32, u16→CU16/32, u32→CU32).     */
/* C2 — a list index/key of static width ≤ 32 skips the extract_u32     */
/*      `li(1<<32)+bgeu` 72 guard at the variable-index sites           */
/*      (ITEMGET/ITEMSET/INSERT/REMOVE/HASKEY/pair); the Nil→72 tag     */
/*      guard and the 85 bounds guard keep their positions.             */
/*                                                                       */
/* Every body runs through the REAL pipeline in all three lowering      */
/* modes; expectations are the interpreter's documented faces           */
/* (cast.rs cast_to_uint_width / to_u128: r == limit is IN range,       */
/* r > limit → CastFail 71, to_u128(Nil) = 0; list extract_u32:         */
/* Nil → 72, ≥ 2³² → 72, then idx ≥ len → 85).                          */

use super::helpers::{compile_with_input, run_input_mode};
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

/* ---- C1: narrowing cast guards ------------------------------------- */

/// u8-source → CU8/CU16/CU32 (all removed), boundary 2^8−1 in range.
#[test]
fn c1_u8_source_casts() {
    let body = "var x = byte(data, 0)\nvar r = ((x as u8) as u64) * 1000000u64 + ((x as u16) as u64) * 1000u64 + (x as u32) as u64\nreturn r as bytes";
    assert_ok(body, &[7], 7 * 1000000 + 7 * 1000 + 7, "u8 casts");
    // 255 == every removed guard's CU8 limit: r == limit is IN range
    let lim = 255u64;
    assert_ok(body, &[255], lim * 1000000 + lim * 1000 + lim, "u8 2^8-1");
    assert_ok(body, &[0], 0, "u8 zero");
}

/// u16-source → CU16/CU32 removed (boundary 2^16−1), CU8 kept (300 → 71,
/// 205 → 205 — the guard survives exactly where the width does not fit).
#[test]
fn c1_u16_source_casts() {
    let body = "var x = size(data)\nvar r = ((x as u16) as u64) * 1000u64 + (x as u32) as u64\nreturn r as bytes";
    assert_ok(body, &vec![0u8; 205], 205 * 1000 + 205, "u16 205");
    // 65535 == the CU16 limit — in range on the REMOVED guard (a u16-width
    // slot; the huge size() input itself would blow the L1 input budget)
    let bmax = "var x = 65535u64 as u16\nvar r = ((x as u16) as u64) * 1000u64 + (x as u32) as u64\nreturn r as bytes";
    assert_ok(bmax, &[], 65535 * 1000 + 65535, "u16 2^16-1");
    // kept guard: 300 > 255 → CastFail 71
    let b8 = "var x = size(data)\nreturn ((x as u8) as u64) as bytes";
    assert_ok(b8, &vec![0u8; 255], 255, "u16->u8 255 in range");
    assert_exit(b8, &vec![0u8; 300], 71, "u16->u8 300");
}

/// u32-source → CU32 removed at the 2^32−1 boundary; CU8/CU16 kept
/// (2^32−1 overflows both → 71).
#[test]
fn c1_u32_source_casts() {
    let body = "var x = 4294967295u64 as u32\nvar r = (x as u32) as u64\nreturn r as bytes";
    assert_ok(body, &[], 4294967295, "u32 2^32-1");
    let b16 = "var x = 4294967295u64 as u32\nreturn ((x as u16) as u64) as bytes";
    assert_exit(b16, &[], 71, "u32->u16 kept guard");
    let b8 = "var x = 256u64\nreturn ((x as u8) as u64) as bytes";
    assert_exit(b8, &[], 71, "u64->u8 kept guard");
    assert_ok(
        "var x = 255u64\nreturn ((x as u8) as u64) as bytes",
        &[],
        255,
        "u64->u8 255 in range",
    );
}

/// may-Nil narrow slot: the interpreter's relaxed to_u128(Nil) = 0 — no
/// fault, value 0 (the 71 face belongs to the OVERFLOW guard, which a
/// u64-width may-Nil slot still runs: 300 → 71, Nil → 0).
#[test]
fn c1_may_nil_slot_narrowing() {
    let body = "var x = byte(data, 0)\nif byte(data, 1) == 1u64 { x = nil }\nvar r = ((x as u8) as u64) * 10u64 + (x as u32) as u64\nreturn r as bytes";
    assert_ok(body, &[7, 0], 77, "may-nil narrow assigned");
    assert_ok(body, &[7, 1], 0, "may-nil narrow Nil->0");
    let b64 = "var x = 300u64\nif byte(data, 0) == 1u64 { x = nil }\nreturn ((x as u8) as u64) as bytes";
    assert_ok(b64, &[1], 0, "may-nil u64 Nil->0");
    assert_exit(b64, &[0], 71, "may-nil u64 300 keeps 71");
}

/* ---- C2: list index 72 guards -------------------------------------- */

/// byte()-width (u8) variable index: read + 85-after face (the 72 guard
/// is gone; an out-of-bounds narrow index still faults 85).
#[test]
fn c2_itemget_u8_index() {
    let body = "var l = [10u64, 20u64, 30u64]\nvar i = byte(data, 0)\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_ok(body, &[0], 10, "u8 idx 0");
    assert_ok(body, &[1], 20, "u8 idx 1");
    assert_ok(body, &[2], 30, "u8 idx 2");
    assert_exit(body, &[255], 85, "u8 idx 255 -> 85");
}

/// u16-width index (size), u32-width index (in range + 100 ≥ len → 85),
/// u64-width index ≥ 2³² → 72 (unprovable — the guard stays).
#[test]
fn c2_itemget_u16_u32_u64_index() {
    let b16 = "var l = [10u64, 20u64, 30u64]\nvar i = size(data)\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_ok(b16, &[0xab, 0xcd], 30, "u16 idx 2");
    let b32 = "var l = [10u64, 20u64, 30u64]\nvar i = 2 as u32\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_ok(b32, &[], 30, "u32 idx 2");
    let b32o = "var l = [10u64, 20u64, 30u64]\nvar i = 100 as u32\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_exit(b32o, &[], 85, "u32 idx 100 -> 85 (not 72)");
    let bu = "var l = [10u64]\nvar i = 4294967296u64\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_exit(bu, &[], 72, "u64 idx 2^32 -> 72 (guard kept)");
    let bu2 = "var l = [10u64]\nvar i = 0u64\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_ok(bu2, &[], 10, "u64 idx 0");
}

/// may-Nil narrow index: the Nil→72 TAG guard fires first (untouched by
/// C2); a non-Nil narrow index that is out of bounds faults 85.
#[test]
fn c2_nil_index_72_first() {
    let body = "var l = [10u64, 20u64, 30u64]\nvar i = byte(data, 0)\nif byte(data, 1) == 1u64 { i = nil }\nvar r = l[i]\nreturn (r as u64) as bytes";
    assert_ok(body, &[0, 0], 10, "nil-idx taken/no");
    assert_exit(body, &[0, 1], 72, "Nil index -> 72 first");
    assert_exit(body, &[9, 0], 85, "narrow OOB -> 85");
}

/// ITEMSET (l[i] = v and item_set) with a u8-width index: write lands,
/// OOB still faults 85 after the removed 72 guard.
#[test]
fn c2_itemset_u8_index() {
    let body = "var l = [1u64, 2u64, 3u64]\nvar i = byte(data, 0)\nl[i] = 99u64\nvar r = (l[0] * 100u64 + l[1]) * 100u64 + l[2]\nreturn (r as u64) as bytes";
    assert_ok(body, &[1], 19903, "itemset u8 idx 1");
    assert_exit(body, &[9], 85, "itemset u8 OOB -> 85");
    let b2 = "var l = [1u64, 2u64, 3u64]\nvar i = byte(data, 0)\nitem_set(l, i, 7u64)\nvar r = (l[0] * 100u64 + l[1]) * 100u64 + l[2]\nreturn (r as u64) as bytes";
    assert_ok(b2, &[2], 10207, "item_set u8 idx 2");
}

/// INSERT / REMOVE with a u8-width index (arena class): the 72 guard is
/// skipped, the 81 (insert past len) / 85 (remove bounds) faces stay.
#[test]
fn c2_insert_remove_u8_index() {
    let bins = "var l = [1u64, 2u64, 3u64]\nvar i = byte(data, 0)\ninsert(l, i, 99u64)\nvar r = ((l[0] * 100u64 + l[1]) * 100u64 + l[2]) * 10u64 + length(l)\nreturn (r as u64) as bytes";
    assert_ok(bins, &[1], 199024, "insert u8 idx 1");
    assert_exit(bins, &[5], 81, "insert u8 past len -> 81");
    let brem = "var l = [1u64, 2u64, 3u64, 4u64]\nvar i = byte(data, 0)\nremove(l, i)\nvar r = ((l[0] * 100u64 + l[1]) * 100u64 + l[2]) * 10u64 + length(l)\nreturn (r as u64) as bytes";
    assert_ok(brem, &[1], 103043, "remove u8 idx 1");
    assert_exit(brem, &[9], 85, "remove u8 OOB -> 85");
}

/// has_key with u8-width keys (the fold-to-false compare survives).
#[test]
fn c2_haskey_u8_key() {
    let body = "var w = [4u64, 5u64]\nvar i = byte(data, 0)\nvar r = (has_key(w, i) as u64) * 10u64 + (has_key(w, byte(data, 1)) as u64)\nreturn (r as u64) as bytes";
    assert_ok(body, &[1, 9], 10, "haskey u8 in/out");
    assert_ok(body, &[0, 1], 11, "haskey u8 both in");
}

/// remove+insert at the same u8-width index — the fused pair lowering's
/// 72 guard is skipped too; the 85 remove bound still fires.
#[test]
fn c2_pair_remove_insert_u8_index() {
    let body = "var l = [1u64, 2u64, 3u64]\nvar i = byte(data, 0)\nremove(l, i)\ninsert(l, i, 99u64)\nvar r = ((l[0] * 100u64 + l[1]) * 100u64 + l[2]) * 10u64 + length(l)\nreturn (r as u64) as bytes";
    assert_ok(body, &[1], 199033, "pair u8 idx 1");
    assert_exit(body, &[9], 85, "pair u8 OOB -> 85");
}

/* ---- activation: the guards really leave the stream ---------------- */

fn insts_of(body: &str) -> usize {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, Rv64Mode::Im).unwrap();
    lo.stats.insts
}

/// Same bodies with the operand/index widened to u64: the difference is
/// exactly the dead guards (C1 ≥ 2 insts: li_k+bltu; C2 ≥ 3: li+bgeu).
#[test]
fn c1_c2_guard_removal_shrinks_code() {
    // C1 u8→CU16: provable width skips li_k(0xFFFF)+bltu
    let a = insts_of("var x = byte(data, 0)\nvar y = (x as u16) as u64\nreturn y as bytes");
    let b = insts_of("var x = byte(data, 0) as u64\nvar y = (x as u16) as u64\nreturn y as bytes");
    assert!(b - a >= 2, "C1 u8->CU16 delta {b}-{a} < 2");
    // C1 u8→CU32: skips li_k(0xFFFF_FFFF)+bltu (3 insts)
    let a = insts_of("var x = byte(data, 0)\nvar y = (x as u32) as u64\nreturn y as bytes");
    let b = insts_of("var x = byte(data, 0) as u64\nvar y = (x as u32) as u64\nreturn y as bytes");
    assert!(b - a >= 3, "C1 u8->CU32 delta {b}-{a} < 3");
    // C2 u8 index: skips li(1<<32)+bgeu
    let a = insts_of("var l = [10u64, 20u64, 30u64]\nvar i = byte(data, 0)\nvar r = l[i]\nreturn (r as u64) as bytes");
    let b = insts_of("var l = [10u64, 20u64, 30u64]\nvar i = byte(data, 0) as u64\nvar r = l[i]\nreturn (r as u64) as bytes");
    assert!(b - a >= 3, "C2 u8 idx delta {b}-{a} < 3");
}
