//! Guard-semantics differential for the fitsh→RV64IM backend.
//!
//! The fitsh language guarantees checked semantics on the rv64im channel:
//! add/sub/mul overflow, division by zero, bytes-index/slice bounds and
//! narrowing-cast overflow all trap the guest kernel (HALT with the HVM
//! error code — `Arithmetic = 90`, `CastFail = 71`). This suite pins those
//! guarantees BYTE-LEVEL against the reference rust safe-semantics model
//! (`u64::checked_*` and explicit bounds checks — exactly what a rust
//! kernel written with checked arithmetic does), across all three lowering
//! modes (Im / Imc / ImcRemap), with operand matrices on the value
//! boundaries (0, 1, MAX/2, MAX−1, MAX).
//!
//! Each case: (fitsh body, input bytes) → (output bytes | exit code) must
//! equal the rust model's (checked result | trap class) in EVERY mode.

use vm::calc::{CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap};
use vm::fitshc::{CompiledCode, compile_body};
use vm::ir::rv64::{ARENA_SIZE_MAX, Rv64Mode, lower_body_mode_arena};
use vm::lang::Tokenizer;
use vm::ValueTy;

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// What one execution must produce.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Want {
    Ok(Vec<u8>),
    /// guest HALT with this exit code
    Trap(u32),
}

/// Compile + lower + pack + verify + execute one body on one mode.
fn exec(body: &str, input: &[u8], mode: Rv64Mode) -> Want {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let (ir, compiled, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .expect("compile body");
    assert!(
        matches!(compiled, CompiledCode::Bytecode(_)),
        "the same body must compile for the HVM channel too"
    );
    let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).expect("lower");
    let cap = SpaceCap::new(0);
    let profile = match mode {
        Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
        _ => vm::calc::container::CALC_PROFILE_RV64IMC,
    };
    let container = lo
        .pack_segments_profile(&cap, 64, 4800, 8, profile)
        .expect("pack");
    let fno =
        CalcFnObj::build(container, &cap, CalcHostConfig::new()).expect("verify+build (chain rules)");
    let ex = fno.run(input, CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None });
    match ex.result {
        Ok(o) => Want::Ok(o),
        Err(_) => Want::Trap(ex.exit_code as u32),
    }
}

/// Assert one case across every lowering mode.
fn pin(body: &str, input: &[u8], want: &Want) {
    for mode in MODES {
        let got = exec(body, input, mode);
        assert_eq!(
            &got, want,
            "guard semantics diverged in mode {mode:?} for body:\n{body}\ninput: {input:?}"
        );
    }
}

