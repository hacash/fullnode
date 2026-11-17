//! T1 (task-1 width closure): the interpreter's dynamic-width semantics,
//! pinned engine-vs-engine on the rv64 channel.
//!
//! The interpreter (`value/cast.rs arithmetic_args2` + `binop_arithmetic`)
//! widens each integer pair to the wider side and checks the overflow face
//! AT that width; shifts are `checked_shl`/`checked_shr` (the AMOUNT must
//! be < the width; the value side truncates silently); `as bytes` is
//! `scalar_bytes` (width-exact big-endian, bool → 1 canonical byte).
//! Every row below is an agree4 (interpreter == all three rv64 modes,
//! value or exit code) unless marked as a compile reject — the two legal
//! closure outcomes of the iron law.
//!
//! Semantics corrections vs the pre-study T1 draft table (verified by
//! probe against the interpreter before any rv64 change):
//! * `u64::MAX << 1` is Ok(0xFF..FE), NOT Err(90) — checked_shl only
//!   guards the amount; the value side truncates.
//! * mixed-width `255u8 + 1u64` is Ok(256) — the interpreter widens the
//!   pair before computing; it does NOT fault.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

/// Interpreter result mapped to the rv64 observable shape.
fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

/// Four-way agreement on EVERY input: interpreter == all three rv64 modes.
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    for input in inputs {
        let want = interp_obs(&it, input);
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

fn rv64_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must contain {frag:?}: {err:?} (body: {body})"
    );
}

/// Interpreter acceptance proof: the body compiles AND runs (any outcome).
fn interp_accepts(body: &str, run: bool) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    if run {
        let _ = it.run(&[1, 2, 3, 4]);
    }
}

/* ================================================================== */
/* Section 1: shift failure forms (A1)                                 */
/* ================================================================== */

#[test]
fn t1_shift_amount_beyond_u64_width() {
    // reg form: amount >= 64 faults 90 on both engines
    let body = "var a = 1u64\nvar b = byte(data, 0) as u64\nreturn (a << b) as bytes";
    let inputs: Vec<Vec<u8>> = [63u8, 64, 65, 128, 255]
        .iter()
        .map(|&b| vec![b])
        .collect();
    agree4(body, &inputs);
    // const form: `1u64 << 64` folds nothing, traps unconditionally
    agree4("return (1u64 << 64) as bytes", &[vec![]]);
    agree4("return (1u64 << 65) as bytes", &[vec![]]);
    agree4("var a = 7u64\nreturn (a >> 64) as bytes", &[vec![]]);
    // literal-slot form (the pre-study plan row): the amount lives in a
    // u64 slot, so the reg form's width check faults even though 64 is
    // known at compile time — no constant absorption through a slot
    agree4("var a = 1u64\nvar b = 64u64\nreturn (a << b) as bytes", &[vec![]]);
}

#[test]
fn t1_shift_value_side_truncates_not_faults() {
    // checked_shl guards the AMOUNT only: MAX << 1 keeps the low bits.
    // (The pre-study T1 draft expected Err(90) here — wrong; pinned by
    // direct interpreter probe before any rv64 change.)
    agree4(
        "var a = 18446744073709551615u64\nreturn (a << 1) as bytes",
        &[vec![]],
    );
    agree4("var a = 2u64\nreturn (a << 63) as bytes", &[vec![]]);
    // narrow-width truncation: 200u8 << 1 is Some(144) in u8 checked_shl
    agree4(
        "var a = 200u8\nreturn (a << 1u8) as bytes",
        &[vec![]],
    );
    agree4(
        "var a = 200u8\nreturn (a << 7u8) as bytes",
        &[vec![]],
    );
}

#[test]
fn t1_shift_amount_beyond_narrow_width() {
    // u8 pair: amount >= 8 faults (both engines, post-A3)
    let body = "var hv = byte(data, 0) / 16\nvar r = (1 << hv)\nreturn r as bytes";
    let inputs: Vec<Vec<u8>> = [0x07u8, 0x17, 0x80, 0xF0, 0xFF]
        .iter()
        .map(|&b| vec![b])
        .collect();
    agree4(body, &inputs);
}

