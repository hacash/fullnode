//! T3 · MULADD / MULSUB lowering (`mul_add` / `mul_sub`, every width to 128).
//!
//! Interpreter ground truth (execute.rs MULADD/MULSUB → triop_arithmetic →
//! arithmetic_args3 + instruction.rs muladd_checked / mulsub_checked via
//! mul_xy_addsub_z_fit_u128): the three `to_uint` operands fault CastFail 71
//! on a non-uint (Nil) value in x, y, z order, then zero-extend to the
//! three-way max width; the FULL 256-bit intermediate is observable — n =
//! x·y, n ±= z at 256 bits (the sub's underflow → Arithmetic 90), the
//! result must fit u128 (bits 128..255 clear, else 90) and casts back to
//! the common width (overflow → 90). The kernel lowers every width through
//! the u256 library — the wide intermediate is interpreter behavior, so
//! `var t = x * y; t + z` is NOT an equivalent split (it faults at the
//! narrower intermediate widths the interpreter accepts).

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const TWO_P64: &str = "18446744073709551616";

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

/* ---- narrow / u64 widths ---- */

#[test]
fn muladdsub_narrow_agree() {
    agree4("return mul_add(100u8, 2u8, 50u8) as bytes", &no_input()); // 250
    agree4("return mul_sub(100u8, 2u8, 50u8) as bytes", &no_input()); // 150
    agree4("return mul_add(65535u16, 65535u16, 1u32) as bytes", &no_input()); // u32 width
    agree4("return mul_sub(4000000000u32, 2u32, 1u32) as bytes", &no_input());
    // the result must fit the COMMON width: 200·2+100 = 500 > u8 → 90
    agree4("return mul_add(200u8, 2u8, 100u8) as bytes", &no_input());
    agree4("return mul_sub(200u8, 2u8, 255u8) as bytes", &no_input());
    // ...but the u16 z widens the width, so the same magnitude passes
    agree4("return mul_add(200u8, 2u8, 100u16) as bytes", &no_input());
}

