//! B3 task 3: source-level `u128` participates in the kernel arithmetic —
//! a u128 value lowers as a (lo, hi) u64 pair (two vstack entries, low half
//! first), 16-byte slots, the interpreter's same-width overflow faces, and
//! a shared soft-divmod routine for 128/128 division. The function-level
//! register pool stays at 8 (spec: do NOT widen it to 16).
//!
//! Faces (all four-way agreed unless marked reject):
//! * `as bytes` on a u128 renders the full 16-byte BE view (scalar_bytes).
//! * add/sub/mul at 128 bits: overflow → Arithmetic 90, exactly the
//!   interpreter's u128 checked ops (MAX+1 → 90, MAX*2 → 90, but
//!   (2^64-1)*2 = 2^65-2 fits and agrees).
//! * div/mod: the shared routine, big dividends/divisors, divide-by-zero
//!   → 90. T5: MIXED-width u128/narrower arithmetic zero-extends the
//!   narrow side (the interpreter's arithmetic_args2 widening) and agrees;
//!   MIXED COMPARISONS stay legal and numeric — the B1 invariant
//!   (lgc_ord_cmp / value_content_eq compare by value).
//! * T6: u128 BSHL/BSHR (amount ≥ 128 → 90), POW (exp > u32 → 90,
//!   overflow → 90), MAX/MIN/ABSDIFF, DIVUP/DIVEXACT (div 0 → 90,
//!   nonzero remainder → +1 / 90) and CLAMP (lo > hi → 90) all agree.
//! * `as u64` narrowing: > u64::MAX faults CastFail 71 at runtime (the
//!   interpreter's cannot_cast face), else agrees.
//! * XOP compound (`x *= 3u128`) on a 16-byte slot: the old pair stages in
//!   pool registers before the rhs evaluates (probe-consistent pool depth),
//!   the result lands back in the cell. T7: a constant rhs absorbs at u128
//!   (`x += 1`, `x /= 16`, `x /= 0` → 90) and u128 require/throw error
//!   constants that fit u64 ride the 64-bit exit channel.
//! * Kernel-subset boundaries pinned: lists mixing u128 with narrower
//!   element widths (T19 admitted uniform u128 lists), slots mixing u128
//!   with narrower flows, u128 shifts/pow, dynamic u128 require/throw
//!   error values (the 64-bit a0 exit channel).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";

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

/// rv64 must reject with the canonical subset text naming `frag`.
fn rv64_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains("outside the translatable kernel subset"),
        "reject must use the canonical subset text: {err:?} (body: {body})"
    );
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
}

/// Interpreter acceptance proof: compiles AND runs (any outcome).
fn interp_accepts(body: &str, run: bool) {
    let it = Interp::try_build(body)
        .unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    if run {
        let _ = it.run(&[7, 9]);
    }
}

fn inputs() -> Vec<Vec<u8>> {
    vec![
        vec![0, 0],
        vec![1, 2],
        vec![255, 255],
        vec![3, 200],
        vec![9, 8],
        vec![16, 4],
    ]
}

/* ---- `as bytes`: the 16-byte BE view ---- */

#[test]
fn t3_max_u128_as_bytes_is_16b_be() {
    agree4(&format!("return {MAX_U128}u128 as bytes"), &[vec![], vec![1]]);
}

#[test]
fn t3_small_u128_as_bytes_zero_fills_hi() {
    // (7 as u128) renders 15 zero bytes then 07
    agree4("return (7u128 as bytes)", &[vec![], vec![1]]);
    // a data-dependent u128 value: b0 feeds the HIGH half
    agree4(
        "var v = ((byte(data, 0) as u128) * 65536u128) + 1u128\nreturn (v as bytes)",
        &inputs(),
    );
}

/* ---- same-width overflow faces ---- */

