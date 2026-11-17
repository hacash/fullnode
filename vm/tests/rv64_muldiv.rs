//! kehuobu 8 / T4 · MULDIV / MULDIVUP lowering.
//!
//! The interpreter widens the three operands (`arithmetic_args3`) and casts
//! the quotient back to that width. Widths up to 32 lower inline (the
//! product fits one limb, so one mul + divu/remu pair is exact); T4 lifted
//! the u64 and u128 widths onto the u256 library — the 256-bit product ÷
//! the shared 256/128 soft-division routine (z == 0 / quotient above u128
//! fault 90 inside; the ceil form bumps a nonzero remainder; the quotient
//! range-checks to the common width). The interpreter's wide intermediate
//! is observable behavior: splitting into `var t = x * y; t / z` is NOT the
//! same semantics, so the op lowers as one unit at every width.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const BIGM: &str = "1000000000000000000000000000007";

/// Four-way agreement: the interpreter's value or fault code must match all
/// three kernel modes.
fn agree4(body: &str) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let want = it.run(&[]);
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
    for (mode, got) in rv.run(&[]) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}]"),
            (Err(w), Err(code)) => assert_eq!(w.code as u64, *code, "{body} [{mode:?}]"),
            _ => panic!("{body} [{mode:?}]: interp={want:?} rv64={got:?}"),
        }
    }
}

#[test]
fn muldiv_floor_and_ceil_agree() {
    agree4("return mul_div(7u32, 5u32, 8u32) as bytes");
    agree4("return mul_div_up(7u32, 5u32, 8u32) as bytes");
    agree4("var a = 255u32\nreturn mul_div(a, 3u32, 7u32) as bytes");
    agree4("var a = 255u32\nreturn mul_div_up(a, 3u32, 7u32) as bytes");
    // exact division: ceil == floor when the remainder is zero
    agree4("return mul_div(6u32, 4u32, 8u32) as bytes");
    agree4("return mul_div_up(6u32, 4u32, 8u32) as bytes");
    agree4("return mul_div(9u16, 5u16, 8u16) as bytes");
}

#[test]
fn muldiv_width_follows_the_widened_operands() {
    // 255*255/2 = 32512 fits u16 but not u8: the u16 divisor widens the
    // three-way width to 16, so the result is a u16 (32512); the all-u8
    // form overflows the u8 result width and faults 90.
    agree4("return mul_div(255u8, 255u8, 2u8) as bytes");
    agree4("return mul_div(255u8, 255u8, 2u16) as bytes");
    agree4("return mul_div(1u8, 2u8, 3u8) as bytes");
    agree4("var a = 1u8\nvar b = 2u16\nreturn mul_div(a, b, 3u32) as bytes");
}

#[test]
fn muldiv_error_faces() {
    agree4("return mul_div(5u32, 3u32, 0u32) as bytes");
    agree4("return mul_div_up(5u32, 3u32, 0u32) as bytes");
    agree4("return mul_div(200u8, 200u8, 1u8) as bytes"); // 40000 > u8
    agree4("return mul_div_up(200u8, 200u8, 1u8) as bytes");
}

/* ---- T4: the u64 and u128 widths (u256 library path) ---- */

/// u64-width products that exceed 64 bits — the very reason the two-statement
/// split is not equivalent (`x * y` would fault at u64, the interpreter's
/// wide intermediate never does).
#[test]
fn muldiv_u64_agrees() {
    // (2^32-1)^2 = 2^64 - 2^33 + 1: only representable through the 256-bit
    // intermediate; /2 floors to 2^63 - 2^32 (fits u64)
    agree4("return mul_div(4294967295u64, 4294967295u64, 2u64) as bytes");
    agree4("return mul_div_up(4294967295u64, 4294967295u64, 2u64) as bytes");
    agree4(
        &format!("return mul_div({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64) as bytes"),
    );
    agree4(
        &format!("return mul_div_up({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64) as bytes"),
    );
    agree4(&format!("return mul_div({MAX_U64}u64, 2u64, 3u64) as bytes"));
    agree4(&format!("return mul_div(0u64, {MAX_U64}u64, 7u64) as bytes"));
    // quotient fits u128 but not the u64 common width → 90
    agree4(&format!("return mul_div({MAX_U64}u64, {MAX_U64}u64, 1u64) as bytes"));
    agree4(&format!("return mul_div_up({MAX_U64}u64, {MAX_U64}u64, 1u64) as bytes"));
    // z = 0 → 90 at the wide width
    agree4(&format!("return mul_div({MAX_U64}u64, 5u64, 0u64) as bytes"));
    agree4(&format!("return mul_div_up({MAX_U64}u64, 5u64, 0u64) as bytes"));
}