#[test]
fn muladdsub_u64_agree() {
    // 3e9 · 3 + 5e9 = 14e9 — fits u64
    agree4(
        "return mul_add(3000000000u64, 3u64, 5000000000u64) as bytes",
        &no_input(),
    );
    agree4(
        "return mul_sub(3000000000u64, 3u64, 1000000000u64) as bytes",
        &no_input(),
    );
    // MAX·MAX needs the full 128-bit product; +0/+1 then overflows the u64
    // common width → 90 (the plain `x * y` would fault here too, but through
    // a DIFFERENT face — the interpreter's wide intermediate is the point)
    agree4(
        &format!("return mul_add({MAX_U64}u64, {MAX_U64}u64, 0u64) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_sub({MAX_U64}u64, {MAX_U64}u64, 1u64) as bytes"),
        &no_input(),
    ); // MAX²-1 > u64 → 90
    // (2^32-1)² + MAX fits u128 but not u64 → 90
    agree4(
        &format!("return mul_add(4294967295u64, 4294967295u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    );
    // product fits u64 exactly: 2^32 · 2^32 = 2^64 → wait, overflows u64;
    // (2^32-1)·(2^32+1)+0 = 2^64 > u64 → 90 | exact boundary via u128 below
    agree4(
        "return mul_sub(4294967296u64, 4294967295u64, 1u64) as bytes",
        &no_input(),
    ); // 2^64-2^32-1 > u64 → 90
}

/* ---- u128 width ---- */

#[test]
fn muladdsub_u128_agree() {
    agree4(
        &format!("return mul_add({BIG1}u128, 1u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_sub({BIG1}u128, 1u128, {BIG2}u128) as bytes"),
        &no_input(),
    );
    // 2^64 · (2^64 - 1) = 2^128 - 2^64 — the largest product still fitting
    // u128; ±small stays inside
    agree4(
        &format!("return mul_add({TWO_P64}u128, {MAX_U64}u128, {MAX_U64}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_sub({TWO_P64}u128, {MAX_U64}u128, 1u128) as bytes"),
        &no_input(),
    );
    // 2^64 · 2^64 = 2^128 exactly — one past the u128 fit → 90 (both forms)
    agree4(
        &format!("return mul_add({TWO_P64}u128, {TWO_P64}u128, 0u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_sub({TWO_P64}u128, {TWO_P64}u128, 0u128) as bytes"),
        &no_input(),
    );
    // MAX² → hi != 0 → 90
    agree4(
        &format!("return mul_add({MAX_U128}u128, {MAX_U128}u128, 0u128) as bytes"),
        &no_input(),
    );
    // mul_sub underflow: 1·1 - 5 < 0 → 90
    agree4("return mul_sub(1u128, 1u128, 5u128) as bytes", &no_input());
    agree4("return mul_sub(1u64, 1u64, 5u64) as bytes", &no_input());
    // 2·2^127 = 2^128 exactly → 90 (the +0 form of the boundary)
    agree4(
        "return mul_add(2u128, 170141183460469231731687303715884105728u128, 0u128) as bytes",
        &no_input(),
    );
}

/* ---- mixed widths: zero-extension to the three-way max ---- */

#[test]
fn muladdsub_mixed_widths_agree() {
    // common width 64: the u8 side zero-extends
    agree4("return mul_add(255u8, 4u64, 7u64) as bytes", &no_input()); // 1027
    agree4("return mul_sub(255u8, 4u64, 1000u64) as bytes", &no_input()); // 20
    // common width 128 with a u128 z: 2·3 + MAX = 2^128 + 5 → hi = 1 → 90
    agree4(
        &format!("return mul_add(2u64, 3u64, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    // 2·3 - MAX underflows the 256-bit intermediate → 90
    agree4(
        &format!("return mul_sub(2u64, 3u64, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    // u128 x·y then a u64 z: BIG1·1 + MAX_U64
    agree4(
        &format!("return mul_add({BIG1}u128, 1u128, {MAX_U64}u64) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_sub({BIG1}u128, 1u128, {MAX_U64}u64) as bytes"),
        &no_input(),
    );
}

/* ---- fault faces ---- */

/// Nil operand → CastFail 71 (the to_uint face), x/y/z order.
#[test]
fn muladdsub_nil_operand_is_71() {
    let inputs = &[vec![0], vec![1]];
    for body in [
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_add(n, 1u64, 3u64)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_sub(7u64, n, 3u64)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_add(7u64, 1u64, n)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = mul_sub(n, 1u128, 3u128)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = mul_add(7u128, 1u128, n)\nreturn (r) as bytes",
    ] {
        agree4(body, inputs);
    }
}

/// A bool operand has no static uint width — a legal compile reject (the
/// interpreter would fault 71 at run time; the kernel refuses the shape).
#[test]
fn muladdsub_bool_operand_rejects() {
    for body in [
        "return mul_add((1u64 == 1u64), 1u64, 3u64) as bytes",
        "return mul_sub(7u128, (1u64 == 1u64), 3u128) as bytes",
    ] {
        assert!(Interp::try_build(body).is_ok(), "{body}: interp should accept");
        assert!(
            Rv64::try_build(body).is_err(),
            "{body}: rv64 should reject the bool operand"
        );
    }
}

/* ---- data-dependent + random ---- */

#[test]
fn muladdsub_data_dependent() {
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_add(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![255, 255, 1]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_sub(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![255, 1, 254]],
    );
}

/// Deterministic random bombardment: fresh u128 operands per input (nonzero
/// high halves exercise the u256 carry/borrow chains), both op forms.
#[test]
fn muladdsub_random_bombardment() {
    let mut seed = 0x9E3779B97F4A7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut inputs = Vec::new();
    for _ in 0..48 {
        inputs.push((0..48).map(|_| (rnd() & 0xFF) as u8).collect::<Vec<u8>>());
    }
    let body = |op: &str, wide: bool| {
        if wide {
            format!(
                "var h = ((byte(data, 0) as u128) * 65536u128) + (byte(data, 1) as u128)\nvar l = ((byte(data, 2) as u128) * 65536u128) + (byte(data, 3) as u128)\nvar a = (h * 18446744073709551616u128) + l\nvar h2 = ((byte(data, 16) as u128) * 65536u128) + (byte(data, 17) as u128)\nvar l2 = ((byte(data, 18) as u128) * 65536u128) + (byte(data, 19) as u128)\nvar b = (h2 * 18446744073709551616u128) + l2\nvar h3 = ((byte(data, 32) as u128) * 65536u128) + (byte(data, 33) as u128)\nvar l3 = ((byte(data, 34) as u128) * 65536u128) + (byte(data, 35) as u128)\nvar c = (h3 * 18446744073709551616u128) + l3\nvar r = {op}(a, b, c)\nreturn ((r) as bytes)"
            )
        } else {
            format!(
                "var a = ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64)\nvar b = ((byte(data, 16) as u64) * 256u64) + (byte(data, 17) as u64)\nvar c = ((byte(data, 32) as u64) * 256u64) + (byte(data, 33) as u64)\nvar r = {op}(a, b, c)\nreturn (r as u64) as bytes"
            )
        }
    };
    agree4(&body("mul_add", false), &inputs);
    agree4(&body("mul_sub", false), &inputs);
    agree4(&body("mul_add", true), &inputs);
    agree4(&body("mul_sub", true), &inputs);
}

/// The wide ops must not disturb values parked below them on the vstack.
#[test]
fn muladdsub_nested_under_live_values() {
    agree4(
        "var p = 123456789u64\nvar r = (p + mul_add(3000000000u64, 3u64, 5000000000u64))\nreturn (r as u64) as bytes",
        &no_input(),
    );
    agree4(&format!(
        "var p = {BIG1}u128\nvar r = (p + mul_sub({BIG1}u128, 1u128, {BIG2}u128))\nreturn ((r) as bytes)"
    ), &no_input());
}