#[test]
fn t3_max_plus_one_is_90() {
    agree4(
        &format!("var a = {MAX_U128}u128\nreturn ((a + 1u128) as bytes)"),
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_max_mul_two_is_90() {
    agree4(
        &format!("var a = {MAX_U128}u128\nreturn ((a * 2u128) as bytes)"),
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_wide_add_mul_that_fit_agree() {
    // (2^64-1)*2 = 2^65-2 fits in u128; multi-half operands on both sides
    agree4(
        "var a = ((18446744073709551615u64 as u128) * 2u128)\nreturn (a as bytes)",
        &[vec![], vec![1]],
    );
    agree4(
        // the multiplier is data-dependent (BIG1*3 as a literal would fold
        // past u128 in the shared frontend): byte0+2 ∈ {2..257} over the
        // inputs below — ×2 fits, ×3 faults 90 on both engines
        &format!(
            "var m = (byte(data, 0) as u128)\nvar b = ({BIG1}u128 * (m + 2u128))\nreturn ((b + {BIG2}u128) as bytes)"
        ),
        &[vec![0], vec![1], vec![255]],
    );
    agree4(
        &format!(
            "var a = ({BIG1}u128 - {BIG2}u128)\nreturn ((a * a) as bytes)"
        ),
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_sub_underflow_is_90() {
    agree4(
        &format!("var a = 5u128\nvar b = {BIG1}u128\nreturn ((a - b) as bytes)"),
        &[vec![], vec![1]],
    );
}

/* ---- division / modulo through the shared routine ---- */

#[test]
fn t3_div_mod_big_agree() {
    agree4(
        &format!(
            "var a = {MAX_U128}u128\nvar b = {BIG2}u128\nreturn ((a / b) as bytes)"
        ),
        &[vec![], vec![1]],
    );
    agree4(
        &format!(
            "var a = {MAX_U128}u128\nvar b = {BIG2}u128\nreturn ((a % b) as bytes)"
        ),
        &[vec![], vec![1]],
    );
    // data-dependent dividend: b0 drives the high half
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128) + (byte(data, 1) as u128)\nvar b = 4294967296u128\nreturn ((a / b) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128) + (byte(data, 1) as u128)\nvar b = 4294967296u128\nreturn ((a % b) as bytes)",
        &inputs(),
    );
    // divisor larger than the dividend (q = 0, r = dividend path)
    agree4(
        &format!(
            "var a = 12345u128\nvar b = {BIG1}u128\nreturn ((a / b) as bytes)"
        ),
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_div_by_zero_is_90() {
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a / 0u128) as bytes)",
        &[vec![1], vec![0]],
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a % 0u128) as bytes)",
        &[vec![1], vec![0]],
    );
}

#[test]
fn t3_div_exact_relationship_holds() {
    // (a / b) * b + (a % b) == a — the routine's q/r must be consistent
    agree4(
        &format!(
            "var a = {MAX_U128}u128\nvar b = {BIG2}u128\nvar q = (a / b)\nvar r = (a % b)\nreturn (((q * b) + r) as bytes)"
        ),
        &[vec![], vec![1]],
    );
}

/* ---- mixed-width faces ---- */

/// T5: the interpreter's arithmetic_args2 widens both sides to the common
/// width — a narrow operand zero-extends into the u128 pair and every
/// checked face stays the interpreter's (overflow 90, div-by-zero 90).
#[test]
fn t5_mixed_width_arithmetic_agrees() {
    // narrow rhs, both directions of the u64 const
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a + 1u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((1u64 + a) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a * 3u64) as bytes)",
        &inputs(),
    );
    // narrow LHS (staged to a writable pair before the rhs evaluates)
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((a - (byte(data, 1) as u128)) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn (((a as u128) * (byte(data, 1) as u128)) as bytes)",
        &inputs(),
    );
    // div/mod with a narrow dividend and a u128 divisor
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = ((byte(data, 1) as u128) + 1u128)\nreturn ((a / b) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = ((byte(data, 1) as u128) + 1u128)\nreturn ((a % b) as bytes)",
        &inputs(),
    );
    // bitwise with a narrow side (hi mask = 0 semantics come free from x0)
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a & 255u8) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((a | (18446744073709551616u128)) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((a ^ 7u32) as bytes)",
        &inputs(),
    );
    // the overflow face stays the interpreter's u128 face: MAX−0 fits,
    // 0−1 underflows at 128 bits even with a u8 subtrahend
    agree4("var a = 0u128\nreturn ((a - 1u8) as bytes)", &[vec![], vec![1]]);
    agree4(
        &format!("var a = {MAX_U128}u128\nreturn ((a + 0u64) as bytes)"),
        &[vec![], vec![1]],
    );
    // compound with a narrow rhs (XOP zero-extends the same way)
    agree4(
        "var a = (byte(data, 0) as u128)\na /= 16u64\na *= 3u64\na += 1u8\nreturn (a as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\na -= 1u64\nreturn (a as bytes)",
        &inputs(),
    );
}