#[test]
fn muldiv_u128_agrees() {
    agree4(&format!("return mul_div({BIG1}u128, {BIG2}u128, {BIGM}u128) as bytes"));
    agree4(&format!("return mul_div_up({BIG1}u128, {BIG2}u128, {BIGM}u128) as bytes"));
    // MAX² / MAX = MAX exactly (zero remainder: ceil == floor)
    agree4(&format!(
        "return mul_div({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"
    ));
    agree4(&format!(
        "return mul_div_up({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"
    ));
    agree4(&format!(
        "return mul_div({MAX_U128}u128, {MAX_U128}u128, {MAX_U64}u128) as bytes"
    ));
    // quotient above u128 (hi >= z) → 90
    agree4(&format!("return mul_div({MAX_U128}u128, {MAX_U128}u128, 1u128) as bytes"));
    agree4(&format!("return mul_div_up({MAX_U128}u128, {MAX_U128}u128, 1u128) as bytes"));
    // z = 0 → 90
    agree4(&format!("return mul_div({BIG1}u128, {BIG2}u128, 0u128) as bytes"));
    agree4(&format!("return mul_div_up({BIG1}u128, {BIG2}u128, 0u128) as bytes"));
}

/// The ceil corner: floor quotient exactly u128::MAX with a nonzero
/// remainder — `quo + 1` overflows u128 → Arithmetic 90 (the floor form of
/// the same operands yields MAX). 2 · (3·2^127 - 1) = 3·2^128 - 2 =
/// 3·(2^128 - 1) + 1, so ÷3 → quo = 2^128 - 1, rem = 1.
#[test]
fn muldiv_u128_ceil_carry_faults() {
    agree4(
        "return mul_div_up(2u128, 40564819207303340847894502572031u128, 3u128) as bytes",
    );
    agree4(
        "return mul_div(2u128, 40564819207303340847894502572031u128, 3u128) as bytes",
    );
}

/// Mixed widths: the interpreter zero-extends every operand to the three-way
/// max (cast_same_uint_width3) — the common width decides the result cast.
#[test]
fn muldiv_wide_mixed_widths_agree() {
    // common width 64: 255·255 = 65025 fits; the u8-only form faults 90
    agree4("return mul_div(255u8, 255u8, 2u64) as bytes");
    agree4("return mul_div_up(255u8, 255u8, 2u64) as bytes");
    // common width 128: the u64 sides zero-extend
    agree4(&format!("return mul_div({BIG1}u128, 12345u64, {BIGM}u128) as bytes"));
    agree4(&format!("return mul_div_up({BIG1}u128, 12345u64, {BIGM}u128) as bytes"));
    agree4(&format!("return mul_div(12345u64, {BIG1}u128, {BIGM}u128) as bytes"));
    // narrow common width, big product for it: 255·255 = 65025 fits u32
    agree4("return mul_div(255u16, 255u16, 1u32) as bytes");
}

