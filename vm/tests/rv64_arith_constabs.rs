//! A7 / A4 / A2 / A9 (docs.loc/fitsh-rv64/survey/arith.md): the arithmetic
//! constant-absorption candidates' end-to-end differentials.
//!
//! Every positive case is a four-way agreement: interpreter result and
//! failure code == all three rv64 modes (Im / Imc / ImcRemap).
//!
//! * A7 — u64 arith_k micro-extensions: DIVUP pow2 constants (the
//!   srli+andi+beq+addi form, remainder classes, masks above the andi
//!   window), DIVUP/DIVEXACT non-pow2 constants (the dead zero guard
//!   drops — a constant divisor is nonzero by proof), POW e=2 in-place
//!   square, at every static width.
//! * A4 — u128 BSHL/BSHR constant shifts: k=0 identity, 0<k<64 direct
//!   shift pairs, k in [64,127], k >= 128 → the same Arithmetic 90 face
//!   the runtime amount check raises, dynamic amounts keep the variable
//!   path.
//! * A2 — u128 expression constant absorption: DIV/MOD pow2 and k=1,
//!   DIVUP/DIVEXACT k=1/pow2, divisors ≥ 2^64, zero-divisor 90, non-pow2
//!   constants stay on the shared routine, XOP compound equivalence.
//! * A9 — the shared u128 multiply routine (site-count gated): multi-site
//!   bodies agree, POW squaring keeps its d==r aliasing, single-site
//!   bodies ride the untouched inline form.

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

/// Every single-byte input: covers whole remainder classes in one sweep.
fn inputs_all_bytes() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8]).collect()
}

/// Two independent bytes (hi, lo pairs for 16-bit spreads).
fn inputs_2bytes() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for a in 0u16..=255 {
        for b in [0u16, 1, 7, 8, 9, 63, 64, 127, 128, 200, 255] {
            v.push(vec![a as u8, b as u8]);
        }
    }
    v
}

/* ================================================================== */
/* A7 — u64 arith_k micro-extensions                                   */
/* ================================================================== */

#[test]
fn a7_divup_pow2_remainder_classes() {
    // x = 0..255 with k = 8: every remainder class (r = 0..7) and the
    // exact-division boundary — the 4-slot srli+andi+beq+addi form
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((div_up(a, 8u64)) as bytes)",
        &inputs_all_bytes(),
    );
    // k = 2: the smallest pow2 beyond the k == 1 identity
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((div_up(a, 2u64)) as bytes)",
        &inputs_all_bytes(),
    );
    // k = 2^63: the largest u64 pow2 (srli by 63)
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_up(a, 9223372036854775808u64)) as bytes)",
        &inputs_2bytes(),
    );
}

#[test]
fn a7_divup_pow2_mask_above_andi_window() {
    // k = 4096: the mask k−1 = 4095 does NOT fit andi's 12-bit signed
    // window — the li+and three-way form must mask exactly (the
    // historical `as i32` truncation bug shape, g10_fullnode_mod_pow2)
    agree4(
        "var a = (((byte(data, 0) as u64) * 4096u64) + (byte(data, 1) as u64))\nreturn ((div_up(a, 4096u64)) as bytes)",
        &inputs_2bytes(),
    );
    // k = 2^32: mask 2^32 − 1 (would truncate to −1 through i32)
    agree4(
        "var a = (((byte(data, 0) as u64) * 16777216u64) + (byte(data, 1) as u64))\nreturn ((div_up(a, 4294967296u64)) as bytes)",
        &inputs_2bytes(),
    );
}

#[test]
fn a7_divup_nonpow2_constant_drops_zero_guard() {
    // non-pow2 constants take the new arith_k divu pair (the `beq rhs,
    // ZERO` guard is dead for a constant — but a zero CONSTANT must still
    // fault 90, so k = 0 stays on the general path)
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_up(a, 3u64)) as bytes)",
        &inputs_2bytes(),
    );
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_up(a, 1000000007u64)) as bytes)",
        &inputs_2bytes(),
    );
    // k = 0 keeps the zero-divisor 90 face (the guard must NOT drop here)
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((div_up(a, 0u64)) as bytes)",
        &inputs_all_bytes(),
    );
}