/// T5: the folded-past-u64 shapes that were the task-3 negative pins — the
/// frontend's constant fold produces a u128-width literal and the mixed
/// add now lowers (the u128→u64 return cast faults 71 for wide values).
#[test]
fn t5_mixed_folded_const_shapes_agree() {
    agree4(
        "var a = (byte(data, 0) as u64)\nvar r = (a + (18446744073709551615u64 + 1u64))\nreturn (r as u64) as bytes",
        &[vec![0], vec![1], vec![200]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar r = (a + (4294967296u64 * 4294967296u64))\nreturn (r as u64) as bytes",
        &[vec![0], vec![1], vec![200]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar r = (a + (2u64 ** 64u64))\nreturn (r as u64) as bytes",
        &[vec![0], vec![1], vec![200]],
    );
    // the same fold on the LEFT of the pair
    agree4(
        "var a = (byte(data, 0) as u64)\nvar r = ((18446744073709551615u64 + 1u64) - a)\nreturn (r as u64) as bytes",
        &[vec![0], vec![1], vec![200]],
    );
}

/// T5 boundary: a bool-ish/widthless operand has no reproducible face (the
/// interpreter's to_uint faults 71 on bools) — still a compile reject.
#[test]
fn t5_boolish_mixed_operand_rejects() {
    let body =
        "var a = (byte(data, 0) as u128)\nreturn ((a + (1u64 == 1u64)) as bytes)";
    rv64_must_reject(body, "definite uint widths");
    interp_accepts(body, true); // interpreter faults 71 (Bool to_uint)
}

#[test]
fn t3_mixed_comparisons_stay_legal_and_agree() {
    // B1 invariant: uint ordered compare / equality is numeric over u128 —
    // a narrower operand zero-extends exactly
    agree4(
        "var a = ((byte(data, 0) as u128) * 4294967296u128)\nvar b = (byte(data, 1) as u64)\nreturn (((a > b) as u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 4294967296u128)\nvar b = (byte(data, 1) as u64)\nreturn (((a < b) as u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nvar b = (byte(data, 0) as u64)\nreturn (((a == b) as u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nvar b = (byte(data, 0) as u64)\nreturn (((a != b) as u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn (((a >= 5u128) as u64) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn (((a <= 5u128) as u64) as bytes)",
        &inputs(),
    );
}

/* ---- narrowing casts ---- */

#[test]
fn t3_narrow_over_max_is_71() {
    agree4(
        &format!(
            "var a = {MAX_U128}u128\nreturn ((a as u64) as bytes)"
        ),
        &[vec![], vec![1]],
    );
    agree4(
        "var a = 256u128\nreturn ((a as u8) as bytes)",
        &[vec![], vec![1]],
    );
    agree4(
        "var a = 65536u128\nreturn ((a as u16) as bytes)",
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_narrow_in_range_agrees() {
    agree4(
        "var a = 12345678901234567890u128\nreturn ((a as u64) as bytes)",
        &[vec![], vec![1]],
    );
    agree4(
        "var a = 255u128\nreturn ((a as u8) as bytes)",
        &[vec![], vec![1]],
    );
    // data-dependent: b0 == 0/1 stays narrow, b0 == 255 faults
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551615u128)\nreturn ((a as u64) as bytes)",
        &inputs(),
    );
}

/* ---- slots, XOP compound, bitwise ---- */

#[test]
fn t3_u128_slot_roundtrip_and_xop() {
    // a 16-byte slot: store, divide in place through the shared routine
    agree4(
        &format!(
            "var x = {MAX_U128}u128\nx = (x / 3u128)\nreturn (x as bytes)"
        ),
        &[vec![], vec![1]],
    );
}

#[test]
fn t3_xop_compound_agree() {
    agree4(
        "var x = (byte(data, 0) as u128)\nx *= 3u128\nx += 1u128\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx /= 3u128\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx -= 1u128\nreturn (x as bytes)",
        &inputs(),
    );
    // no `%=` token in the shared frontend — the mod compound is spelled
    // as an assignment (leftover note: compound % on a u128 slot rejects)
    agree4(
        "var x = (byte(data, 0) as u128)\nx = (x % 1000u128)\nreturn (x as bytes)",
        &inputs(),
    );
    // the shared frontend has no &=/|=/^= compound tokens — bitwise faces go
    // through value expressions
    agree4(
        "var x = (byte(data, 0) as u128)\nx = (x & 255u128)\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx = (x | 18446744073709551616u128)\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = (byte(data, 0) as u128)\nx = (x ^ 12345678901234567890u128)\nreturn (x as bytes)",
        &inputs(),
    );
}

#[test]
fn t3_xop_div_overflow_face_stays_90() {
    // compound mul overflow on the slot path must fault 90 like the binary
    // form (the slot's old value * 2^128 overflows for any nonzero old)
    agree4(
        "var x = (byte(data, 0) as u128)\nx *= 18446744073709551616u128\nreturn (x as bytes)",
        &inputs(),
    );
}

#[test]
fn t3_u128_in_loop_agree() {
    // u128 slots inside a while: the induction cell is 16 bytes
    agree4(
        "var s = 0u128\nvar i = 0u64\nwhile i < 20 {\n  s = (s + (i as u128) * 18446744073709551616u128 + 1u128)\n  i += 1u64\n}\nreturn (s as bytes)",
        &[vec![], vec![1]],
    );
}

/* ---- kernel-subset boundaries (compile rejects) ---- */

#[test]
fn t3_u128_list_element_mixed_width_rejects() {
    // T19 flipped the old "u128 list elements" pin: u128 elements are now
    // admitted (16B stride, positive coverage in rv64_list_shapes.rs). The
    // remaining boundary is the WIDTH MIX inside one list: the flat frame
    // block has one fixed stride, the interpreter keeps per-element widths
    let body = "var l = [1u64, 2u128]\nreturn (byte(data, 0) as bytes)";
    rv64_must_reject(body, "element width is fixed");
    interp_accepts(body, false);
    let body2 = "var l = [2u128]\nappend(l, 5u64)\nreturn (byte(data, 0) as bytes)";
    rv64_must_reject(body2, "element width is fixed");
    let body3 = "var l = []\nappend(l, 340282366920938463463374607431768211455u128)\nl[0] = 5u64\nreturn (byte(data, 0) as bytes)";
    rv64_must_reject(body3, "element width is fixed");
}

#[test]
fn t3_mixed_width_slot_rejects() {
    // a slot fed both u64 and u128 flows: the u64 flow's overflow face
    // would widen once the join picks the 16-byte cell
    let body = "var x = 1u64\nx = 340282366920938463463374607431768211455u128\nreturn (x as bytes)";
    rv64_must_reject(body, "u128");
    interp_accepts(body, true);
    // a bool flow into a u128 slot truncates the high half — same reject
    let body2 = "var x = 340282366920938463463374607431768211455u128\nx = (1u64 == 1u64)\nreturn (x as bytes)";
    rv64_must_reject(body2, "u128");
}

/// T6: shifts at u128 — checked_shl/checked_shr semantics (amount ≥ 128 →
/// 90; a legal amount truncates silently), the full [0,63]/[64,127] amount
/// range, and the flipped `u128-shift` negative-pin body.
#[test]
fn t6_shifts_agree() {
    // dynamic amounts across the 64 boundary (b1 ∈ [0..255] → hi bits)
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar s = (byte(data, 1) as u128)\nreturn ((a >> s) as bytes)",
        &[vec![0, 0], vec![1, 1], vec![255, 63], vec![3, 64], vec![9, 127]],
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar s = (byte(data, 1) as u128)\nreturn ((a << s) as bytes)",
        &[vec![0, 0], vec![1, 1], vec![255, 63], vec![3, 64], vec![9, 127], vec![4, 124]],
    );
    // constant amounts 0 / 63 / 64 / 127 and the silent truncation face
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551615u128)\nreturn ((a >> 64u128) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a << 0u128) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn (((a << 63u128) >> 62u128) as bytes)",
        &inputs(),
    );
    // amount ≥ 128 → Arithmetic 90 on both engines (rhs > u32::MAX too)
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a << 128u128) as bytes)",
        &[vec![0], vec![1]],
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nvar s = 18446744073709551616u128\nreturn ((a >> s) as bytes)",
        &[vec![0], vec![1]],
    );
    // the flipped task-3 `u128-shift` pin body (a<<70 narrows via as u64:
    // 0 stays 0, any nonzero a ≥ 2^70 faults 71 on the narrowing cast)
    agree4(
        "var a = (byte(data, 0) as u64)\nvar r = (((a as u128) << 70u128) as u64)\nreturn (r) as bytes",
        &[vec![0], vec![1], vec![255]],
    );
    // a narrow amount zero-extends (u8 shift amounts stay legal)
    agree4(
        "var a = ((byte(data, 0) as u128) * 65536u128)\nreturn ((a >> 4u8) as bytes)",
        &inputs(),
    );
}