/* ================================================================== */
/* Section 2: narrow-width arithmetic (A3)                             */
/* ================================================================== */

#[test]
fn t1_narrow_add_sub_mul() {
    // u8 add: 255 + 1 faults 90 (same result chosen: domain check)
    agree4("var a = 255u8\nreturn (a + 1u8) as bytes", &[vec![]]);
    agree4("var a = 254u8\nreturn (a + 1u8) as bytes", &[vec![]]);
    // u8 sub underflow: exact at every width
    agree4("var a = 3u8\nreturn (a - 5u8) as bytes", &[vec![]]);
    agree4("var a = 3u8\nreturn (a - 3u8) as bytes", &[vec![]]);
    // u8 mul: 200 * 2 = 400 >= 256 faults; 100 * 2 = 200 fits
    agree4("var a = 200u8\nreturn (a * 2u8) as bytes", &[vec![]]);
    agree4("var a = 100u8\nreturn (a * 2u8) as bytes", &[vec![]]);
    // u16 face via size(): 40 entries / 24 = 1; (size/24) * 40000 >= 2^16
    // faults on a 393216-byte input — too big for the harness; the u8/u32
    // faces below cover the same check instead
    agree4("var a = 70000u32\nreturn (a * 70000u32) as bytes", &[vec![]]);
    agree4("var a = 65535u32\nreturn (a * 2u32) as bytes", &[vec![]]);
}

#[test]
fn t1_mixed_width_pairs_widen_not_fault() {
    // The interpreter widens the pair to the wider side BEFORE computing:
    // 255u8 + 1u64 = Ok(256) at u64. rv64 computes the same at the static
    // max width. (Pre-study draft expected Err(90) — corrected.)
    agree4("var a = 255u8\nvar b = 1u64\nreturn (a + b) as bytes", &[vec![]]);
    agree4("var a = 255u8\nvar b = 1u64\nreturn (a * b) as bytes", &[vec![]]);
    // narrow << u64 amount keeps the u64 plane (no u8 truncation)
    agree4(
        "var a = 200u8\nreturn (a << 1u64) as bytes",
        &[vec![]],
    );
    // narrow << narrow mixed widths widen to the wider side: 1u8 << 9u16
    // is Ok(U16(512)) — amount 9 < width 16, no u8 plane involved
    agree4(
        "var a = 1u8\nvar b = 9u16\nreturn (a << b) as bytes",
        &[vec![]],
    );
    // u8 widening into a u16 sum: b0*256 + b1 <= 65535 always fits
    let body = "var b0 = byte(data, 0)\nvar b1 = byte(data, 1)\nvar s = b0 * 256 + b1\nreturn s as bytes";
    let inputs: Vec<Vec<u8>> = [vec![0x00, 0x00], vec![0xFF, 0xFF], vec![0x12, 0x34]].to_vec();
    agree4(body, &inputs);
}

#[test]
fn t1_mixed_width_comparisons_stay_compilable() {
    // ordered compare and == are NUMERIC on widened values (lgc_ord_cmp /
    // value_content_eq): u8 vs u64 stays compilable with identical results
    let body = "var a = byte(data, 0)\nvar b = 200u64\nvar r = if a < b { 1 } else { 2 }\nreturn r as bytes";
    let inputs: Vec<Vec<u8>> = [3u8, 199, 200, 255].iter().map(|&b| vec![b]).collect();
    agree4(body, &inputs);
    let body = "var a = byte(data, 0)\nvar b = 3u64\nvar r = if a == b { 1 } else { 2 }\nreturn r as bytes";
    let inputs: Vec<Vec<u8>> = [3u8, 4, 255].iter().map(|&b| vec![b]).collect();
    agree4(body, &inputs);
}