#[test]
fn a7_divexact_nonpow2_and_pow2() {
    // div_exact with a non-pow2 constant: the remainder check stays, the
    // zero guard drops (inexact → 90 on both engines for most inputs)
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_exact_op(a, 3u64)) as bytes)",
        &inputs_2bytes(),
    );
    // pow2 constants keep the existing exact form
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_exact_op(a, 16u64)) as bytes)",
        &inputs_2bytes(),
    );
    // k = 1: identity, no face
    agree4(
        "var a = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nreturn ((div_exact_op(a, 1u64)) as bytes)",
        &inputs_2bytes(),
    );
    // k = 0 → 90 (zero divisor)
    agree4(
        "var a = (byte(data, 0) as u64)\nreturn ((div_exact_op(a, 0u64)) as bytes)",
        &inputs_all_bytes(),
    );
}

#[test]
fn a7_pow_e2_inplace_all_widths() {
    // e = 2 at every static width: the in-place square (the `mv a0, d`
    // drops) with the same mulhu 90 face and the narrow range check
    for (ty, lit) in [("u8", "2u8"), ("u16", "2u16"), ("u32", "2u32"), ("u64", "2u64")] {
        agree4(
            &format!(
                "var a = (byte(data, 0) as {ty})\nreturn (((a ** {lit})) as bytes)"
            ),
            &inputs_all_bytes(),
        );
    }
    // u64 boundary: (2^32 − 1)^2 = 2^64 − 2^33 + 1 fits; 2^32^2 = 2^64
    // overflows → 90 — the data-dependent face rides the same form
    agree4(
        "var a = (((byte(data, 0) as u64) * 16777216u64) + (byte(data, 1) as u64))\nreturn (((a ** 2u64)) as bytes)",
        &inputs_2bytes(),
    );
}

#[test]
fn a7_pow_other_exponents_regression() {
    // e = 0 / 1 / 3 / 64 keep the historical forms byte-for-byte
    for e in ["0u64", "1u64", "3u64", "64u64"] {
        agree4(
            &format!(
                "var a = (((byte(data, 0) as u64) + 1u64))\nreturn (((a ** {e})) as bytes)"
            ),
            &inputs_all_bytes(),
        );
    }
    // narrow-width exponents too (the range check after the chain)
    agree4(
        "var a = (byte(data, 0) as u8)\nreturn (((a ** 3u8)) as bytes)",
        &inputs_all_bytes(),
    );
}

#[test]
fn a7_xop_divup_shapes_via_expression_regression() {
    // the u64 XOP path rides arith_k_handles too — `x /= 8` and a
    // non-pow2 `x /= 3` must agree (DIV has had constant arms; the new
    // DIVUP arms only widen the handled set, XOP has no DivUp op)
    agree4(
        "var x = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nx /= 8u64\nreturn (x as bytes)",
        &inputs_2bytes(),
    );
    agree4(
        "var x = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nx /= 3u64\nreturn (x as bytes)",
        &inputs_2bytes(),
    );
    agree4(
        "var x = (((byte(data, 0) as u64) * 65536u64) + (byte(data, 1) as u64))\nx /= 0u64\nreturn (x as bytes)",
        &inputs_2bytes(),
    );
}

/* ================================================================== */
/* A4 — u128 BSHL/BSHR constant shift specialization                   */
/* ================================================================== */

/// `hi·2^64 + lo` from two input bytes — a full-width u128 pair.
fn u128_from_bytes(body: &str) -> String {
    format!(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = ((a + (byte(data, 1) as u128)) as u128)\n{body}"
    )
}

#[test]
fn a4_u128_shift_constants_below_64() {
    // 0 < k < 64: the direct shift pair; hi moves across the seam
    for k in ["1u128", "8u128", "63u128"] {
        agree4(
            &u128_from_bytes(&format!("return ((b << {k}) as bytes)")),
            &inputs_2bytes(),
        );
        agree4(
            &u128_from_bytes(&format!("return ((b >> {k}) as bytes)")),
            &inputs_2bytes(),
        );
    }
}

#[test]
fn a4_u128_shift_constants_cross_seam() {
    // k in [64, 127]: the two-instruction form
    for k in ["64u128", "65u128", "100u128", "127u128"] {
        agree4(
            &u128_from_bytes(&format!("return ((b << {k}) as bytes)")),
            &inputs_2bytes(),
        );
        agree4(
            &u128_from_bytes(&format!("return ((b >> {k}) as bytes)")),
            &inputs_2bytes(),
        );
    }
}

#[test]
fn a4_u128_shift_constant_zero_is_identity() {
    agree4(
        &u128_from_bytes("return ((b << 0u128) as bytes)"),
        &inputs_2bytes(),
    );
    agree4(
        &u128_from_bytes("return ((b >> 0u128) as bytes)"),
        &inputs_2bytes(),
    );
}