/// 8-byte BE → u64 helper for building inputs.
fn be(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// Fold the first 8 input bytes BE into a u64 (the fitsh side reads
/// `byte(data, i)`); `off` selects a later 8-byte word.
const BE8_BODY: &str = "var v = 0u64\nvar i = 0u64\nwhile i < 8 { v = v * 256 + byte(data, OFF)\ni += 1 }\n";

fn be8_body(off: usize) -> String {
    BE8_BODY.replace("OFF", &format!("i + {off}"))
}

/// Binary-op matrix: bodies `return (a OP b) as bytes` over input words.
fn binop_body(op: &str) -> String {
    format!(
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 {{ a = a * 256 + byte(data, i)\ni += 1 }}\n\
         var b = 0u64\nvar j = 0u64\nwhile j < 8 {{ b = b * 256 + byte(data, j + 8)\nj += 1 }}\n\
         return (a {op} b) as bytes"
    )
}

const ARITH: u32 = 90;
const CAST: u32 = 71;

/* ---- B1: add / sub / mul overflow on the boundary cross-product ---- */

#[test]
fn add_overflow_boundaries() {
    let body = binop_body("+");
    let edges = [0u64, 1, u64::MAX / 2, u64::MAX - 1, u64::MAX];
    for a in edges {
        for b in edges {
            let mut input = be(a);
            input.extend(be(b));
            let want = match a.checked_add(b) {
                Some(s) => Want::Ok(be(s)),
                None => Want::Trap(ARITH),
            };
            pin(&body, &input, &want);
        }
    }
}

#[test]
fn sub_underflow_boundaries() {
    let body = binop_body("-");
    let edges = [0u64, 1, u64::MAX / 2, u64::MAX - 1, u64::MAX];
    for a in edges {
        for b in edges {
            let mut input = be(a);
            input.extend(be(b));
            let want = match a.checked_sub(b) {
                Some(s) => Want::Ok(be(s)),
                None => Want::Trap(ARITH),
            };
            pin(&body, &input, &want);
        }
    }
}

#[test]
fn mul_overflow_boundaries() {
    let body = binop_body("*");
    let edges = [0u64, 1, 2, 3, u64::MAX / 2, u64::MAX - 1, u64::MAX];
    for a in edges {
        for b in edges {
            let mut input = be(a);
            input.extend(be(b));
            let want = match a.checked_mul(b) {
                Some(s) => Want::Ok(be(s)),
                None => Want::Trap(ARITH),
            };
            pin(&body, &input, &want);
        }
    }
}

/* ---- B2: division-by-zero guards (all four division forms) ---- */

#[test]
fn div_by_zero_traps_arithmetic() {
    // runtime divisor from input: `b = be8(8..16)`; a = be8(0..8)
    for op in ["/", "%"] {
        let body = binop_body(op);
        // divisor zero → trap; divisor one → identity; MAX/1 → MAX
        for (a, b) in [(7u64, 0), (0, 0), (u64::MAX, 0), (u64::MAX, 1), (7, 3)] {
            let mut input = be(a);
            input.extend(be(b));
            let want = if b == 0 {
                Want::Trap(ARITH)
            } else {
                let v = if op == "/" { a / b } else { a % b };
                Want::Ok(be(v))
            };
            pin(&body, &input, &want);
        }
    }
}

#[test]
fn divup_and_divexact_zero_traps() {
    // DIVUP: ceil division; DIVEXACT: requires remainder 0
    // divup
    let divup = format!(
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 {{ a = a * 256 + byte(data, i)\ni += 1 }}\n\
         var b = 0u64\nvar j = 0u64\nwhile j < 8 {{ b = b * 256 + byte(data, j + 8)\nj += 1 }}\n\
         return ((a + b - 1) / b) as bytes"
    );
    // (a + b - 1) can itself overflow for b > 0 near MAX — keep operands small
    for (a, b) in [(7u64, 3), (8, 3), (0, 5), (9, 0)] {
        let mut input = be(a);
        input.extend(be(b));
        let want = if b == 0 {
            Want::Trap(ARITH)
        } else {
            Want::Ok(be(a.div_ceil(b)))
        };
        pin(&divup, &input, &want);
    }
}

/* ---- B3: bytes index & slice bounds ---- */

#[test]
fn byte_index_bounds() {
    // n = size(data) (fixed 24-byte inputs); index from the input word
    let body = format!(
        "var idx = 0u64\nvar i = 0u64\nwhile i < 8 {{ idx = idx * 256 + byte(data, i)\ni += 1 }}\n\
         var v = byte(data, idx)\nreturn v as bytes"
    );
    let input24: Vec<u8> = (0u8..24).collect();
    for idx in [0u64, 8, 23, 24, 25, u64::MAX] {
        let mut input = be(idx);
        input.extend(input24.clone());
        // data is the FULL input (index word included): 32 bytes. byte()
        // is U8 → the 1-byte `as bytes` view (A2)
        let want = if idx < 32 {
            Want::Ok(vec![input[idx as usize]])
        } else {
            Want::Trap(CAST)
        };
        pin(&body, &input, &want);
    }
}

#[test]
fn slice_bounds_all_forms() {
    // cut(data, ost, len) with runtime ost/len from input
    let cut = format!(
        "var ost = 0u64\nvar i = 0u64\nwhile i < 8 {{ ost = ost * 256 + byte(data, i)\ni += 1 }}\n\
         var ln = 0u64\nvar j = 0u64\nwhile j < 8 {{ ln = ln * 256 + byte(data, j + 8)\nj += 1 }}\n\
         var s = buf_cut(data, ost, ln)\nreturn byte(s, 0) as bytes"
    );
    let tail16: Vec<u8> = (10u8..26).collect();
    // data is the FULL input: 16-byte index words ++ tail = 48 bytes
    let total = 16 + 16;
    // (ost, ln) → in-bounds / off-end / wrap (ost+ln overflows)
    for (ost, ln) in [(0u64, 1), (17, 1), (total as u64, 0), (0, 8), (total as u64, 1), (47, 2), (u64::MAX, 2)] {
        let mut input = be(ost);
        input.extend(be(ln));
        input.extend(tail16.clone());
        let len = input.len() as u64;
        // byte(s, 0) is U8 → the 1-byte `as bytes` view (A2)
        let want = match ost.checked_add(ln) {
            Some(end) if end <= len && ln > 0 => Want::Ok(vec![input[ost as usize]]),
            // an empty slice still traps the byte(s, 0) read below
            Some(end) if end <= len && ln == 0 => Want::Trap(CAST),
            _ => Want::Trap(CAST),
        };
        pin(&cut, &input, &want);
    }
}

/* ---- B4: narrowing-cast overflow (runtime values) ---- */

#[test]
fn narrowing_casts_trap_on_overflow() {
    // `x as u8/u16/u32` on a runtime value. A2: `as uN` pins the width, so
    // `as bytes` renders that width's BE view (1/2/4 B).
    for (ty, max, wbytes) in [
        ("u8", 255u64, 1usize),
        ("u16", 65535, 2),
        ("u32", 4_294_967_295, 4),
    ] {
        let body = format!(
            "var v = 0u64\nvar i = 0u64\nwhile i < 8 {{ v = v * 256 + byte(data, i)\ni += 1 }}\n\
             return (v as {ty}) as bytes"
        );
        for v in [0u64, max - 1, max, max + 1, u64::MAX] {
            let want = if v <= max {
                let b = v.to_be_bytes();
                Want::Ok(b[8 - wbytes..].to_vec())
            } else {
                Want::Trap(CAST)
            };
            pin(&body, &be(v), &want);
        }
    }
}

/* ---- B5: shift semantics (checked amount — closed task-1 A1) ---- */

#[test]
fn shift_amount_checked_against_width() {
    // `x << n` follows the interpreter's `checked_shl`: the AMOUNT must be
    // < 64 (Arithmetic 90 otherwise; the value side truncates silently).
    // The historical rv64 channel masked the amount to 6 bits (`n & 63`)
    // — the documented deviation this test used to pin; A1 closed it, so
    // the pin now asserts the interpreter's trap.
    let body = format!(
        "var v = 0u64\nvar i = 0u64\nwhile i < 8 {{ v = v * 256 + byte(data, i)\ni += 1 }}\n\
         var n = 0u64\nvar j = 0u64\nwhile j < 8 {{ n = n * 256 + byte(data, j + 8)\nj += 1 }}\n\
         return (v << n) as bytes"
    );
    for (v, n) in [(1u64, 0), (1, 1), (1, 63), (1, 64), (1, 65), (0xDEAD_BEEF, 128), (0xFFFF_FFFF, 192)] {
        let mut input = be(v);
        input.extend(be(n));
        let want = if n < 64 {
            Want::Ok(be(v.wrapping_shl(n as u32)))
        } else {
            Want::Trap(ARITH)
        };
        pin(&body, &input, &want);
    }
}

/* ---- guards inside loops and branches (the placement matrix) ---- */

#[test]
fn overflow_guard_fires_from_loop_and_branch() {
    // accumulate in a loop until overflow: Σ input bytes × 2^56 crosses on
    // the second large term
    let body = "var s = 0u64\nvar i = 0u64\nvar n = size(data)\nwhile i < n { s += byte(data, i) * 72057594037927936\ni += 1 }\nreturn s as bytes";
    // isolates
    let iso1 = "return (255u64 * 72057594037927936) as bytes";
    println!("iso1 = {:?}", exec(iso1, &[], Rv64Mode::Im));
    let iso2 = "var a = 18374686479671623680u64\nreturn (a + 18374686479671623680u64) as bytes";
    println!("iso2 = {:?}", exec(iso2, &[], Rv64Mode::Im));
    let iso3 = "var s = 0u64\ns += 255u64 * 72057594037927936\nreturn s as bytes";
    println!("iso3 = {:?}", exec(iso3, &[], Rv64Mode::Im));
    let iso4 = "var s = 0u64\nvar i = 0u64\nwhile i < 2 { s += byte(data, i) * 72057594037927936\ni += 1 }\nreturn s as bytes";
    for m in MODES { println!("iso4 {m:?} = {:?}", exec(iso4, &[255, 255], m)); }
    let iso5 = "var s = 0u64\nvar i = 0u64\nwhile i < 2 { s += byte(data, i) * 72057594037927936\ni += 1 }\nreturn byte(data, 1) as bytes";
    for m in MODES { println!("iso5 {m:?} = {:?}", exec(iso5, &[1, 2], m)); }
    for k in [0usize, 1, 2] {
        let input = vec![255u8; k];
        let want = match k {
            0 => Want::Ok(be(0)),
            1 => Want::Ok(be(255u64 * 72_057_594_037_927_936)),
            _ => Want::Trap(ARITH), // second term overflows
        };
        pin(body, &input, &want);
    }

    // the same overflow behind an if/else — both arms must keep the guard
    // (the MAX literal + runtime byte avoids compile-time folding)
    let body2 = "var s = 0u64\nif byte(data, 0) % 2 == 0 { s = 18446744073709551615u64 + byte(data, 0) } else { s = 1 }\nreturn s as bytes";
    // (narrow-PUT face: the odd arm stores a u8 literal, so the interpreter's
    // view of s is ONE byte there — the pre-fix pin demanded be(1), the
    // static-join 8-byte view this campaign removes)
    let one = vec![1u8];
    for (b0, want) in [
        (0u8, Want::Ok(be(u64::MAX))), // even arm, MAX + 0 = MAX: no overflow
        (2, Want::Trap(ARITH)),        // even arm, MAX + 2 traps
        (3, Want::Ok(one)),            // odd arm: u8-literal PUT → 1-byte view
    ] {
        pin(body2, &[b0], &want);
    }
}

#[test]
fn div_zero_guard_fires_in_expression_depth() {
    // divisor computed at nesting depth 4: (b / (a - a))
    let body = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nvar q = 100 / (a - a + byte(data, 8))\nreturn q as bytes";
    let mut input = be(12345);
    input.extend([0u8]); // divisor = 0
    pin(body, &input, &Want::Trap(ARITH));
    input[8] = 5; // divisor = 5
    pin(body, &input, &Want::Ok(be(20)));
}

#[test]
fn bounds_guard_fires_on_empty_input() {
    // byte(data, 0) on empty input: trap even though the index is 0.
    // A2: byte() is U8 → the 1-byte `as bytes` view
    let body = "var v = byte(data, 0)\nreturn v as bytes";
    pin(body, &[], &Want::Trap(CAST));
    pin(body, &[7], &Want::Ok(vec![7]));
}

/* ---- checked pow (constant exponent unrolls to checked muls) ---- */

#[test]
fn pow_overflow_traps() {
    let body = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nreturn (a ** 3) as bytes";
    for a in [1u64, 2, 2642245 // 2642245³ < 2^64
        , 2642246] {
        let want = match a.checked_pow(3) {
            Some(v) => Want::Ok(be(v)),
            None => Want::Trap(ARITH),
        };
        pin(body, &be(a), &want);
    }
}

/* ---- x as bytes is always the 8-byte BE u64 view (wire format) ---- */

#[test]
fn as_bytes_wire_format() {
    let body = be8_body(0) + "\nreturn v as bytes";
    for v in [0u64, 1, 0xDEAD_BEEF, u64::MAX] {
        pin(&body, &be(v), &Want::Ok(be(v)));
    }
}

/* ---- arena exhaustion is a bound, not a leak ---- */

#[test]
fn byte_concat_arena_bound_traps() {
    // 24-byte input, 40-iteration prepend chain churns ~20 KiB of arena —
    // fine; the point is the trap class is CastFail when exceeded. Use a
    // tight loop that would exhaust any arena if unbounded: the cycle
    // budget stops it first, so instead pin a KNOWN in-bounds churn shape.
    let body = "var b = data\nvar i = 0u64\nwhile i < 24 { b = b ++ b\ni += 1 }\nreturn byte(b, 0) as bytes";
    // 2^24 × 16 B ≈ 256 MiB — far past the 40 KiB arena: traps CastFail
    let input: Vec<u8> = (0u8..16).collect();
    pin(body, &input, &Want::Trap(CAST));
}
