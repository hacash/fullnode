//! C3 (constant integer folding) + A1 (constant-exponent POW loop)
//! differentials: engine-vs-engine (interpreter vs rv64 Im/Imc/ImcRemap)
//! on every double-literal shape the rv64 backend can actually reach, plus
//! the A1 exponent×base boundary matrix.
//!
//! Reachability note (probed before writing the pins): the fitsh frontend
//! (`lang/syntax/expr.rs combine_uint_literals`) already folds double
//! uint-literal ADD/SUB/MUL/DIV/MOD/POW/BAND/BXOR/BOR/BSHL/BSHR in u128
//! arithmetic, widening the result type instead of faulting. The shapes
//! that still reach the rv64 eval arms as double literals are:
//!   * every comparison (kept, width-aligned),
//!   * builtin 2-arg calls with literal args (max/min/abs_diff/div_up/
//!     div_exact_op — call args never pass through combine_uint_literals),
//!   * the kept fault forms: SUB underflow, DIV/MOD by literal zero,
//!     shift amount ≥ width (the frontend keeps faults at runtime).
//! The C3 backend fold therefore only fires on in-range shapes and leaves
//! every deterministic-fault shape on its existing emission — the
//! `1u64 << 64` "folds nothing, traps unconditionally" pin in
//! rv64_width_semantics is the contract.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

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

/// Lowered code-stream length in 4-byte words, Im mode.
fn im_words(body: &str) -> usize {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let (_, code) = &rv.codes[0];
    assert_eq!(code.len() % 4, 0, "Im stream is word-aligned");
    code.len() / 4
}

fn byte_inputs(vals: &[u8]) -> Vec<Vec<u8>> {
    vals.iter().map(|&b| vec![b]).collect()
}

/* ================================================================== */
/* C3: folded comparisons (the frontend keeps these pairs)             */
/* ================================================================== */

#[test]
fn c3_literal_comparisons_all_directions() {
    for body in [
        "return ((2 < 3)) as bytes",
        "return ((3 < 2)) as bytes",
        "return ((2 <= 2)) as bytes",
        "return ((2 <= 1)) as bytes",
        "return ((2 >= 3)) as bytes",
        "return ((3 >= 3)) as bytes",
        "return ((5 == 5)) as bytes",
        "return ((5 != 5)) as bytes",
        "return ((0 == 0)) as bytes",
        "return ((18446744073709551615u64 == 18446744073709551615u64)) as bytes",
        "return ((18446744073709551615u64 < 18446744073709551614u64)) as bytes",
        "return ((18446744073709551615u64 > 0)) as bytes",
    ] {
        agree4(body, &[vec![]]);
    }
}

#[test]
fn c3_literal_comparisons_mixed_widths() {
    // the widen join (cast_same_uint_width2 max) is the common width; the
    // comparison itself is numeric over the extracted u128 values
    for body in [
        "return (((2 as u8) < (3 as u16))) as bytes",
        "return (((300 as u16) < (200 as u8))) as bytes",
        "return (((300 as u16) == (300 as u32))) as bytes",
        "return (((70000 as u32) < (70000 as u64))) as bytes",
        "return (((70000 as u32) <= (70000 as u64))) as bytes",
        "return (((255 as u8) < (256 as u16))) as bytes",
    ] {
        agree4(body, &[vec![]]);
    }
}

#[test]
fn c3_literal_builtin_pairs() {
    // call arguments never pass through the frontend's combine pass —
    // these are the real double-literal arithmetic sites at the backend
    for body in [
        "return (max(2, 3) as u64) as bytes",
        "return (max(3, 2) as u64) as bytes",
        "return (max(10, 70000) as u64) as bytes",
        "return (min(70000, 3) as u64) as bytes",
        "return (min(250, 300) as u64) as bytes",
        "return (abs_diff(9, 4) as u64) as bytes",
        "return (abs_diff(4, 9) as u64) as bytes",
        "return (abs_diff(18446744073709551615u64, 1) as u64) as bytes",
        "return (div_up(5, 2) as u64) as bytes",
        "return (div_up(6, 2) as u64) as bytes",
        "return (div_up(18446744073709551615u64, 1) as u64) as bytes",
        "return (div_exact(8, 2) as u64) as bytes",
        "return (div_exact(18446744073709551615u64, 1) as u64) as bytes",
    ] {
        agree4(body, &[vec![]]);
    }
}