#[test]
fn a4_u128_shift_constant_at_or_above_128_is_90() {
    // the amount check is a pure face: k >= 128 faults 90 exactly like
    // the runtime guards (bne shi / slo >= 128) would
    for k in ["128u128", "129u128", "255u128", "18446744073709551616u128"] {
        agree4(
            &u128_from_bytes(&format!("return ((b << {k}) as bytes)")),
            &inputs_2bytes(),
        );
        agree4(
            &u128_from_bytes(&format!("return ((b >> {k}) as bytes)")),
            &inputs_2bytes(),
        );
    }
}

#[test]
fn a4_u128_shift_dynamic_regression() {
    // variable amounts keep the 17-slot checked path
    agree4(
        &u128_from_bytes("var s = (byte(data, 1) as u128)\nreturn ((b << s) as bytes)"),
        &inputs_2bytes(),
    );
    agree4(
        &u128_from_bytes("var s = ((byte(data, 1) as u128) + 120u128)\nreturn ((b >> s) as bytes)"),
        &inputs_2bytes(),
    );
    // dynamic ≥ 128 → 90 (the runtime check survives)
    agree4(
        &u128_from_bytes("var s = ((byte(data, 1) as u128) + 128u128)\nreturn ((b << s) as bytes)"),
        &inputs_2bytes(),
    );
}

#[test]
fn a4_u128_shift_narrow_lhs_and_mixed_literal() {
    // a narrow lhs zero-extends (hi = ZERO) — the constant forms must
    // read the zero hi exactly like the variable path
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a << 8u128) as bytes)",
        &inputs_all_bytes(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a >> 8u64) as bytes)",
        &inputs_all_bytes(),
    );
    agree4(
        "var a = (byte(data, 0) as u128)\nreturn ((a << 64u128) as bytes)",
        &inputs_all_bytes(),
    );
}

#[test]
fn a4_u128_shift_in_expression_chain() {
    // the specialized shift composes: results feed further u128 ops
    agree4(
        &u128_from_bytes(
            "var c = ((b << 8u128) as u128)\nvar d = ((c >> 4u128) as u128)\nreturn (((d + 1u128)) as bytes)",
        ),
        &inputs_2bytes(),
    );
}

/* ================================================================== */
/* A2 — u128 expression constant absorption (DIV/MOD/DIVUP/DIVEXACT)   */
/* ================================================================== */

const TWO64: &str = "18446744073709551616";
const TWO96: &str = "79228162514264337593543950336";
const TWO120: &str = "1329227995784915872903807060280344576";
const TWO127: &str = "170141183460469231731687303715884105728";

/// b0·2^64 + b1·2^32 + b2 — a three-byte-spread u128 (high half set,
/// mid bits, low bits: every shift/mask seam class).
fn u128_3bytes(body: &str) -> String {
    format!(
        "var a = (((byte(data, 0) as u128) * {TWO64}u128) + ((byte(data, 1) as u128) * 4294967296u128))\nvar b = ((a + (byte(data, 2) as u128)) as u128)\n{body}"
    )
}

fn inputs_3bytes() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for &a in [0u16, 1, 3, 7, 8, 200, 255].iter() {
        for &b in [0u16, 1, 15, 16, 255].iter() {
            for &c in [0u16, 1, 8, 255].iter() {
                v.push(vec![a as u8, b as u8, c as u8]);
            }
        }
    }
    v
}

#[test]
fn a2_div_mod_pow2_low() {
    // j < 64: the shift pair / the andi mask
    for k in ["2u128", "4u128", "16u128", "1024u128", "9223372036854775808u128"] {
        agree4(
            &u128_3bytes(&format!("return ((b / {k}) as bytes)")),
            &inputs_3bytes(),
        );
        agree4(
            &u128_3bytes(&format!("return ((b % {k}) as bytes)")),
            &inputs_3bytes(),
        );
    }
}