#[test]
fn t1_narrow_div_mod_have_no_overflow_face() {
    // div/mod at u8: value equals the u64 computation; the score_sum leaf
    // `b0 / 16` must stay compilable with identical results
    let body = "var b = byte(data, 0)\nreturn (b / 16) as bytes";
    let inputs: Vec<Vec<u8>> = [0u8, 15, 16, 255].iter().map(|&b| vec![b]).collect();
    agree4(body, &inputs);
    let body = "var b = byte(data, 0)\nreturn (b % 16) as bytes";
    agree4(body, &inputs);
}

#[test]
fn t1_narrow_pow_faults_at_width() {
    let body = "var b = byte(data, 0)\nvar r = (2 ** b)\nreturn r as bytes";
    let inputs: Vec<Vec<u8>> = [0u8, 1, 7, 8, 9, 20].iter().map(|&b| vec![b]).collect();
    agree4(body, &inputs);
}

#[test]
fn t1_narrow_xop_counters_fault_at_width() {
    // a u8 counter loop: `i += 1` faults at 256 on both engines (the
    // compound assignment keeps the slot/rhs common width)
    let body = "var i = 0u8\nvar k = 0u64\nwhile k < 300 { i += 1\nk += 1 }\nreturn i as bytes";
    agree4(body, &[vec![]]);
}

/* ================================================================== */
/* Section 3: `as bytes` widths (A2)                                   */
/* ================================================================== */

#[test]
fn t1_as_bytes_width_table() {
    // scalar_bytes: u8 → 1 B, u16 → 2, u32 → 4, u64 → 8, bool → 1 canonical
    agree4("return 5u8 as bytes", &[vec![]]);
    agree4("return 258u16 as bytes", &[vec![]]);
    agree4("return 70000u32 as bytes", &[vec![]]);
    agree4("return 5u64 as bytes", &[vec![]]);
    agree4("return true as bytes", &[vec![]]);
    agree4("return false as bytes", &[vec![]]);
    // a bytes literal passes through (identity cast)
    agree4("return \"ab\" as bytes", &[vec![]]);
    // runtime values of each width
    agree4("var b = byte(data, 0)\nreturn b as bytes", &[vec![0xAB]]);
    agree4("var n = size(data)\nreturn n as bytes", &[vec![0xAB; 4]]);
    // LENGTH → u32: the 4-byte view
    agree4("var l = [1u64, 2u64]\nreturn (length(l)) as bytes", &[vec![]]);
    // u8 arithmetic result: 100*2 = 200 stays u8 (1-byte view)
    agree4("var a = 100u8\nreturn (a * 2u8) as bytes", &[vec![]]);
    // the u64-pinned wire form keeps the 8-byte view
    agree4("return (5u8 as u64) as bytes", &[vec![]]);
}

#[test]
fn t1_as_bytes_unknown_width_rejects() {
    // A bool mixed with ints in one slot has a dynamically-widthed `as
    // bytes` view (the interpreter emits [0,0] for the u16 path and [1]
    // for the bool path) — no static width exists, so the kernel subset
    // rejects; the interpreter accepts and runs. (A runtime comparison is
    // the mixing shape; a fully-literal comparison folds to a uint at the
    // shared front end and never mixes.)
    let body = "var t = 0u16\nif size(data) > 100 { t = (size(data) < 200) }\nreturn t as bytes";
    rv64_must_reject(
        body,
        "the operand has no static uint width",
    );
    interp_accepts(body, true);
    // pure bool through a slot is fine (1 canonical byte)
    agree4("var t = 1u64 < 2u64\nreturn t as bytes", &[vec![]]);
}

/* ================================================================== */
/* Section 4: fused byte-idiom gating (A3)                             */
/* ================================================================== */

