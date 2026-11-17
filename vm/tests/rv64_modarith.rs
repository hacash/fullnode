//! T2 · ADDMOD / MULMOD lowering (`add_mod` / `mul_mod`, u64 + u128 widths).
//!
//! Interpreter ground truth (execute.rs ADDMOD/MULMOD → triop_arithmetic →
//! arithmetic_args3 + instruction.rs addmod_checked / mulmod_checked):
//! the three `to_uint` operands fault CastFail 71 on a non-uint (Nil) value
//! in x, y, z order, then zero-extend to the three-way max width; z == 0
//! faults Arithmetic 90; the modulus result — (x+y) mod z via add_mod_u128's
//! overflow-free gap branch, (x·y) mod z via mul_mod_u128's double-and-add
//! — casts back to that common width (always fits: < z). The kernel lowers
//! every width up to 64 inline on one register and the 128-bit width
//! through the (lo, hi) pair machinery (two shared-divmod residues + the
//! in-place add_mod branch for add_mod; a shared double-and-add routine for
//! mul_mod). u256-width mulmod needs the u256 library and stays rejected.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const BIGM: &str = "1000000000000000000000000000007";

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

/* ---- u64 width ---- */

#[test]
fn addmod_u64_agrees() {
    agree4("return add_mod(5u64, 7u64, 10u64) as bytes", &no_input());
    agree4(
        &format!("return add_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    ); // (2·MAX) mod MAX = 0
    agree4(
        &format!("return add_mod({MAX_U64}u64, 1u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    ); // 0 + 1
    agree4(
        &format!("return add_mod({MAX_U64}u64, {MAX_U64}u64, 3u64) as bytes"),
        &no_input(),
    ); // residues 2+2 mod 3 = 1
    agree4("return add_mod(7u64, 0u64, 5u64) as bytes", &no_input());
    agree4("return add_mod(0u64, 0u64, 1u64) as bytes", &no_input());
}

#[test]
fn mulmod_u64_agrees() {
    agree4("return mul_mod(11u64, 13u64, 17u64) as bytes", &no_input());
    agree4("return mul_mod(0u64, 13u64, 17u64) as bytes", &no_input());
    agree4("return mul_mod(19u64, 1u64, 17u64) as bytes", &no_input());
    agree4(
        &format!("return mul_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64) as bytes"),
        &no_input(),
    ); // 0
    // 2^32 squared mod (2^32 - 1) = 1
    agree4(
        "return mul_mod(4294967296u64, 4294967296u64, 4294967295u64) as bytes",
        &no_input(),
    );
    // odd modulus with dense bit patterns
    agree4(
        "return mul_mod(123456789u64, 987654321u64, 1000000007u64) as bytes",
        &no_input(),
    );
}

#[test]
fn modarith_z_zero_and_one() {
    agree4("return add_mod(5u64, 3u64, 0u64) as bytes", &no_input());
    agree4("return mul_mod(5u64, 3u64, 0u64) as bytes", &no_input());
    agree4(
        &format!("return add_mod({BIG1}u128, {BIG2}u128, 0u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_mod({BIG1}u128, {BIG2}u128, 0u128) as bytes"),
        &no_input(),
    );
    // z = 1 reduces everything to 0 (exercises the subtract branch at the
    // first partial: every residue is 0, sums reach z immediately)
    agree4("return add_mod(5u64, 3u64, 1u64) as bytes", &no_input());
    agree4("return mul_mod(5u64, 3u64, 1u64) as bytes", &no_input());
    agree4(
        &format!("return mul_mod({MAX_U128}u128, {MAX_U128}u128, 1u128) as bytes"),
        &no_input(),
    );
}

#[test]
fn modarith_data_dependent_u64() {
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = add_mod(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![1, 2, 0]],
    );
    agree4(
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar r = mul_mod(a, b, c)\nreturn (r as u64) as bytes",
        &[vec![0, 0, 0], vec![255, 255, 255], vec![200, 100, 7], vec![1, 2, 0]],
    );
}

/* ---- narrow widths (the common width is the three-way max) ---- */

#[test]
fn modarith_narrow_widths_agree() {
    agree4("return add_mod(200u8, 200u8, 256u16) as bytes", &no_input()); // 144
    agree4("return add_mod(255u8, 255u8, 255u8) as bytes", &no_input()); // 0
    agree4("return mul_mod(250u8, 250u8, 251u8) as bytes", &no_input()); // 1
    agree4("return mul_mod(65535u16, 65535u16, 65537u32) as bytes", &no_input());
    agree4("return add_mod(1u8, 2u8, 3u32) as bytes", &no_input());
    agree4("return mul_mod(3000000000u32, 3000000000u32, 7u32) as bytes", &no_input());
}

/* ---- u128 width ---- */

#[test]
fn addmod_u128_agrees() {
    agree4(
        &format!("return add_mod({BIG1}u128, {BIG2}u128, {BIGM}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return add_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // 0
    agree4(
        &format!("return add_mod({MAX_U128}u128, 1u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return add_mod({MAX_U128}u128, {MAX_U128}u128, 3u128) as bytes"),
        &no_input(),
    ); // (2^128-1 is odd: residues 2+2 mod 3 = 1)
    agree4(
        &format!(
            "var a = ((byte(data, 0) as u128) * 18446744073709551616u128) + (byte(data, 1) as u128)\nvar b = ((byte(data, 2) as u128) * 18446744073709551616u128) + (byte(data, 3) as u128)\nvar c = ((byte(data, 4) as u128) * 79228162514264337593543950336u128) + 1u128\nreturn ((add_mod(a, b, c)) as bytes)"
        ),
        &[vec![0; 5], vec![255; 5], vec![1, 2, 3, 4, 5], vec![200, 100, 7, 9, 1]],
    );
}

#[test]
fn mulmod_u128_agrees() {
    agree4(
        &format!("return mul_mod({BIG1}u128, {BIG2}u128, {BIGM}u128) as bytes"),
        &no_input(),
    );
    agree4(
        &format!("return mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128) as bytes"),
        &no_input(),
    ); // 0
    // 2^128-1 factors over 2^64-1, so MAX² mod (2^64-1) = 0
    agree4(
        &format!(
            "return mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U64}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return mul_mod({MAX_U128}u128, {MAX_U128}u128, {BIGM}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "var a = ((byte(data, 0) as u128) * 18446744073709551616u128) + (byte(data, 1) as u128)\nvar b = ((byte(data, 2) as u128) * 18446744073709551616u128) + (byte(data, 3) as u128)\nvar c = ((byte(data, 4) as u128) * 79228162514264337593543950336u128) + 1u128\nreturn ((mul_mod(a, b, c)) as bytes)"
        ),
        &[vec![0; 5], vec![255; 5], vec![1, 2, 3, 4, 5], vec![200, 100, 7, 9, 1]],
    );
}

/// Mixed widths: the interpreter zero-extends every operand to the three-way
/// max (cast_same_uint_width3), so a u64 side in a u128 add_mod/mul_mod is
/// plain zero-extension — no `as u128` pinning needed (unlike the plain
/// binary u128 arithmetic).
#[test]
fn modarith_mixed_widths_agree() {
    agree4(
        &format!(
            "return add_mod({BIG1}u128, 12345u64, {BIGM}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        &format!(
            "return mul_mod({BIG1}u128, 12345u64, {BIGM}u128) as bytes"
        ),
        &no_input(),
    );
    agree4(
        "var a = ((byte(data, 0) as u128) * 18446744073709551616u128)\nvar b = (byte(data, 1) as u64)\nvar c = ((byte(data, 2) as u128) * 3u128) + 1u128\nreturn ((mul_mod(a, b, c)) as bytes)",
        &[vec![0; 3], vec![255; 3], vec![7, 9, 1]],
    );
    // narrow sides mix up too (common width 64)
    agree4("return add_mod(255u8, 4294967296u64, 65537u64) as bytes", &no_input());
    agree4("return mul_mod(65535u16, 4294967295u32, 65537u64) as bytes", &no_input());
}

/// The 128-bit ops must not disturb values parked below them on the vstack
/// (the shared routines only touch their own fixed working registers and
/// the frame workspace — pool-external in every mode).
#[test]
fn modarith_nested_under_live_values() {
    agree4(
        &format!(
            "var z = {BIGM}u128\nvar a = ((byte(data, 0) as u128) * 123456789u128) + 1u128\nvar r = (a + add_mod(a, a, z))\nreturn ((r) as bytes)"
        ),
        &[vec![0], vec![255], vec![7]],
    );
    agree4(
        &format!(
            "var z = {BIGM}u128\nvar a = ((byte(data, 0) as u128) * 123456789u128) + 1u128\nvar r = (a + mul_mod(a, 12345678901234567890123456789u128, z))\nreturn ((r) as bytes)"
        ),
        &[vec![0], vec![255], vec![7]],
    );
    agree4(
        "var z = (byte(data, 1) as u64) + 1u64\nvar a = (byte(data, 0) as u64)\nvar r = (a * mul_mod(a, 1000000007u64, z))\nreturn ((r) as bytes)",
        &[vec![0, 0], vec![255, 255], vec![7, 3]],
    );
}

/// Nil operand → CastFail 71 (the to_uint face), x/y/z order, both widths.
#[test]
fn modarith_nil_operand_is_71() {
    let inputs = &[vec![0], vec![1]];
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = add_mod(n, 1u64, 3u64)\nreturn (r) as bytes",
        inputs,
    );
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_mod(n, 1u64, 3u64)\nreturn (r) as bytes",
        inputs,
    );
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = add_mod(7u64, n, 3u64)\nreturn (r) as bytes",
        inputs,
    );
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = mul_mod(7u64, 1u64, n)\nreturn (r) as bytes",
        inputs,
    );
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = add_mod(n, 1u128, 3u128)\nreturn (r) as bytes",
        inputs,
    );
    agree4(
        "var q = 0u64\nif (q == 1u64) { var n = 5u128 }\nvar r = mul_mod(n, 1u128, 3u128)\nreturn (r) as bytes",
        inputs,
    );
}