#[test]
fn a2_div_mod_pow2_high() {
    // j >= 64: the divisor itself leaves u64 — the "另形" forms
    // (DIV: lo = hi >> (j−64); MOD j = 64: hi clear; MOD j > 64: the hi
    // mask rides andi (j = 65) or li+and (2^100, 2^127))
    let hi = format!(
        "var c = ((byte(data, 0) as u128) * {TWO96}u128)\n{{body}}"
    );
    for k in [
        "18446744073709551616u128",                        // 2^64
        "36893488147419103232u128",                        // 2^65 (andi mask 1)
        "1267650600228229401496703205376u128",             // 2^100 (li mask)
        "170141183460469231731687303715884105728u128",     // 2^127 (li mask)
    ] {
        agree4(
            &hi.replace("{body}", &format!("return ((c / {k}) as bytes)")),
            &inputs_all_bytes(),
        );
        agree4(
            &hi.replace("{body}", &format!("return ((c % {k}) as bytes)")),
            &inputs_all_bytes(),
        );
    }
    // the full 3-byte value against the high divisors (low bits make the
    // MOD masks and the DIV truncation visible)
    for k in [
        "18446744073709551616u128",
        "1267650600228229401496703205376u128",
        "170141183460469231731687303715884105728u128",
    ] {
        agree4(
            &u128_3bytes(&format!("return ((b / {k}) as bytes)")),
            &inputs_3bytes(),
        );
        agree4(
            &u128_3bytes(&format!("return ((b % {k}) as bytes)")),
            &inputs_3bytes(),
        );
    }
}

#[test]
fn a2_k1_identities() {
    for body in [
        "return ((b / 1u128) as bytes)",
        "return ((b % 1u128) as bytes)",
        "return ((div_up(b, 1u128)) as bytes)",
        "return ((div_exact_op(b, 1u128)) as bytes)",
    ] {
        agree4(&u128_3bytes(body), &inputs_3bytes());
    }
}

#[test]
fn a2_zero_divisor_is_90() {
    // the zero face keeps its code and its order (71 before 90) —
    // intercepted as one jal to the arith fault
    for body in [
        "return ((b / 0u128) as bytes)",
        "return ((b % 0u128) as bytes)",
        "return ((div_up(b, 0u128)) as bytes)",
        "return ((div_exact_op(b, 0u128)) as bytes)",
    ] {
        agree4(&u128_3bytes(body), &inputs_3bytes());
    }
}