/// T6: POW at u128 — exp 0/1, plain squares, the u32 exponent bound and
/// the u128 overflow face, all against the interpreter's checked_pow.
#[test]
fn t6_pow_agrees() {
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a ** 0u128) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a ** 1u128) as bytes)",
        &inputs(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a ** 2u128) as bytes)",
        &inputs(),
    );
    // data-dependent exponent (0..7): the square-and-multiply loop
    agree4(
        "var a = ((byte(data, 0) as u128) + 1u128)\nvar e = (byte(data, 1) as u128)\nreturn ((a ** e) as bytes)",
        &[vec![1, 0], vec![1, 1], vec![2, 5], vec![3, 7], vec![255, 3]],
    );
    // bases 0/1 with a big-but-u32 exponent stay exact (dynamic base: a
    // literal base would fold at parse time and never reach the op)
    agree4(
        "var a = ((byte(data, 0) as u128) % 2u128)\nreturn ((a ** 4294967295u128) as bytes)",
        &[vec![0], vec![1]],
    );
    // exponent > u32::MAX → 90 BEFORE any squaring (interpreter exp_u32)
    agree4(
        "var a = (byte(data, 0) as u128)\nvar e = 4294967296u128\nreturn ((a ** e) as bytes)",
        &[vec![0], vec![1], vec![2]],
    );
    // dynamic exponent beyond u32 → 90
    agree4(
        "var a = 2u128\nvar e = (18446744073709551616u128 + (byte(data, 0) as u128))\nreturn ((a ** e) as bytes)",
        &[vec![0], vec![1]],
    );
    // overflow → 90 (3^81 > 2^128; a dynamic base keeps it off the fold)
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a ** 81u128) as bytes)",
        &[vec![3], vec![2]],
    );
    // wide base × u32-width exponent that fits: 2^127
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a ** 127u128) as bytes)",
        &[vec![2], vec![1]],
    );
    // a narrow base zero-extends (staged pair)
    agree4(
        "var a = (byte(data, 0) as u64)\nvar e = (byte(data, 1) as u128)\nreturn (((a ** e) / 3u128) as bytes)",
        &[vec![5, 3], vec![2, 4], vec![255, 2]],
    );
}

