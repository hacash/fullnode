//! T26 · the FIN* family lowering (FIN2 / FIN3 / FIN4 / FINP3 / FINP4 /
//! FINPOW3 — the full `rt/fin.rs` catalog).
//!
//! Interpreter ground truth (execute.rs FIN* dispatch →
//! instruction.rs fin2/3/4/p3/p4/pow3_checked): every operand goes through
//! `to_uint` (a non-uint/Nil operand faults CastFail 71), zero-extends to
//! the common width, the kernel computes through u256 intermediates
//! (mul_wide / div_u256_by_u{128,129} / the rounding policies), and the
//! result casts back to that common width (an overflow faults Arithmetic
//! 90). FIN3/u64_mad keeps the ORIGINAL base width. FINP3/FINP4 return
//! Bool. FINPOW3/rpow rounds half-up after every multiplication;
//! z == 0 → 90 and y == 0 → z.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const MAX_U128_M1: &str = "340282366920938463463374607431768211454";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const BIG3: &str = "555555555555555555555555555555555555";

/// Four-way agreement on every input: interpreter value/fault == all three
/// kernel modes.
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
    for input in inputs {
        let want = it.run(input);
        for (mode, got) in rv.run(input) {
            match (&want, &got) {
                (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}] input={input:?}"),
                (Err(w), Err(code)) => {
                    assert_eq!(w.code as u64, *code, "{body} [{mode:?}] input={input:?}")
                }
                _ => panic!("{body} [{mode:?}] input={input:?}: interp={want:?} rv64={got:?}"),
            }
        }
    }
}

fn no_input() -> Vec<Vec<u8>> {
    vec![vec![]]
}

/* ---------------- FIN2: div_* ---------------- */

#[test]
fn fin2_div_roundings() {
    // 7 / 2 across every policy
    agree4("return div_exact(7u64, 2u64) as bytes", &no_input()); // 90 (rem)
    agree4("return div_floor(7u64, 2u64) as bytes", &no_input()); // 3
    agree4("return div_ceil(7u64, 2u64) as bytes", &no_input()); // 4
    agree4("return div_half_up(7u64, 2u64) as bytes", &no_input()); // 4
    agree4("return div_half_even(7u64, 2u64) as bytes", &no_input()); // 4
    // half-even ties: 5/2 → 2 (even), 7/2 → 4 (even)
    agree4("return div_half_even(5u64, 2u64) as bytes", &no_input()); // 2
    agree4("return div_half_up(5u64, 2u64) as bytes", &no_input()); // 3
    // exact division passes Exact
    agree4("return div_exact(8u64, 2u64) as bytes", &no_input()); // 4
    // divisor zero → 90 under every policy
    agree4("return div_floor(7u64, 0u64) as bytes", &no_input());
    agree4("return div_exact(0u64, 0u64) as bytes", &no_input());
    // u128 width + boundary values
    agree4(
        &format!("return div_floor({MAX_U128}u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return div_ceil({MAX_U128}u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return div_half_even({MAX_U128}u128, 3u128) as bytes"),
        &no_input(),
    );
    agree4("return div_floor(0u64, 5u64) as bytes", &no_input());
}

#[test]
fn fin2_div_data_dependent() {
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = div_half_up(a, b)\nreturn (r as u64) as bytes",
        &[vec![0, 0], vec![255, 255], vec![7, 2], vec![5, 2], vec![1, 0], vec![200, 100]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = div_half_even(a, b)\nreturn (r as u64) as bytes",
        &[vec![0, 0], vec![255, 255], vec![7, 2], vec![5, 2], vec![1, 0]],
    );
    // mixed widths: u8 / u16 → common width 16 result
    agree4(
        "var a = (byte(data, 0) as u8)\nvar b = (byte(data, 1) as u16)\nvar r = div_floor(a, b)\nreturn (r as u64) as bytes",
        &[vec![255, 2], vec![7, 2], vec![7, 0], vec![0, 5]],
    );
}

/* ---------------- FIN2: sqrt_mul_* ---------------- */