#[test]
fn a2_divup_pow2_forms() {
    for k in [
        "8u128",
        "4096u128",
        "9223372036854775808u128",
        "18446744073709551616u128",
        "1267650600228229401496703205376u128",
        "170141183460469231731687303715884105728u128",
    ] {
        agree4(
            &u128_3bytes(&format!("return ((div_up(b, {k})) as bytes)")),
            &inputs_3bytes(),
        );
    }
    // the lo-wrap carry: x = 4·2^64 − 3 → q = x >> 2 = 2^64 − 1 with
    // r = 1 → the +1 wraps out_lo to 0 and carries into out_hi
    agree4(
        "var c = ((((byte(data, 0) as u128) + 1u128) * 4u128) * 18446744073709551616u128)\nvar d = ((c - 3u128) as u128)\nreturn ((div_up(d, 4u128)) as bytes)",
        &inputs_all_bytes(),
    );
    // exactness on the high forms
    agree4(
        "var c = ((byte(data, 0) as u128) * 79228162514264337593543950336u128)\nreturn ((div_exact_op(c, 18446744073709551616u128)) as bytes)",
        &inputs_all_bytes(),
    );
    agree4(
        &u128_3bytes("return ((div_exact_op(b, 18446744073709551616u128)) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("return ((div_exact_op(b, 8u128)) as bytes)"),
        &inputs_3bytes(),
    );
}

#[test]
fn a2_nonpow2_keeps_routine() {
    // non-pow2 constants are NOT absorbed — the shared routine call
    // stays (regression: same faces, same values)
    for k in ["3u128", "1000u128", "1000000007u128"] {
        agree4(
            &u128_3bytes(&format!("return ((b / {k}) as bytes)")),
            &inputs_3bytes(),
        );
        agree4(
            &u128_3bytes(&format!("return ((b % {k}) as bytes)")),
            &inputs_3bytes(),
        );
    }
}

#[test]
fn a2_xop_compound_equivalence() {
    // the XOP arm was already folded — the expression path must agree
    // with it AND with the interpreter, in one body
    agree4(
        &u128_3bytes("b /= 16u128\nreturn ((b) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("b = (b / 16u128)\nreturn ((b) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("b = (b % 1024u128)\nreturn ((b) as bytes)"),
        &inputs_3bytes(),
    );
    // zero-divisor XOP keeps its 90
    agree4(
        &u128_3bytes("b /= 0u128\nreturn ((b) as bytes)"),
        &inputs_3bytes(),
    );
}

#[test]
fn a2_composite_chains() {
    // absorbed sites compose with shifts and each other
    agree4(
        &u128_3bytes("var c = (((b / 16u128) % 8u128) as u128)\nreturn ((c) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("var c = (((b % 1024u128) / 8u128) as u128)\nreturn ((c) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("var c = (((b / 8u128) << 2u128) as u128)\nreturn ((c) as bytes)"),
        &inputs_3bytes(),
    );
    agree4(
        &u128_3bytes("var c = ((div_up(b, 8u128) * 8u128) as u128)\nreturn ((c) as bytes)"),
        &inputs_3bytes(),
    );
}

/* ================================================================== */
/* A9 — the shared u128 multiply routine (site-count gated)            */
/* ================================================================== */

/// A mul-chain body with `n` data-dependent u128 multiply sites plus the
/// widening mul of the builder itself — `n` counts the CHAIN sites.
fn u128_mul_chain(n: usize) -> String {
    let mut b = String::from(
        "var a = (((byte(data, 0) as u128) * 18446744073709551616u128) + ((byte(data, 1) as u128) * 4294967296u128))\nvar m = (((byte(data, 2) as u128) + 1u128))\n",
    );
    b.push_str("var c0 = ((a * m) as u128)\n");
    for i in 1..n {
        b.push_str(&format!("var c{i} = ((c{} * m) as u128)\n", i - 1));
    }
    b.push_str(&format!("return ((c{}) as bytes)", n - 1));
    b
}

#[test]
fn a9_mul_chain_sites_agree() {
    // 1 site (builder mul only): gate OFF — the inline form, untouched
    agree4(&u128_mul_chain(1), &inputs_3bytes());
    // 2 chain sites + builder mul = 3 total: gate ON — the shared routine
    agree4(&u128_mul_chain(2), &inputs_3bytes());
    // 4 chain sites + builder mul = 5 total: deeper amortization
    agree4(&u128_mul_chain(4), &inputs_3bytes());
    // 2 total: gate OFF (the threshold is 3 — a 2-site body must not
    // activate the routine, keeping the baseline byte-for-byte)
    let two = "var a = (((byte(data, 0) as u128) * 18446744073709551616u128) + ((byte(data, 1) as u128) * 4294967296u128))\nvar c0 = ((a * m) as u128)\nreturn ((c0) as bytes)".replace("var c0 = ((a * m)", "var c0 = ((a + a)");
    agree4(&two, &inputs_3bytes());
}

#[test]
fn a9_mul_chain_overflow_face() {
    // the 90 face through the routine: (b0+1)(b1+1)·2^112 reaches 2^128
    // exactly at b0 = b1 = 255
    let body = "var a = ((((byte(data, 0) as u128) + 1u128)) * 72057594037927936u128)\nvar m = ((((byte(data, 1) as u128) + 1u128)) * 72057594037927936u128)\nvar c = ((a * m) as u128)\nvar d = ((c * 2u128) as u128)\nvar e = ((d * 2u128) as u128)\nreturn ((e) as bytes)";
    agree4(body, &inputs_2bytes());
}

#[test]
fn a9_pow_aliasing_sites() {
    // a u128 POW carries TWO mul_keep sites (d == b squaring aliasing) —
    // with one expression mul the body reaches the threshold and the POW
    // loop rides the shared routine with its aliasing contract intact
    let body = "var p = ((((byte(data, 0) as u128) + 2u128)) )\nvar m = ((((byte(data, 1) as u128) + 1u128)) )\nvar c = ((p * m) as u128)\nvar q = ((p ** 3u128) as u128)\nreturn (((c + q)) as bytes)";
    agree4(body, &inputs_2bytes());
    // the 90 face through the POW loop's mul_keep sites (p ≥ 2 → p^64
    // leaves u128 for most inputs)
    let body90 = "var p = ((((byte(data, 0) as u128) + 1u128)) )\nvar m = ((((byte(data, 1) as u128) + 1u128)) )\nvar c = ((p * m) as u128)\nvar q = ((p ** 64u128) as u128)\nreturn (((c + q)) as bytes)";
    agree4(body90, &inputs_2bytes());
    // POW alone (2 sites): gate OFF — the historical inline loop
    agree4(
        "var p = ((((byte(data, 0) as u128) + 2u128)) )\nreturn (((p ** 3u128)) as bytes)",
        &inputs_all_bytes(),
    );
}

#[test]
fn a9_xop_mul_and_list_mul_sites() {
    // XOP `*=` on a u128 slot (k >= 2 constant and variable rhs) is a
    // mul site each — three of them activate the routine
    let body = "var x = ((((byte(data, 0) as u128) + 1u128)) )\nx *= 3u128\nx *= (byte(data, 1) as u128) + 1u128\nvar y = ((x * 3u128) as u128)\nreturn ((y) as bytes)";
    agree4(body, &inputs_2bytes());
}