/// T6: MAX/MIN select the whole widened operand; ABSDIFF at 128 bits.
#[test]
fn t6_max_min_absdiff_agree() {
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = (byte(data, 1) as u64)\nreturn ((max(a, b)) as bytes)",
        &[vec![0, 0], vec![0, 5], vec![1, 200], vec![255, 255]],
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = (byte(data, 1) as u64)\nreturn ((min(a, b)) as bytes)",
        &[vec![0, 0], vec![0, 5], vec![1, 200], vec![255, 255]],
    );
    // the operands swap sides (narrow on the left)
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = (byte(data, 1) as u64)\nreturn ((max(b, a)) as bytes)",
        &[vec![0, 0], vec![0, 5], vec![1, 200], vec![255, 255]],
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = (byte(data, 1) as u128)\nreturn ((abs_diff(a, b)) as bytes)",
        &[vec![0, 0], vec![0, 5], vec![9, 200], vec![1, 0]],
    );
    agree4(
        &format!("var a = {MAX_U128}u128\nvar b = 1u128\nreturn ((abs_diff(a, b)) as bytes)"),
        &[vec![], vec![1]],
    );
}

/// T6: DIVUP/DIVEXACT through the shared divmod routine (quotient and
/// remainder together): divide-by-zero 90, ceil on a nonzero remainder,
/// exact fault on one.
#[test]
fn t6_divup_divexact_agree() {
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = ((byte(data, 1) as u128) + 1u128)\nreturn ((div_up(a, b)) as bytes)",
        &[vec![0, 0], vec![1, 2], vec![255, 255], vec![4, 3]],
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = ((byte(data, 1) as u128) + 1u128)\nreturn ((div_exact_op(a, b)) as bytes)",
        &[vec![0, 1], vec![16, 4], vec![4, 2], vec![255, 1]],
    );
    // div by zero → 90 on both
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((div_up(a, 0u128)) as bytes)",
        &[vec![1], vec![0]],
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((div_exact_op(a, 0u128)) as bytes)",
        &[vec![1], vec![0]],
    );
    // exact quotient +1 vs fault on remainder (fixed values)
    agree4("var r = (div_up(10u128, 3u128))\nreturn (r as bytes)", &[vec![], vec![1]]);
    agree4("var r = (div_up(9u128, 3u128))\nreturn (r as bytes)", &[vec![], vec![1]]);
    agree4("var r = (div_exact_op(10u128, 3u128))\nreturn (r as bytes)", &[vec![], vec![1]]);
    agree4("var r = (div_exact_op(9u128, 3u128))\nreturn (r as bytes)", &[vec![], vec![1]]);
    // a narrow divisor zero-extends
    agree4(
        "var a = ((byte(data, 0) as u128) * 4294967296u128)\nreturn ((div_up(a, 3u64)) as bytes)",
        &[vec![0], vec![1], vec![255]],
    );
}