#[test]
fn fin2_sqrt_mul() {
    agree4("return sqrt_mul_floor(9u64, 9u64) as bytes", &no_input()); // 9
    agree4("return sqrt_mul_ceil(9u64, 9u64) as bytes", &no_input()); // 9
    agree4("return sqrt_mul_floor(8u64, 8u64) as bytes", &no_input()); // 64·... 8·8=64 → 8
    agree4("return sqrt_mul_floor(3u64, 5u64) as bytes", &no_input()); // 15 → 3
    agree4("return sqrt_mul_ceil(3u64, 5u64) as bytes", &no_input()); // 15 → 4
    agree4("return sqrt_mul_floor(0u64, 12345u64) as bytes", &no_input()); // 0
    agree4("return sqrt_mul_ceil(1u64, 1u64) as bytes", &no_input()); // 1
    // 256-bit products: isqrt above u128 range needs the full 256-bit path
    agree4(
        &format!("return sqrt_mul_floor({MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return sqrt_mul_ceil({MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return sqrt_mul_floor({BIG1}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return sqrt_mul_ceil({BIG1}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    // (2^64)·(2^64 - 1) + 1 is (2^128 - 2^64 + 1) — a perfect square
    // ((2^64 - 1)^2): floor == ceil
    agree4(
        "return sqrt_mul_floor(18446744073709551615u128, 18446744073709551615u128) as bytes",
        &no_input(),
    );
    agree4(
        "return sqrt_mul_ceil(18446744073709551615u128, 18446744073709551615u128) as bytes",
        &no_input(),
    );
}

/* ---------------- FIN2: quantize_* ---------------- */

#[test]
fn fin2_quantize() {
    agree4("return quantize_floor(7u64, 5u64) as bytes", &no_input()); // 5
    agree4("return quantize_ceil(7u64, 5u64) as bytes", &no_input()); // 10
    agree4("return quantize_floor(10u64, 5u64) as bytes", &no_input()); // 10 (exact)
    agree4("return quantize_ceil(10u64, 5u64) as bytes", &no_input()); // 10
    agree4("return quantize_floor(0u64, 5u64) as bytes", &no_input()); // 0
    agree4("return quantize_floor(7u64, 0u64) as bytes", &no_input()); // 90
    agree4("return quantize_ceil(7u64, 0u64) as bytes", &no_input()); // 90
    // a Ceil rounding past u128::MAX → 90 (quo+1)*step overflows
    agree4(
        &format!("return quantize_ceil({MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // exact → MAX
    agree4(
        &format!("return quantize_ceil({MAX_U128}u128, 2u128) as bytes"),
        &no_input(),
    ); // (2^127)·2 = 2^128 → 90
    agree4(
        &format!("return quantize_floor({BIG1}u128, 1000000007u128) as bytes"),
        &no_input(),
    );
}

/* ---------------- FIN2: sat_add / sat_sub ---------------- */

#[test]
fn fin2_sat_ops() {
    agree4("return sat_add(200u8, 100u8) as bytes", &no_input()); // 255
    agree4("return sat_sub(100u8, 200u8) as bytes", &no_input()); // 0
    agree4("return sat_add(100u8, 27u8) as bytes", &no_input()); // 127
    agree4("return sat_sub(200u8, 199u8) as bytes", &no_input()); // 1
    agree4(
        &format!("return sat_add({MAX_U64}u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    ); // MAX
    agree4(
        &format!("return sat_add({MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // MAX (w = 128)
    agree4(
        &format!("return sat_sub(0u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // 0
    // promotion: the saturation width is the COMMON width
    agree4("return sat_add(250u8, 1000u64) as bytes", &no_input()); // 1250 at u64
    agree4("return sat_sub(1000u64, 250u8) as bytes", &no_input()); // 750
}

/* ---------------- FIN3: mul_div_* / scaled_div_* ---------------- */

#[test]
fn fin3_mul_div_roundings() {
    agree4("return mul_div_exact(10u64, 4u64, 5u64) as bytes", &no_input()); // 8
    agree4("return mul_div_exact(10u64, 4u64, 3u64) as bytes", &no_input()); // 90
    agree4("return mul_div_floor(10u64, 4u64, 3u64) as bytes", &no_input()); // 13
    agree4("return mul_div_ceil(10u64, 4u64, 3u64) as bytes", &no_input()); // 14
    agree4("return mul_div_half_up(10u64, 4u64, 8u64) as bytes", &no_input()); // 5
    agree4("return mul_div_half_even(10u64, 4u64, 8u64) as bytes", &no_input()); // 5 (odd→6? 40/8 exact=5)
    agree4("return mul_div_half_even(41u64, 1u64, 2u64) as bytes", &no_input()); // 20.5 → 20
    agree4("return mul_div_half_up(41u64, 1u64, 2u64) as bytes", &no_input()); // 21
    agree4("return mul_div_floor(10u64, 4u64, 0u64) as bytes", &no_input()); // 90
    // quotient above u128 → 90 (N_hi >= V)
    agree4(
        &format!("return mul_div_floor({MAX_U128}u128, {MAX_U128}u128, 1u128) as bytes"),
        &no_input(),
    );
    // u128 products
    agree4(
        &format!("return mul_div_floor({BIG1}u128, {BIG2}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_div_ceil({BIG1}u128, {BIG2}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_div_half_even({BIG1}u128, {BIG2}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    // scaled_div shares the mul_div kernel
    agree4("return scaled_div_floor(10u64, 4u64, 3u64) as bytes", &no_input());
    agree4("return scaled_div_ceil(10u64, 4u64, 3u64) as bytes", &no_input());
    agree4("return scaled_div_half_even(41u64, 1u64, 2u64) as bytes", &no_input());
}

/* ---------------- FIN3: dev_scaled_* ---------------- */

#[test]
fn fin3_dev_scaled() {
    agree4("return dev_scaled_floor(110u64, 100u64, 1000u64) as bytes", &no_input()); // 100
    agree4("return dev_scaled_ceil(110u64, 100u64, 1000u64) as bytes", &no_input()); // 100
    agree4("return dev_scaled_floor(111u64, 100u64, 1000u64) as bytes", &no_input()); // 110
    agree4("return dev_scaled_ceil(111u64, 100u64, 1000u64) as bytes", &no_input()); // 111→ceil 111? 10·1000/100=100 → floor 100? no: |111-100|·1000/100 = 11000/100 = 110 → both 110
    agree4("return dev_scaled_half_even(105u64, 100u64, 1000u64) as bytes", &no_input()); // 50
    agree4("return dev_scaled_floor(90u64, 100u64, 1000u64) as bytes", &no_input()); // 100
    // reference 0 / scale 0 → 90
    agree4("return dev_scaled_floor(5u64, 0u64, 10u64) as bytes", &no_input());
    agree4("return dev_scaled_ceil(5u64, 10u64, 0u64) as bytes", &no_input());
    // the rounding divisor is the REFERENCE (big scale exercises 256-bit)
    agree4(
        &format!("return dev_scaled_floor({BIG1}u128, {BIG3}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return dev_scaled_half_even({BIG1}u128, {BIG3}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
}

/* ---------------- FIN3: scaled_add / scaled_sub ---------------- */

#[test]
fn fin3_scaled_addsub() {
    agree4("return scaled_add_floor(100u64, 3u64, 2u64) as bytes", &no_input()); // 101
    agree4("return scaled_add_ceil(100u64, 3u64, 2u64) as bytes", &no_input()); // 102
    agree4("return scaled_sub_floor(100u64, 3u64, 2u64) as bytes", &no_input()); // 99
    agree4("return scaled_sub_ceil(100u64, 3u64, 2u64) as bytes", &no_input()); // 98
    agree4("return scaled_add_floor(100u64, 1u64, 0u64) as bytes", &no_input()); // 90
    agree4("return scaled_sub_floor(1u64, 3u64, 2u64) as bytes", &no_input()); // 90 (underflow)
    agree4(
        &format!("return scaled_add_floor({BIG1}u128, {MAX_U128}u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return scaled_add_ceil({BIG1}u128, {MAX_U128}u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
}

/* ---------------- FIN3: mul_shr_* ---------------- */

#[test]
fn fin3_mul_shr() {
    agree4("return mul_shr_floor(10u64, 4u64, 3u64) as bytes", &no_input()); // 40>>3 = 5
    agree4("return mul_shr_ceil(10u64, 4u64, 3u64) as bytes", &no_input()); // 5
    agree4("return mul_shr_floor(10u64, 4u64, 2u64) as bytes", &no_input()); // 10
    agree4("return mul_shr_ceil(10u64, 4u64, 2u64) as bytes", &no_input()); // 10 (40>>2 exact)
    agree4("return mul_shr_ceil(41u64, 1u64, 2u64) as bytes", &no_input()); // 21 (dropped 1)
    agree4("return mul_shr_floor(41u64, 1u64, 2u64) as bytes", &no_input()); // 20
    // shift 0 with a product above u128 → 90
    agree4(
        &format!("return mul_shr_floor({MAX_U128}u128, 2u128, 0u128) as bytes"),
        &no_input(),
    );
    // every shift band: 63/64/65, 127/128/129/191/192/255/256/300
    for s in [63u64, 64, 65, 127, 128, 129, 191, 192, 255, 256, 300] {
        agree4(
            &format!("return mul_shr_floor({MAX_U128}u128, {MAX_U128}u128, {s}u128) as bytes"),
            &no_input(),
        );
        agree4(
            &format!("return mul_shr_ceil({MAX_U128}u128, {MAX_U128}u128, {s}u128) as bytes"),
            &no_input(),
        );
    }
    agree4("return mul_shr_floor(0u64, 0u64, 999u64) as bytes", &no_input()); // 0
    agree4("return mul_shr_ceil(1u64, 1u64, 999u64) as bytes", &no_input()); // 1
}

/* ---------------- FIN3: u64_mad (the width exception) ---------------- */

#[test]
fn fin3_u64_mad() {
    agree4("return u64_mad(100u64, 3u64, 10u64) as bytes", &no_input()); // 130
    agree4("return u64_mad(250u8, 3u64, 10u64) as bytes", &no_input()); // 280 → 90 (u8 base!)
    agree4("return u64_mad(250u8, 3u64, 1u64) as bytes", &no_input()); // 253 fits u8
    agree4("return u64_mad(5u16, 3u64, 10u64) as bytes", &no_input()); // 35 at u16
    // index·stride above u128 → 90
    agree4(
        &format!("return u64_mad(1u64, {MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    // sum above u128 → 90
    agree4(
        &format!("return u64_mad({MAX_U128}u128, 2u128, {MAX_U64}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return u64_mad({BIG3}u128, {BIG1}u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
}

/* ---------------- FIN3: mul_div_den_add / den_sub ---------------- */

#[test]
fn fin3_mul_div_den() {
    // den = z + y (the u129 divisor path when it overflows u128)
    agree4(
        "return mul_div_den_add_floor(100u64, 7u64, 3u64) as bytes",
        &no_input(),
    ); // 700/10 = 70
    agree4(
        "return mul_div_den_add_ceil(100u64, 7u64, 3u64) as bytes",
        &no_input(),
    );
    agree4(
        "return mul_div_den_sub_floor(100u64, 3u64, 13u64) as bytes",
        &no_input(),
    ); // 300/10 = 30
    agree4(
        "return mul_div_den_sub_ceil(100u64, 3u64, 13u64) as bytes",
        &no_input(),
    );
    // den_sub underflow (z < y) → 90; den zero → 90; den_add zero (0+0) → 90
    agree4("return mul_div_den_sub_floor(5u64, 10u64, 3u64) as bytes", &no_input());
    agree4("return mul_div_den_sub_floor(5u64, 0u64, 0u64) as bytes", &no_input());
    agree4("return mul_div_den_add_floor(5u64, 0u64, 0u64) as bytes", &no_input());
    // the u129 divisor: z + y overflows u128
    agree4(
        &format!("return mul_div_den_add_floor({BIG1}u128, {BIG2}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_div_den_add_ceil({BIG1}u128, {BIG2}u128, {BIG3}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!(
            "return mul_div_den_add_floor({BIG1}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"
        ),
        &no_input(),
    ); // den = 2^128·2 - ... u129 path
}

/* ---------------- FIN4: mul_add_div / mul_sub_div ---------------- */

#[test]
fn fin4_mul_addsub_div() {
    agree4("return mul_add_div_exact(10u64, 4u64, 5u64, 9u64) as bytes", &no_input()); // 45/9=5
    agree4("return mul_add_div_exact(10u64, 4u64, 5u64, 8u64) as bytes", &no_input()); // 90
    agree4("return mul_add_div_floor(10u64, 4u64, 5u64, 9u64) as bytes", &no_input());
    agree4("return mul_add_div_ceil(10u64, 4u64, 5u64, 9u64) as bytes", &no_input());
    agree4("return mul_add_div_half_up(10u64, 4u64, 4u64, 8u64) as bytes", &no_input()); // 44/8=5.5→6
    agree4("return mul_add_div_half_even(10u64, 4u64, 4u64, 8u64) as bytes", &no_input()); // →6
    agree4("return mul_add_div_half_even(10u64, 4u64, 0u64, 8u64) as bytes", &no_input()); // 40/8=5
    agree4("return mul_sub_div_floor(10u64, 4u64, 5u64, 9u64) as bytes", &no_input()); // 35/9=3
    agree4("return mul_sub_div_ceil(10u64, 4u64, 5u64, 9u64) as bytes", &no_input()); // 4
    agree4("return mul_sub_div_floor(5u64, 4u64, 100u64, 9u64) as bytes", &no_input()); // 90 (neg)
    agree4("return mul_add_div_floor(10u64, 4u64, 5u64, 0u64) as bytes", &no_input()); // 90
    // x·y + z above 256 bits → 90
    agree4(
        &format!(
            "return mul_add_div_floor({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128, 3u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return mul_sub_div_ceil({BIG1}u128, {BIG2}u128, {BIG3}u128, {MAX_U64}u128) as bytes"
        ),
        &no_input(),
    );
}

/* ---------------- FIN4: mul3_div ---------------- */

#[test]
fn fin4_mul3_div() {
    agree4("return mul3_div_exact(2u64, 3u64, 5u64, 3u64) as bytes", &no_input()); // 10
    agree4("return mul3_div_exact(2u64, 3u64, 5u64, 7u64) as bytes", &no_input()); // 90
    agree4("return mul3_div_floor(2u64, 3u64, 5u64, 7u64) as bytes", &no_input()); // 4
    agree4("return mul3_div_ceil(2u64, 3u64, 5u64, 7u64) as bytes", &no_input()); // 5
    agree4("return mul3_div_half_up(2u64, 3u64, 5u64, 4u64) as bytes", &no_input()); // 7.5→8
    agree4("return mul3_div_half_even(2u64, 3u64, 5u64, 4u64) as bytes", &no_input()); // →8
    agree4("return mul3_div_half_even(2u64, 3u64, 5u64, 2u64) as bytes", &no_input()); // 15/2=7.5→8
    agree4("return mul3_div_floor(2u64, 3u64, 5u64, 0u64) as bytes", &no_input()); // 90
    // 256-bit overflow of the triple product → 90
    agree4(
        &format!(
            "return mul3_div_floor({MAX_U128}u128, {MAX_U128}u128, 4u128, 7u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return mul3_div_floor({BIG1}u128, {BIG2}u128, {BIG3}u128, {BIG2}u128) as bytes"
        ),
        &no_input(),
    );
}

/* ---------------- FIN4: wavg2 ---------------- */

#[test]
fn fin4_wavg2() {
    agree4("return wavg2_exact(10u64, 1u64, 30u64, 1u64) as bytes", &no_input()); // 20
    agree4("return wavg2_floor(10u64, 1u64, 31u64, 1u64) as bytes", &no_input()); // 20.5→20
    agree4("return wavg2_ceil(10u64, 1u64, 31u64, 1u64) as bytes", &no_input()); // 21
    agree4("return wavg2_half_up(10u64, 1u64, 31u64, 1u64) as bytes", &no_input()); // 21
    agree4("return wavg2_half_even(10u64, 1u64, 31u64, 1u64) as bytes", &no_input()); // 21? 20.5 tie → even = 20
    // den zero → 90
    agree4("return wavg2_floor(10u64, 0u64, 30u64, 0u64) as bytes", &no_input());
    // lhs == rhs shortcut
    agree4("return wavg2_exact(7u64, 3u64, 7u64, 9u64) as bytes", &no_input()); // 7
    // u129 den (wx + wy overflows u128)
    agree4(
        &format!(
            "return wavg2_floor({BIG1}u128, {MAX_U128}u128, {BIG3}u128, {MAX_U64}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return wavg2_half_even({BIG1}u128, {MAX_U128}u128, {BIG3}u128, {MAX_U64}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return wavg2_ceil({BIG3}u128, {BIG2}u128, {BIG1}u128, {BIG2}u128) as bytes"
        ),
        &no_input(),
    );
}

/* ---------------- FIN4: lerp ---------------- */

#[test]
fn fin4_lerp() {
    agree4("return lerp_exact(10u64, 30u64, 1u64, 2u64) as bytes", &no_input()); // 20
    agree4("return lerp_floor(10u64, 31u64, 1u64, 2u64) as bytes", &no_input()); // 20.5→20
    agree4("return lerp_ceil(10u64, 31u64, 1u64, 2u64) as bytes", &no_input()); // 21
    agree4("return lerp_half_up(10u64, 31u64, 1u64, 2u64) as bytes", &no_input()); // 21
    agree4("return lerp_half_even(10u64, 31u64, 1u64, 2u64) as bytes", &no_input()); // 20 (tie even)
    agree4("return lerp_floor(10u64, 30u64, 0u64, 2u64) as bytes", &no_input()); // 10
    agree4("return lerp_floor(10u64, 30u64, 2u64, 2u64) as bytes", &no_input()); // 30
    // base 0 / weight > base → 90
    agree4("return lerp_floor(10u64, 30u64, 1u64, 0u64) as bytes", &no_input());
    agree4("return lerp_floor(10u64, 30u64, 3u64, 2u64) as bytes", &no_input());
    // 256-bit numerator (start·left + end·weight overflows u128)
    agree4(
        &format!(
            "return lerp_floor({MAX_U128}u128, {MAX_U128}u128, {BIG2}u128, {MAX_U128}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return lerp_half_even({BIG1}u128, {BIG3}u128, {BIG2}u128, {BIG3}u128) as bytes"
        ),
        &no_input(),
    );
}

/* ---------------- FINP3 / FINP4 (Bool results) ---------------- */

#[test]
fn finp3_abs_diff_lte() {
    agree4("return (abs_diff_lte(5u64, 9u64, 4u64) as u64) as bytes", &no_input()); // 1
    agree4("return (abs_diff_lte(5u64, 9u64, 3u64) as u64) as bytes", &no_input()); // 0
    agree4("return (abs_diff_lte(9u64, 5u64, 4u64) as u64) as bytes", &no_input()); // 1
    agree4("return (abs_diff_lte(7u64, 7u64, 0u64) as u64) as bytes", &no_input()); // 1
    agree4(
        &format!("return (abs_diff_lte({BIG1}u128, {BIG3}u128, {MAX_U128}u128) as u64) as bytes"),
        &no_input(),
    );
}

#[test]
fn finp4_predicates() {
    // within_bps: reference 0 / scale 0 / tolerance > scale → 90
    agree4(
        "return (within_bps(105u64, 100u64, 10u64, 1000u64) as u64) as bytes",
        &no_input(),
    ); // |5|·1000 <= 100·10 → 1
    agree4(
        "return (within_bps(120u64, 100u64, 10u64, 1000u64) as u64) as bytes",
        &no_input(),
    ); // 20000 > 1000 → 0
    agree4(
        "return (within_bps(105u64, 0u64, 10u64, 1000u64) as u64) as bytes",
        &no_input(),
    ); // 90
    agree4(
        "return (within_bps(105u64, 100u64, 10u64, 0u64) as u64) as bytes",
        &no_input(),
    ); // 90
    agree4(
        "return (within_bps(105u64, 100u64, 1001u64, 1000u64) as u64) as bytes",
        &no_input(),
    ); // 90
    agree4(
        &format!(
            "return (within_bps({BIG1}u128, {BIG3}u128, {BIG2}u128, {MAX_U128}u128) as u64) as bytes"
        ),
        &no_input(),
    );
    // cross products at 256 bits
    agree4(
        "return (cross_lte(2u64, 3u64, 3u64, 2u64) as u64) as bytes",
        &no_input(),
    ); // 6 <= 6 → 1
    agree4(
        "return (cross_lte(2u64, 4u64, 3u64, 2u64) as u64) as bytes",
        &no_input(),
    ); // 8 <= 6 → 0
    agree4(
        "return (cross_gte(2u64, 4u64, 3u64, 2u64) as u64) as bytes",
        &no_input(),
    ); // 1
    agree4(
        "return (cross_eq(2u64, 3u64, 3u64, 2u64) as u64) as bytes",
        &no_input(),
    ); // 1
    agree4(
        "return (cross_eq(2u64, 3u64, 3u64, 3u64) as u64) as bytes",
        &no_input(),
    ); // 0
    agree4(
        &format!("return (cross_lte({MAX_U128}u128, {MAX_U128}u128, {BIG1}u128, {BIG2}u128) as u64) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return (cross_gte({BIG1}u128, {BIG2}u128, {MAX_U128}u128, {MAX_U128}u128) as u64) as bytes"),
        &no_input(),
    );
}

/* ---------------- FINPOW3: rpow ---------------- */

#[test]
fn finpow3_rpow() {
    agree4("return rpow_half_up(3u64, 0u64, 10u64) as bytes", &no_input()); // 1 (3^0 → base scale? y=0 → z=10)
    agree4("return rpow_half_up(3u64, 1u64, 10u64) as bytes", &no_input()); // 3
    agree4("return rpow_half_up(3u64, 2u64, 10u64) as bytes", &no_input()); // 9
    agree4("return rpow_half_up(3u64, 3u64, 10u64) as bytes", &no_input()); // 27
    agree4("return rpow_half_up(3u64, 4u64, 10u64) as bytes", &no_input()); // 81
    agree4("return rpow_half_up(2u64, 64u64, 1000u64) as bytes", &no_input()); // 2^64
    agree4("return rpow_half_up(0u64, 5u64, 10u64) as bytes", &no_input()); // 0
    agree4("return rpow_half_up(5u64, 3u64, 0u64) as bytes", &no_input()); // 90 (z=0)
    agree4("return rpow_half_up(0u64, 0u64, 0u64) as bytes", &no_input()); // 90 (z=0 first)
    // half-up rounding after every multiplication (1.05^2 at scale 100)
    agree4("return rpow_half_up(105u64, 2u64, 100u64) as bytes", &no_input()); // 110
    agree4("return rpow_half_up(105u64, 4u64, 100u64) as bytes", &no_input());
    agree4("return rpow_half_up(995u64, 3u64, 1000u64) as bytes", &no_input());
    // u128 bases with big exponents
    agree4(
        &format!("return rpow_half_up({BIG2}u128, 17u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    agree4(
        "return rpow_half_up(2u128, 127u128, 3u128) as bytes",
        &no_input(),
    );
}

/* ---------------- cross-cutting faces ---------------- */

#[test]
fn fin_nil_operand_fails_71() {
    // a may-Nil local flowing into a fin kernel faults 71 (to_uint)
    agree4(
        "var n = nil\nvar r = mul_div_floor(5u64, n, 2u64)\nreturn (r as u64) as bytes",
        &no_input(),
    );
    agree4(
        "var n = nil\nvar r = div_floor(n, 2u64)\nreturn (r as u64) as bytes",
        &no_input(),
    );
    agree4(
        "var n = nil\nvar r = lerp_floor(1u64, 2u64, n, 5u64)\nreturn (r as u64) as bytes",
        &no_input(),
    );
    agree4(
        "var n = nil\nvar r = rpow_half_up(3u64, 2u64, n)\nreturn (r as u64) as bytes",
        &no_input(),
    );
}

#[test]
fn fin_fit_back_overflow_90() {
    // the result must fit the COMMON width (u8 here) — overflow → 90
    agree4("return div_floor(2000u64, 1u64) as bytes", &no_input()); // u64, fine
    agree4(
        "var a = (byte(data, 0) as u8)\nvar r = mul_div_floor(a, 100u64, 1u64)\nreturn (r as u64) as bytes",
        &[vec![255], vec![3], vec![0]],
    ); // common width u64 — never overflows
    agree4(
        "var a = 5u8\nvar b = 100u8\nvar r = mul_div_floor(a, b, 1u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 500 > 255 → 90 (u8 common width)
    agree4(
        "var r = scaled_add_floor(250u8, 3u8, 2u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 250 + 1 = 251 fits
    agree4(
        "var r = scaled_add_ceil(250u8, 3u8, 2u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 250 + 2 = 252 fits
    agree4(
        "var r = wavg2_floor(200u8, 1u8, 250u8, 1u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 225 fits u8
    agree4(
        "var r = wavg2_floor(240u8, 1u8, 250u8, 1u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 245 fits
    agree4(
        "var r = lerp_floor(200u8, 250u8, 1u8, 2u8)\nreturn (r as u64) as bytes",
        &no_input(),
    ); // 225 fits
}

#[test]
fn fin_data_dependent_mixed() {
    // a small random-ish sweep across several kernels at u64
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_div_half_up(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![1, 2, 3], vec![10, 10, 0]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_div_den_add_floor(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![10, 10, 0]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = wavg2_half_even(a, 3u64, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![10, 10, 0], vec![7, 9, 4]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = rpow_half_up(a, 3u64, b)\nreturn (r as u64) as bytes",
        &[vec![0, 0], vec![3, 10], vec![255, 100], vec![17, 50], vec![5, 0]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = mul_shr_ceil(12345u64, a, b)\nreturn (r as u64) as bytes",
        &[vec![7, 3], vec![255, 130], vec![1, 0], vec![9, 64], vec![3, 128], vec![2, 200]],
    );
}

#[test]
fn fin_nested_and_statement_forms() {
    // FIN inside a bigger expression and under control flow
    agree4(
        "return (1u64 + mul_div_floor(10u64, 4u64, 3u64)) as bytes",
        &no_input(),
    );
    agree4(
        "var t = 0u64\nif ((byte(data, 0) as u64) > 5u64) { t = mul_div_floor(10u64, 4u64, 3u64) } else { t = div_ceil(7u64, 2u64) }\nreturn (t as u64) as bytes",
        &[vec![0], vec![255]],
    );
    agree4(
        "var s = 0u64\nvar i = 0u64\nwhile (i < 4u64) { s = u64_mad(s, 1u64, mul_div_floor(i, 10u64, 4u64))\n i = (i + 1u64) }\nreturn (s as u64) as bytes",
        &no_input(),
    );
}

/* ------- D2/D3 (fin-control survey): rounding-policy boundary matrix -------
 *
 * HalfUp / HalfEven — now shared ROUTINES inside the lowering — plus the
 * Floor ("round-down") and Ceil siblings, across every rounded-division
 * FIN family × the boundary shapes: quotient exactly .5 (tie up / tie to
 * even / tie down by final parity), all-ones operands, all-zero operands,
 * and the round-up carry chains (q+1 still fits u128 vs q = u128::MAX →
 * the checked +1 faults 90 from INSIDE the routine). Interpreter vs all
 * three lowered modes must agree on value AND fault code. */

#[test]
fn fin_round_policy_boundary_matrix() {
    // FIN2 div_*: ties, all-ones, all-zero, divisor faces
    agree4("return div_half_up(7u64, 2u64) as bytes", &no_input()); // 3.5 → 4
    agree4("return div_half_up(5u64, 2u64) as bytes", &no_input()); // 2.5 → 3
    agree4("return div_half_even(7u64, 2u64) as bytes", &no_input()); // 3.5 → 4 (q odd → up)
    agree4("return div_half_even(5u64, 2u64) as bytes", &no_input()); // 2.5 → 2 (q even stays)
    agree4("return div_floor(7u64, 2u64) as bytes", &no_input()); // 3 (round down)
    agree4("return div_ceil(7u64, 2u64) as bytes", &no_input()); // 4
    agree4("return div_half_up(0u64, 5u64) as bytes", &no_input()); // zero numerator
    agree4("return div_half_up(0u64, 0u64) as bytes", &no_input()); // 0/0 → 90
    agree4("return div_half_even(7u64, 0u64) as bytes", &no_input()); // x/0 → 90
    // top tie: (2^128-1)/2 → q = 2^127-1, rem 1 ≥ d−d/2 → up fits
    agree4(
        &format!("return div_half_up({MAX_U128}u128, 2u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return div_half_even({MAX_U128}u128, 2u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return div_half_up({MAX_U128}u128, 1u128) as bytes"),
        &no_input(),
    ); // all-ones / 1
    agree4(
        &format!("return div_half_up({MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // 1 rem 0
    agree4(
        &format!("return div_half_up(1u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // far below half → 0

    // FIN3 mul_div_* / scaled_div_*: 256-bit numerators, ties at both
    // parities, zero faces
    agree4("return mul_div_half_up(3u64, 1u64, 2u64) as bytes", &no_input()); // 1.5 → 2
    agree4("return mul_div_half_even(3u64, 1u64, 2u64) as bytes", &no_input()); // q=1 odd → 2
    agree4("return mul_div_half_even(1u64, 1u64, 2u64) as bytes", &no_input()); // 0.5, q=0 even → 0
    agree4("return mul_div_floor(3u64, 1u64, 2u64) as bytes", &no_input()); // 1
    agree4("return scaled_div_half_up(3u64, 1u64, 2u64) as bytes", &no_input());
    agree4("return scaled_div_half_even(1u64, 1u64, 2u64) as bytes", &no_input());
    agree4("return mul_div_half_up(0u64, 9u64, 2u64) as bytes", &no_input()); // zeros
    agree4("return mul_div_half_up(3u64, 1u64, 0u64) as bytes", &no_input()); // /0 → 90
    // the round-up CARRY faces through a 256-bit numerator:
    // n = 2·MAX+1 = 2^129-1, d = 2 → q = MAX, rem 1 ≥ 1 → up → 90
    agree4(
        &format!("return mul_add_div_half_up({MAX_U128}u128, 2u128, 1u128, 2u128) as bytes"),
        &no_input(),
    ); // → 90
    agree4(
        &format!("return mul_add_div_half_even({MAX_U128}u128, 2u128, 1u128, 2u128) as bytes"),
        &no_input(),
    ); // → 90 (above half)
       // the fits sibling: n = 2^129-3 → q = MAX-1, rem 1 → up → MAX
    agree4(
        &format!("return mul_add_div_half_up({MAX_U128_M1}u128, 2u128, 1u128, 2u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_add_div_half_even({MAX_U128_M1}u128, 2u128, 1u128, 2u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_div_half_up({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // all-ones everywhere

    // FIN3 dev_scaled_half_even (rounds against the REFERENCE; 3 args:
    // x, reference, scale — diff = |x−ref|)
    agree4("return dev_scaled_half_even(4u64, 1u64, 2u64) as bytes", &no_input()); // 3·1/2 = 1.5 → 2
    agree4("return dev_scaled_half_even(2u64, 1u64, 2u64) as bytes", &no_input()); // 1·1/2 = 0.5 → 0
    agree4("return dev_scaled_half_even(3u64, 1u64, 2u64) as bytes", &no_input()); // 2·1/2 = 1 exact
    agree4("return dev_scaled_half_even(3u64, 1u64, 0u64) as bytes", &no_input()); // ref 0 → 90
    agree4("return dev_scaled_half_even(3u64, 1u64, 2u64) as bytes", &no_input());

    // FIN4 mul3_div_* / lerp_* / wavg2_*: ties, parity, zero faces
    agree4("return mul3_div_half_up(3u64, 1u64, 1u64, 2u64) as bytes", &no_input()); // 1.5 → 2
    agree4("return mul3_div_half_even(3u64, 1u64, 1u64, 2u64) as bytes", &no_input()); // → 2
    agree4("return mul3_div_half_even(1u64, 1u64, 1u64, 2u64) as bytes", &no_input()); // → 0
    agree4("return lerp_half_up(0u64, 3u64, 2u64, 1u64) as bytes", &no_input()); // 1.5 → 2
    agree4("return lerp_half_even(0u64, 3u64, 2u64, 1u64) as bytes", &no_input()); // → 2
    agree4("return lerp_half_even(0u64, 1u64, 2u64, 1u64) as bytes", &no_input()); // 0.5 → 0
    agree4("return lerp_half_up(0u64, 1u64, 0u64, 0u64) as bytes", &no_input()); // base 0 → 90
    agree4("return lerp_half_up(0u64, 1u64, 2u64, 3u64) as bytes", &no_input()); // weight > base → 90
    agree4("return wavg2_half_up(0u64, 3u64, 1u64, 1u64) as bytes", &no_input()); // 1.5 → 2
    agree4("return wavg2_half_even(0u64, 3u64, 1u64, 1u64) as bytes", &no_input()); // → 2
    agree4("return wavg2_half_up(0u64, 1u64, 1u64, 1u64) as bytes", &no_input()); // 0.5 → 1
    agree4("return wavg2_half_even(0u64, 1u64, 1u64, 1u64) as bytes", &no_input()); // final parity even → 0
    agree4("return wavg2_half_up(1u64, 1u64, 0u64, 0u64) as bytes", &no_input()); // den 0 → 90
    // u129 denominator (bit 128 set): den = 2^128 exactly
    agree4(
        &format!("return wavg2_half_up(0u64, 3u64, {MAX_U128}u128, 1u128) as bytes"),
        &no_input(),
    );

    // FINPOW3 rpow_half_up: per-multiplication half-up
    agree4("return rpow_half_up(15u64, 1u64, 1u64) as bytes", &no_input()); // 1.5 → 2
    agree4("return rpow_half_up(5u64, 1u64, 1u64) as bytes", &no_input()); // 0.5 → 1
    agree4("return rpow_half_up(25u64, 1u64, 1u64) as bytes", &no_input()); // 2.5 → 3
    agree4("return rpow_half_up(2u64, 0u64, 1u64) as bytes", &no_input()); // y = 0 → base
    agree4("return rpow_half_up(2u64, 1u64, 0u64) as bytes", &no_input()); // z = 0 → 90
    agree4(
        &format!("return rpow_half_up({MAX_U64}u64, 2u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    ); // all-ones chain

    // FINP3 / FINP4 have no rounding tail — boundary-value smoke over the
    // 256-bit cross products they lower through mul_wide
    agree4(
        &format!("return (abs_diff_lte({MAX_U128}u128, 0u128, {MAX_U128}u128)) as bytes"),
        &no_input(),
    );
    agree4("return (abs_diff_lte(0u64, 0u64, 0u64)) as bytes", &no_input());
    agree4(
        &format!("return (cross_eq({MAX_U128}u128, {MAX_U128}u128, 1u128, 1u128)) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return (cross_lte(0u128, {MAX_U128}u128, 1u128, 1u128)) as bytes"),
        &no_input(),
    );
    agree4("return (within_bps(1u64, 1u64, 0u64, 1u64)) as bytes", &no_input()); // ref 0 → 90
    agree4("return (within_bps(1u64, 1u64, 1u64, 0u64)) as bytes", &no_input()); // scale 0 → 90
    agree4(
        &format!("return (within_bps({MAX_U64}u64, 0u64, {MAX_U64}u64, {MAX_U64}u64)) as bytes"),
        &no_input(),
    );
}

/// D2/D3 site-count gate + routine single-emission. Every `ret` in the
/// stream is either a routine epilogue or the body's own `return`
/// epilogue (each body here has exactly one `return`), so the ret count
/// is 1 + the number of distinct shared routines. D1 link convention:
/// the body ret stays a plain `ret` (`jalr x0, 0(x1)` — the c.jr ra
/// halfword in the RVC modes) while every shared routine returns through
/// the mode-scratch a0 (`jr a0`: x10 in Im/Imc, x5 in ImcRemap); both
/// forms decode to `Jalr { rd: 0, imm: 0 }`, so the census counts ra +
/// the mode scratch through the consensus decoder.
///
/// * BELOW the threshold (static site count < 3 per primitive) the D2/D3
///   gates keep the historical inline forms — only the 256/128 divmod
///   routine (ungated) appears besides the body's own return.
/// * AT/ABOVE the threshold (≥3 mul_wide, ≥3 HalfUp, ≥3 HalfEven sites)
///   the shared routines activate, and each is emitted EXACTLY ONCE —
///   adding MORE sites must not move the ret count.
/// * FINPOW3 (2 mul_wide + 2 HalfUp static sites) sits below the
///   threshold: its mul_wide/round tails stay inline (the N=2 form the
///   gate deliberately keeps at baseline size).
#[test]
fn fin_shared_routines_emitted_once() {
    use vm::calc::container::CALC_PROFILE_RV64IMC;
    use vm::calc::decode::{decode_at, Inst};
    use vm::ir::rv64::Rv64Mode;
    let count_rets = |code: &[u8], mode: &Rv64Mode| -> usize {
        // body ret link + routine link (D1 mode-scratch a0: x10/x10/x5)
        let scratch = match mode {
            Rv64Mode::ImcRemap => 5u8,
            _ => 10u8,
        };
        let profile = if *mode == Rv64Mode::Im { 0u8 } else { CALC_PROFILE_RV64IMC };
        let mut n = 0;
        let mut off = 0usize;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, profile)
                .unwrap_or_else(|e| panic!("decode @{off}: {e:?}"));
            if matches!(&inst, Inst::Jalr { rd: 0, imm: 0, rs1 }
                if *rs1 == 1 || *rs1 == scratch)
            {
                n += 1;
            }
            off += len as usize;
        }
        assert_eq!(off, code.len(), "stream must end on an instruction boundary");
        n
    };
    let base = "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\n";
    // below the gate: 2 HalfUp + 1 HalfEven round sites, 1 mul_wide site
    let small = format!(
        "{base}var r1 = div_half_up(a, b)\nvar r2 = mul_div_half_up(a, b, 100u64)\n\
         var r3 = div_half_even(a, b)\nreturn ((r1 + r2 + r3) as u64) as bytes"
    );
    // at/above the gate: 3 HalfUp + 3 HalfEven + 3 mul_wide sites
    let big = format!(
        "{base}var r1 = div_half_up(a, b)\nvar r2 = div_half_up(b, a)\n\
         var r3 = mul_div_half_up(a, b, 100u64)\nvar r4 = div_half_even(a, b)\n\
         var r5 = div_half_even(b, a)\nvar r6 = mul_div_half_even(a, b, 100u64)\n\
         var r7 = u64_mad(a, b, 3u64)\n\
         return ((((r1 + r2 + r3 + r4) + r5) + r6 + r7) as u64) as bytes"
    );
    // one MORE site of each primitive — the ret counts must not move
    let bigger = format!(
        "{base}var r1 = div_half_up(a, b)\nvar r2 = div_half_up(b, a)\n\
         var r3 = mul_div_half_up(a, b, 100u64)\nvar r4 = div_half_even(a, b)\n\
         var r5 = div_half_even(b, a)\nvar r6 = mul_div_half_even(a, b, 100u64)\n\
         var r7 = u64_mad(a, b, 3u64)\nvar r8 = div_half_up(a, a)\n\
         var r9 = div_half_even(a, a)\nvar r10 = mul_div_half_up(b, a, 50u64)\n\
         return ((((((r1 + r2 + r3 + r4) + r5) + r6 + r7) + r8 + r9) + r10) as u64) as bytes"
    );
    let rvs = Rv64::try_build(&small).unwrap_or_else(|e| panic!("small: {e}"));
    let rvb = Rv64::try_build(&big).unwrap_or_else(|e| panic!("big: {e}"));
    let rvbb = Rv64::try_build(&bigger).unwrap_or_else(|e| panic!("bigger: {e}"));
    for i in 0..rvs.codes.len() {
        let (mode, cs) = &rvs.codes[i];
        let ns = count_rets(cs, mode);
        let nb = count_rets(&rvb.codes[i].1, mode);
        let nbb = count_rets(&rvbb.codes[i].1, mode);
        assert_eq!(
            ns, 2,
            "{mode:?}: below the gate only body return + the divmod routine (rets = {ns})"
        );
        assert_eq!(
            nb, 5,
            "{mode:?}: body return + divmod + mul_wide + round_halfup + round_halfeven (rets = {nb})"
        );
        assert_eq!(
            nbb, 5,
            "{mode:?}: extra sites must add ZERO routine bodies (rets = {nbb})"
        );
    }
    // FINPOW3: 2 mul_wide + 2 HalfUp static sites — below the gate, the
    // loop's tails stay inline (divmod only)
    let rv2 = Rv64::try_build("return rpow_half_up(3u64, 3u64, 1u64) as bytes")
        .unwrap_or_else(|e| panic!("rpow: {e}"));
    for (mode, code) in &rv2.codes {
        let n = count_rets(code, mode);
        assert_eq!(
            n, 2,
            "{mode:?}: FINPOW3 sits below the gate — inline mul_wide/round, divmod only (rets = {n})"
        );
    }
}