#[test]
fn c3_frontend_folded_shapes_still_agree() {
    // end-to-end pins: the frontend folds these to single literals before
    // the backend sees them — the four-way agreement guards the pipeline
    for body in [
        "return (2 + 3) as bytes",
        "return (2 ** 10) as bytes",
        "return ((2 as u64) ** 10) as bytes",
        "return ((2 as u8) ** 10) as bytes",
        "return ((255 as u8) + 1) as bytes",
        "return ((200 as u8) + (100 as u8)) as bytes",
        "return ((250 as u8) + (10 as u16)) as bytes",
        "return ((70000 as u64) * 2) as bytes",
    ] {
        agree4(body, &[vec![]]);
    }
}

#[test]
fn c3_fold_shrinks_the_compare_site() {
    // the folded literal compare must beat the dynamic twin by a clear
    // margin (fold: one li vs 2×operand materialization + sltu)
    let folded = "return ((2 < 3)) as bytes";
    let dynamic = "var a = byte(data, 0) as u64\nvar b = byte(data, 1) as u64\nreturn ((a < b)) as bytes";
    let (f, d) = (im_words(folded), im_words(dynamic));
    assert!(
        f + 3 <= d,
        "folded compare site {f} words must beat dynamic twin {d} words"
    );
    let folded_max = "return (max(2, 3) as u64) as bytes";
    let dynamic_max = "var a = byte(data, 0) as u64\nvar b = byte(data, 1) as u64\nreturn (max(a, b) as u64) as bytes";
    let (f, d) = (im_words(folded_max), im_words(dynamic_max));
    assert!(
        f + 3 <= d,
        "folded max site {f} words must beat dynamic twin {d} words"
    );
}

/* ================================================================== */
/* C3: fault shapes keep their runtime Arithmetic-90 face              */
/* ================================================================== */

#[test]
fn c3_fault_shapes_stay_runtime_traps() {
    // NOT compile rejects — the emitted checked form faults 90 exactly
    // like the interpreter (pinned contract, rv64_width_semantics)
    for body in [
        "return ((1 - 2)) as bytes",
        "return (((1 as u64) - (2 as u64))) as bytes",
        "return (((5 as u64) / 0)) as bytes",
        "return (((5 as u64) % 0)) as bytes",
        "return (div_exact(8, 3) as u64) as bytes",
        "return (div_up(5, 0) as u64) as bytes",
        "return (1u64 << 64) as bytes",
    ] {
        agree4(body, &[vec![]]);
    }
}

#[test]
fn c3_cast_literal_overflow_still_rejects() {
    // the literal-narrowing overflow is a fail-fast reject (probed: the
    // shared frontend rejects `300 as u8` before the backend CU errf arm
    // would — same fail-fast contract, no runtime path exists)
    let err = Rv64::try_build("return (300 as u8) as bytes")
        .err()
        .unwrap_or_else(|| panic!("(300 as u8) must reject"));
    assert!(err.contains("overflow"), "unexpected reject text: {err:?}");
}

/* ================================================================== */
/* A1: constant exponent e in [5..64] rides the checked multiply loop  */
/* ================================================================== */

/// Largest base with `base^e <= u64::MAX` (the interpreter's checked_pow
/// face boundary), computed from the semantics itself.
fn max_passing_base(e: u64) -> u64 {
    let mut best = 0u64;
    let mut b = 1u64;
    while b < 65536 {
        if u64::checked_pow(b, e as u32).is_none() {
            break;
        }
        best = b;
        b += 1;
    }
    best
}