/// T6: CLAMP at u128 — lo > hi faults 90 BEFORE clamping (BUG-010 face),
/// else min/max chain at 128 bits; bounds may be narrower (zero-extended).
#[test]
fn t6_clamp_agrees() {
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nreturn ((clamp(a, 2u128, 18446744073709551615u128)) as bytes)",
        &[vec![0], vec![1], vec![255]],
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((clamp(a, 3u64, 200u8)) as bytes)",
        &inputs(),
    );
    // x below lo / above hi / inside — distinct pick paths
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((clamp(a, 100u128, 200u128)) as bytes)",
        &inputs(),
    );
    // lo > hi → Arithmetic 90 on both engines
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((clamp(a, 200u128, 100u128)) as bytes)",
        &[vec![0], vec![1], vec![255]],
    );
    // equal bounds are legal (lo == hi)
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((clamp(a, 7u128, 7u128)) as bytes)",
        &inputs(),
    );
}

/// T6 boundary: a u128 bound with a NARROW value would produce a u128
/// result the width model cannot represent (static CLAMP width = x's) —
/// stays a compile reject.
#[test]
fn t6_clamp_narrow_value_with_u128_bound_rejects() {
    let body = "var a = (byte(data, 0) as u64)\nreturn ((clamp(a, 0u128, 5u128)) as bytes)";
    rv64_must_reject(body, "CLAMP");
    interp_accepts(body, true);
}