/// A bool operand has no static uint width — a legal compile reject (the
/// interpreter would fault 71 at run time; the kernel refuses the shape).
#[test]
fn modarith_bool_operand_rejects() {
    for body in [
        "return add_mod((1u64 == 1u64), 1u64, 3u64) as bytes",
        "return mul_mod((1u64 == 1u64), 1u64, 3u64) as bytes",
    ] {
        assert!(Interp::try_build(body).is_ok(), "{body}: interp should accept");
        assert!(
            Rv64::try_build(body).is_err(),
            "{body}: rv64 should reject the bool operand"
        );
    }
}

/// Deterministic random bombardment: fresh operands per input, all four op
/// forms (add/mul × u64/u128), every input four-way agreed. The u128 forms
/// build a nonzero high half so the routine's wrap-carry and hi-bit scan
/// paths are exercised.
#[test]
fn modarith_random_bombardment() {
    let mut seed = 0x243F6A8885A308D3u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut inputs = Vec::new();
    for _ in 0..64 {
        inputs.push((0..48).map(|_| (rnd() & 0xFF) as u8).collect::<Vec<u8>>());
    }
    agree4(
        "var a = ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64)\nvar b = ((byte(data, 16) as u64) * 256u64) + (byte(data, 17) as u64)\nvar c = ((byte(data, 32) as u64) * 256u64) + (byte(data, 33) as u64)\nvar r = add_mod(a, b, c)\nreturn (r as u64) as bytes",
        &inputs,
    );
    agree4(
        "var a = ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64)\nvar b = ((byte(data, 16) as u64) * 256u64) + (byte(data, 17) as u64)\nvar c = ((byte(data, 32) as u64) * 256u64) + (byte(data, 33) as u64)\nvar r = mul_mod(a, b, c)\nreturn (r as u64) as bytes",
        &inputs,
    );
    let u128_body = |op: &str| {
        format!(
            "var h = ((byte(data, 0) as u128) * 65536u128) + (byte(data, 1) as u128)\nvar l = ((byte(data, 2) as u128) * 65536u128) + (byte(data, 3) as u128)\nvar a = (h * 18446744073709551616u128) + l\nvar h2 = ((byte(data, 16) as u128) * 65536u128) + (byte(data, 17) as u128)\nvar l2 = ((byte(data, 18) as u128) * 65536u128) + (byte(data, 19) as u128)\nvar b = (h2 * 18446744073709551616u128) + l2\nvar h3 = ((byte(data, 32) as u128) * 65536u128) + (byte(data, 33) as u128)\nvar l3 = ((byte(data, 34) as u128) * 65536u128) + (byte(data, 35) as u128)\nvar c = (h3 * 18446744073709551616u128) + l3\nvar r = {op}(a, b, c)\nreturn ((r) as bytes)"
        )
    };
    agree4(&u128_body("add_mod"), &inputs);
    agree4(&u128_body("mul_mod"), &inputs);
}