fn two_byte_base(b: u64) -> Vec<u8> {
    vec![(b >> 8) as u8, (b & 0xFF) as u8]
}

#[test]
fn a1_const_exponent_boundary_matrix() {
    let pre = "var a = ((byte(data, 0) as u64) * 256) + (byte(data, 1) as u64)\n";
    for e in [5u64, 8, 16, 31, 32, 33, 63, 64] {
        let bmax = max_passing_base(e);
        let body = format!("{pre}return ((a ** {e}) as u64) as bytes");
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        for base in [0u64, 1, 2, 3] {
            inputs.push(two_byte_base(base));
        }
        inputs.push(two_byte_base(bmax));
        inputs.push(two_byte_base(bmax + 1)); // first overflowing base -> 90
        agree4(&body, &inputs);
    }
}

#[test]
fn a1_small_exponents_stay_linear() {
    // e <= 4 keeps the unrolled checked-mul chain — pin the linear arm's
    // value and overflow faces across the base range
    let pre = "var a = ((byte(data, 0) as u64) * 256) + (byte(data, 1) as u64)\n";
    for e in [0u64, 1, 2, 3, 4] {
        let body = format!("{pre}return ((a ** {e}) as u64) as bytes");
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        for base in [0u64, 1, 2, 3, 255, 65535] {
            inputs.push(two_byte_base(base));
        }
        inputs.push(two_byte_base(max_passing_base(e)));
        inputs.push(two_byte_base(max_passing_base(e) + 1)); // overflow -> 90
        agree4(&body, &inputs);
    }
}

#[test]
fn a1_narrow_width_tail_range_check() {
    // w = 8/16/32: the loop runs unchecked in u64 space and the tail
    // range check reproduces the narrow checked_pow face
    agree4(
        "var a = byte(data, 0)\nreturn ((a ** 5)) as bytes",
        &byte_inputs(&[0, 1, 2, 3, 4]),
    );
    agree4(
        "var a = (byte(data, 0) as u16)\nreturn ((a ** 9)) as bytes",
        &byte_inputs(&[0, 1, 2, 3, 4]),
    );
    agree4(
        "var a = (byte(data, 0) as u32)\nreturn ((a ** 17)) as bytes",
        &byte_inputs(&[0, 1, 2, 3, 4]),
    );
}

#[test]
fn a1_dynamic_exponent_unchanged() {
    let body = "var a = ((byte(data, 0) as u64) * 256) + (byte(data, 1) as u64)\n\
                var b = ((byte(data, 2) as u64) % 70)\n\
                return ((a ** b) as u64) as bytes";
    let inputs = vec![
        vec![0, 1, 64],  // 1 ** 64 -> 1 (no u32 pre-check face here)
        vec![0, 0, 10],  // 0 ** 10 -> 0
        vec![0, 2, 64],  // 2 ** 64 -> 90
        vec![0, 2, 63],  // 2 ** 63 -> fits
        vec![0, 3, 5],   // 3 ** 5
        vec![1, 200, 3], // 200 ... ** 3
    ];
    agree4(body, &inputs);
}

#[test]
fn a1_pow64_site_stays_small() {
    // identical bodies except the exponent: e=64 used to unroll to
    // 1 + 63*3 = 190 chain words where e=3 unrolls to 7; the loop form
    // makes every e in [5..64] the same ~11-word site, so the e=64 body
    // must sit within a few words of the e=3 body
    let pre = "var a = ((byte(data, 0) as u64) * 256) + (byte(data, 1) as u64)\n";
    let w64 = im_words(&format!("{pre}return ((a ** 64) as u64) as bytes"));
    let w3 = im_words(&format!("{pre}return ((a ** 3) as u64) as bytes"));
    assert!(
        w64 <= w3 + 5,
        "e=64 loop site ({w64} words) must stay within 5 words of the e=3 chain ({w3})"
    );
}