/* ---- choose / value-if with u128 arms ---- */

#[test]
fn t3_choose_u128_arms_agree() {
    agree4(
        &format!(
            "var c = (byte(data, 0) % 2u64)\nvar r = choose((c == 0u64), {MAX_U128}u128, 5u128)\nreturn ((r / 2u128) as bytes)"
        ),
        &inputs(),
    );
    // value-position if (IRIFR, lazy arms) — both arms two entries
    agree4(
        &format!(
            "var c = (byte(data, 0) % 2u64)\nvar r = (if (c == 0u64) {{ {MAX_U128}u128 }} else {{ 5u128 }})\nreturn ((r / 2u128) as bytes)"
        ),
        &inputs(),
    );
}

#[test]
fn t3_mixed_arm_shape_rejects() {
    // a u128 arm and a u64 arm occupy different vstack entry counts —
    // the BUG-017 family reject
    let body = "var c = (byte(data, 0) % 2u64)\nvar r = (if (c == 0u64) { 340282366920938463463374607431768211455u128 } else { 5u64 })\nreturn (r as bytes)";
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains("choose arms must have the same result type"),
        "{err:?}"
    );
}

/* ---- the pool stays at 8 ---- */

#[test]
fn t3_deep_u128_expr_rejects_beyond_pool() {
    // four u128 values fill the 8-register pool; a fifth u128 operand must
    // hit the depth limit (spec: the function-level pool is NOT widened to
    // 16 — the depth face is the documented subset limit)
    let body = "var a = (byte(data, 0) as u128)\nvar b = (byte(data, 1) as u128)\nvar c = (byte(data, 2) as u128)\nvar d = (byte(data, 3) as u128)\nvar e = (byte(data, 4) as u128)\nreturn ((a + (b + (c + (d + e)))) as bytes)";
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains("nesting exceeds") || err.contains("register pool"),
        "u128 depth overflow must hit the pool limit: {err:?}"
    );
}

#[test]
fn t3_deep_u128_expr_within_pool_agrees() {
    // four live u128 values = 8 pool registers exactly; still lowers.
    // inputs stay in byte-range on purpose: out-of-range byte() is the
    // pinned 42-vs-71 divergence (rv64_oracle_ground g5), not a task-3 face
    agree4(
        "var a = (byte(data, 0) as u128)\nvar b = (byte(data, 1) as u128)\nvar c = (byte(data, 2) as u128)\nvar d = (byte(data, 3) as u128)\nreturn ((((a * b) + (c * d)) / 3u128) as bytes)",
        &[vec![0, 0, 0, 0], vec![1, 2, 3, 4], vec![255, 255, 1, 1], vec![9, 8, 7, 6]],
    );
}

/* ---- T7: u128 require/throw error values + XOP constant absorption ---- */

const USER_ABORT: u16 = 150;
const THROW_ABORT: u16 = 151;

/// The require/throw user VALUE is the rv64 exit code itself; the
/// interpreter raises 150/151 with the value rendered in the message
/// (`42u128` — trailing_number strips the suffix).
fn user_value_agree(body: &str, input: &[u8], want: u64, code: u16) {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    let fault = it.run(input).expect_err("body must fault");
    assert_eq!(fault.code, code, "class: {}", fault.msg);
    assert_eq!(
        c3lib::trailing_number(&fault.msg),
        Some(want),
        "user value in msg: {}",
        fault.msg
    );
    for (mode, got) in rv.run(input) {
        assert_eq!(got, Err(want), "mode {mode:?} exit code of:\n{body}");
    }
}