#[test]
fn t1_fused_le_gate_bare_shift_faults() {
    // `byte << 8` with a BARE literal: the interpreter faults at u8 — the
    // fused lhu load must NOT fire (it would compute); the unfused path's
    // narrow check faults identically
    agree4(
        "var b = byte(data, 0)\nvar c = byte(data, 1)\nvar s = (b << 8) | c\nreturn s as bytes",
        &[vec![0xAB, 0xCD]],
    );
    // width-pinned amount fuses and agrees
    agree4(
        "var b = byte(data, 0)\nvar c = byte(data, 1)\nvar s = ((b << 8u64) | c)\nreturn s as bytes",
        &[vec![0xAB, 0xCD]],
    );
    // BE mul-add group: multiplier literals keep the interpreter
    // overflow-free — stays fused, width-exact 2-byte view
    agree4(
        "var b = byte(data, 0)\nvar c = byte(data, 1)\nvar s = b * 256 + c\nreturn s as bytes",
        &[vec![0xAB, 0xCD]],
    );
}

/* ================================================================== */
/* Section 5: the hacdtestnet kernels (T6 anchor rows)                  */
/* ================================================================== */

/// Field-major score_sum wire (names[6n] || numbers[8n] || vg[10n]) with
/// every value inside the narrow domains: vg nibbles <= 7 and decimal
/// digits <= 7 — the interpreter computes every narrow face in range.
fn score_wire_safe(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut wire = Vec::with_capacity(n * 24);
    for _ in 0..n * 6 {
        wire.push(b'A' + (next() % 26) as u8);
    }
    for _ in 0..n {
        // decimal digits 0..=7 only
        let num: u64 = (0..19).fold(0u64, |a, k| a * 10 + (next() % 8));
        wire.extend_from_slice(&num.to_be_bytes());
    }
    for _ in 0..n {
        for _ in 0..10 {
            wire.push((next() % 128) as u8 & 0x77); // both nibbles <= 7
        }
    }
    wire
}

#[test]
fn t1_score_sum_agrees_on_narrow_safe_wires() {
    let body = include_str!("fitsh_kernels_score_sum.inc");
    let inputs: Vec<Vec<u8>> =
        (0..6usize).map(|i| score_wire_safe(i, 0x9E37_79B9 + i as u64)).collect();
    agree4(body, &inputs);
}

#[test]
fn t1_score_sum_faults_together_on_narrow_overflow() {
    // a number containing decimal digit 9: the interpreter's u8 shift
    // `1 << (digit - 48)` faults 90; the closed rv64 channel faults at the
    // same site (the pre-closure channel computed a value — iron-law fix)
    let body = include_str!("fitsh_kernels_score_sum.inc");
    let mut wire = score_wire_safe(1, 0x9E37_79B9);
    // record 0's number occupies bytes [6..14)
    wire[6..14].copy_from_slice(&9u64.to_be_bytes());
    agree4(body, &[wire]);
    // nibble >= 8 in the Common-branch visual gene: `1 << hv` faults
    let mut wire = score_wire_safe(1, 0x9E37_79B9);
    let vg0 = 6 + 8; // first vg byte
    wire[vg0] = 0x88;
    agree4(body, &[wire]);
}

#[test]
fn t1_hacd200_agrees() {
    let body = include_str!("fitsh_kernels_hacd200.inc");
    // single 32-byte record (nb=0, gb=6) and multi-record shapes
    let mut single = vec![b'A'; 6];
    single.extend(12345u64.to_be_bytes());
    single.extend([0x01u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x00, 0x12, 0x34]);
    single.extend([0u8; 8]);
    let mut multi = vec![0u8, 2]; // cnt = 2 records
    for k in 0u64..2 {
        let base = 2 + k * 20;
        let _ = base;
        multi.extend([b'M'; 6]);
        multi[2..4].copy_from_slice(&[0, 2]);
    }
    // rebuild a well-formed multi-record wire: cnt = b0*256 + b1 = 2
    let mut multi: Vec<u8> = vec![0u8, 2];
    for r in 0..2 {
        let _ = r;
        multi.extend([b'M'; 6]); // name[6]
        multi.extend(777u64.to_be_bytes()); // num at nb+16..nb+20 (u32)
        multi.extend([0u8; 6]); // padding to 20 B/record
    }
    agree4(body, &[single, multi]);
}