#[test]
fn muldiv_wide_data_dependent() {
    let inputs: &[Vec<u8>] = &[
        vec![0, 0, 0],
        vec![255, 255, 255],
        vec![200, 100, 7],
        vec![1, 2, 0],
        vec![255, 1, 1],
    ];
    for body in [
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_div(a, b, c)\nreturn (r as u64) as bytes",
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_div_up(a, b, c)\nreturn (r as u64) as bytes",
    ] {
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
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
}

/// The wide division must not disturb values parked below it on the vstack
/// (the shared 256/128 routine only touches its fixed working registers and
/// the frame workspace — pool-external in every mode).
#[test]
fn muldiv_wide_nested_under_live_values() {
    agree4(
        "var p = 123456789u64\nvar r = (p + mul_div(4294967295u64, 4294967295u64, 3u64))\nreturn (r as u64) as bytes",
    );
    agree4(&format!(
        "var p = {BIGM}u128\nvar r = (p + mul_div_up({BIG1}u128, {BIG2}u128, {BIGM}u128))\nreturn ((r) as bytes)"
    ));
    agree4(&format!(
        "var p = {BIGM}u128\nvar r = (p + mul_add({BIG1}u128, 1u128, 1u128))\nreturn ((r) as bytes)"
    ));
}

/// Nil operand → CastFail 71 (the to_uint face), x/y/z order, both widths.
#[test]
fn muldiv_wide_nil_operand_is_71() {
    let inputs = &[vec![0], vec![1]];
    for body in [
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_div(n, 1u64, 3u64)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_div_up(7u64, n, 3u64)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_div(7u64, 1u64, n)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = mul_div(n, 1u128, 3u128)\nreturn (r) as bytes",
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = mul_div_up(7u128, 1u128, n)\nreturn (r) as bytes",
    ] {
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
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
}

/// A bool operand has no static uint width — a legal compile reject (the
/// interpreter would fault 71 at run time; the kernel refuses the shape).
#[test]
fn muldiv_bool_operand_rejects() {
    for body in [
        "return mul_div((1u64 == 1u64), 1u64, 3u64) as bytes",
        "return mul_div_up(7u128, (1u64 == 1u64), 3u128) as bytes",
    ] {
        assert!(Interp::try_build(body).is_ok(), "{body}: interp should accept");
        assert!(
            Rv64::try_build(body).is_err(),
            "{body}: rv64 should reject the bool operand"
        );
    }
}

/// Deterministic random bombardment at the u64 and u128 widths.
#[test]
fn muldiv_wide_random_bombardment() {
    let mut seed = 0x13198A2E03707344u64;
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
    let bodies = [
        // u64: two u64 operands and a nonzero-ish u64 divisor
        "var a = ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64)\nvar b = ((byte(data, 16) as u64) * 256u64) + (byte(data, 17) as u64)\nvar c = ((byte(data, 32) as u64) * 256u64) + (byte(data, 33) as u64) + 1u64\nvar r = mul_div(a, b, c)\nreturn (r as u64) as bytes",
        "var a = ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64)\nvar b = ((byte(data, 16) as u64) * 256u64) + (byte(data, 17) as u64)\nvar c = ((byte(data, 32) as u64) * 256u64) + (byte(data, 33) as u64) + 1u64\nvar r = mul_div_up(a, b, c)\nreturn (r as u64) as bytes",
        // u128: full 128-bit operands with nonzero high halves
        "var h = ((byte(data, 0) as u128) * 65536u128) + (byte(data, 1) as u128)\nvar l = ((byte(data, 2) as u128) * 65536u128) + (byte(data, 3) as u128)\nvar a = (h * 18446744073709551616u128) + l\nvar h2 = ((byte(data, 16) as u128) * 65536u128) + (byte(data, 17) as u128)\nvar l2 = ((byte(data, 18) as u128) * 65536u128) + (byte(data, 19) as u128)\nvar b = (h2 * 18446744073709551616u128) + l2\nvar h3 = ((byte(data, 32) as u128) * 65536u128) + (byte(data, 33) as u128)\nvar l3 = ((byte(data, 34) as u128) * 65536u128) + (byte(data, 35) as u128)\nvar c = ((h3 * 18446744073709551616u128) + l3) + 1u128\nvar r = mul_div(a, b, c)\nreturn ((r) as bytes)",
        "var h = ((byte(data, 0) as u128) * 65536u128) + (byte(data, 1) as u128)\nvar l = ((byte(data, 2) as u128) * 65536u128) + (byte(data, 3) as u128)\nvar a = (h * 18446744073709551616u128) + l\nvar h2 = ((byte(data, 16) as u128) * 65536u128) + (byte(data, 17) as u128)\nvar l2 = ((byte(data, 18) as u128) * 65536u128) + (byte(data, 19) as u128)\nvar b = (h2 * 18446744073709551616u128) + l2\nvar h3 = ((byte(data, 32) as u128) * 65536u128) + (byte(data, 33) as u128)\nvar l3 = ((byte(data, 34) as u128) * 65536u128) + (byte(data, 35) as u128)\nvar c = ((h3 * 18446744073709551616u128) + l3) + 1u128\nvar r = mul_div_up(a, b, c)\nreturn ((r) as bytes)",
    ];
    for body in bodies {
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
        for input in &inputs {
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
}