#[test]
fn t7_throw_const_u128_user_value() {
    // a u128-SUFFIXED constant ≤ u64::MAX rides the 64-bit exit channel
    user_value_agree("throw (42u128)", &[0; 4], 42, THROW_ABORT);
    // NOTE: value 0 is NOT assertable — the trap epilogue's a0 = 0 IS the
    // clean-halt encoding on the calc ABI (exit 0 + empty output), so
    // `throw 0` can never carry the fault class through the channel. A
    // pre-existing face of the SAME shape on the u64 path (mv A0, ev with
    // ev = 0), unchanged by T7 and left to the ABI ledger.
    user_value_agree(
        "throw (18446744073709551615u128)",
        &[0; 4],
        18446744073709551615,
        THROW_ABORT,
    );
}

#[test]
fn t7_require_const_u128_user_value() {
    // require failing: exit = the user value; passing: silent on both
    user_value_agree(
        "require (42u128), byte(data, 0) < 10\nreturn (byte(data, 0) as u64) as bytes",
        &[10],
        42,
        USER_ABORT,
    );
    agree4(
        "require (42u128), byte(data, 0) < 10\nreturn (byte(data, 0) as u64) as bytes",
        &[vec![0], vec![9]],
    );
    // a cast-produced constant (ir_literal_value unwraps the cast)
    user_value_agree(
        "require (7 as u128), byte(data, 0) == 0\nreturn (byte(data, 0) as u64) as bytes",
        &[1],
        7,
        USER_ABORT,
    );
}

#[test]
fn t7_dynamic_and_oversized_u128_error_values_reject() {
    // a dynamic u128 error value cannot ride the 64-bit exit channel
    let body = "var a = (byte(data, 0) as u128)\nthrow (a)";
    rv64_must_reject(body, "64-bit");
    interp_accepts(body, true);
    // a u128 constant beyond u64::MAX neither rides the channel
    let body2 = "throw (18446744073709551616u128)";
    rv64_must_reject(body2, "64-bit");
    // same faces on require
    let body3 = "var a = (byte(data, 0) as u128)\nrequire (a), 1u64 == 1u64\nreturn 0u64 as bytes";
    rv64_must_reject(body3, "64-bit");
}

#[test]
fn t7_xop_u128_constant_absorption_agrees() {
    // `x += 1` / `x -= 1` / `x *= k` on a u128 slot: plain literals (the
    // old path demanded `as u128`-pinned constants)
    agree4(
        "var x = (byte(data, 0) as u128)\nx += 1\nx *= 3\nx -= 2\nreturn (x as bytes)",
        &inputs(),
    );
    // pow2 and general constant divisors, and the k=0 unconditional 90
    agree4(
        "var x = ((byte(data, 0) as u128) * 65536u128 + 7u128)\nx /= 256\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = ((byte(data, 0) as u128) * 65537u128)\nx /= 1000\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = (byte(data, 0) as u128)\nx /= 1\nreturn (x as bytes)",
        &inputs(),
    );
    agree4(
        "var x = (byte(data, 0) as u128)\nx /= 0\nreturn (x as bytes)",
        &[vec![1], vec![0]],
    );
    // overflow faces stay 90 through the absorbed constants
    agree4(
        &format!("var x = {MAX_U128}u128\nx += 0\nx *= 1\nreturn (x as bytes)"),
        &[vec![], vec![1]],
    );
    agree4(
        "var x = 0u128\nx -= 1\nreturn (x as bytes)",
        &[vec![], vec![1]],
    );
    agree4(
        &format!("var x = {MAX_U128}u128\nx += 1\nreturn (x as bytes)"),
        &[vec![], vec![1]],
    );
    // a u64 slot widened by a u128 compound rhs: the XOP join grows the
    // slot's static width to 128, so the initial u64 PUT is the mixed-width
    // reject (same family as `var x = 1u64; x = <u128>` — the kernel frame
    // cannot widen a cell mid-body)
    let body = "var x = 1u64\nx += (18446744073709551615u64 + 1u64)\nreturn (x as u64) as bytes";
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains("mixes u128 and narrower values"),
        "narrow-slot u128 compound must hit the mixed-width PUT reject: {err}"
    );
}
