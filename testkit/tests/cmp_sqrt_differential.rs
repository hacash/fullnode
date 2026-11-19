//! D8 (fin-control.md) + A8 (arith.md) targeted differential corpus — the
//! same pipeline discipline as `routine_tax_differential.rs`: fitsh source →
//! `compile_rv64im_body_kernel` (per mode) → container → consensus verify
//! (`verify_container_with_cap`, so every emitted instruction passes the
//! consensus decoder) → calc-interpreter execution in all three lowering
//! modes, against NATIVE Rust expectations.
//!
//! D8 — the u128 compare branch fusion (`emit_u128_cmp_branch`):
//!   * conditional position (`if (a OP b) … else …`, both outcome shapes)
//!     for all six directions (LT/GT/LE/GE/EQ/NEQ) over the u128 edge
//!     neighborhood: 0 / 1 / 2^64−1 / 2^64 / 2^64+1 / all-F / MAX−1 / MAX,
//!     cross-half-carry pairs and mixed u64×u128 widths;
//!   * the same comparisons in VALUE position (`(a OP b) as u64`, the
//!     unchanged `emit_u128_cmp` materializing path — regression cover);
//!   * a fused compare as a `while` backedge condition (loop-carried);
//!   * the exactly-one-Nil conditional-compare face (Arithmetic 90) — the
//!     guard runs before the pops in both the old and the fused shape, so
//!     the face must be unchanged;
//!   * NOT in scope: the survey's `emit_u128_cmp` sltu direct-write —
//!     proven unsound (the positional `d` handback can recycle a popped
//!     operand register), see `emit_u128_cmp_branch`'s doc comment.
//!
//! A8 — the sqrt routine-ization gate (second commit):
//!   * single-site sqrt / sqrt_up bodies (gate OFF → the inline form must
//!     stay byte-identical in behavior),
//!   * 2/3/5-site bodies (gate ON → the routine is emitted once per
//!     variant), u64 and u128 domains, widths 8/16/32/64,
//!   * edge inputs: 0 / 1 / 2^k−1 / 2^k / perfect squares ±1 / MAX²
//!     neighborhood, SQRTUP's ceil faces (2^128−1 → 2^64 wrap),
//!   * the w<64 range-check 90 face and the nil-72 face staying at the
//!     site (fault order per face preserved).
//!
//! With `--features calc-aot` (x86-64) every case is additionally run
//! through the AOT backend and compared to the interpreter engine
//! (output / abort class / exit code).

use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::SpaceCap;
use vm::fitshc::{compile_rv64im_body_kernel, Rv64imBodyMeta};
use vm::ir::rv64::Rv64Mode;
use vm::lang::Tokenizer;

const BUDGET: u64 = 50_000_000;
const ARITH: u64 = 90;
const EXTRACT: u64 = 71;
const MAX128: u128 = u128::MAX;
const MAX64: u128 = u64::MAX as u128;

/* ---------------- case model ------------------------------------------ */

#[derive(Debug, Clone)]
enum Want {
    Ok(Vec<u8>),
    Exit(u64),
}

struct Case {
    id: &'static str,
    body: String,
    input: Vec<u8>,
    want: Want,
}

fn be(v: u128) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be8(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/* ---------------- engine mounting ------------------------------------- */

fn compile_container(body: &str, m: Rv64Mode) -> vm::calc::container::CalcContainer {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let bytes = compile_rv64im_body_kernel(
        "cmp_sqrt_kernel",
        tokens,
        Some("data"),
        &[],
        &[],
        &[],
        &Rv64imBodyMeta { mode: m, ..Rv64imBodyMeta::default() },
        &SpaceCap::new(0),
    )
    .unwrap_or_else(|e| panic!("body must compile ({m:?}): {e}\n{body}"));
    let (c, _) = vm::calc::container::CalcContainer::decode(&bytes).expect("container decodes");
    c
}

fn run_case(c: &Case, m: Rv64Mode) {
    let container = compile_container(&c.body, m);
    let module = verify_container_with_cap(&container, &SpaceCap::new(0))
        .unwrap_or_else(|e| panic!("[{} {m:?}] consensus verify rejected: {e}", c.id));
    let (res, _regs, _mem) = interp::run_lockstep(&module, &container, &c.input, BUDGET, None);
    let got = match res {
        Ok(r) => Ok(r.output),
        Err((CalcAbort::GuestExit(code), _)) => Err(code),
        Err((a, _)) => panic!("[{} {m:?}] unexpected abort {a:?}", c.id),
    };
    match (&c.want, got) {
        (Want::Ok(w), Ok(o)) => assert_eq!(
            o, *w,
            "[{} {m:?}] output mismatch (input 0x{}): got 0x{} want 0x{}",
            c.id,
            hex::encode(&c.input),
            hex::encode(&o),
            hex::encode(w)
        ),
        (Want::Exit(e), Err(g)) => assert_eq!(
            g, *e,
            "[{} {m:?}] exit mismatch (input 0x{}): got {g} want {e}",
            c.id,
            hex::encode(&c.input)
        ),
        (want, got) => panic!(
            "[{} {m:?}] face mismatch (input 0x{}): got {got:?} want {want:?}",
            c.id,
            hex::encode(&c.input)
        ),
    }

    // AOT parity (feature-gated backend): same lowered code, both engines.
    #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
    {
        let aot = vm::calc::aot::aot_compile(&module)
            .unwrap_or_else(|| panic!("[{} {m:?}] aot compile refused", c.id));
        let (out, _ar, _am) = aot.run_with_state(&module, &container, &c.input, BUDGET);
        match (&c.want, out.abort.is_none(), out.exit_code) {
            (Want::Ok(w), true, 0) => assert_eq!(
                out.output.as_deref(),
                Some(w.as_slice()),
                "[{} {m:?}] aot output mismatch",
                c.id
            ),
            (Want::Exit(e), false, g) => assert_eq!(g, *e, "[{} {m:?}] aot exit mismatch", c.id),
            _ => panic!("[{} {m:?}] aot face mismatch: {:?}", c.id, out.abort),
        }
    }
}

/* ====================================================================== */
/* D8: the u128 compare branch fusion                                     */
/* ====================================================================== */

/// Conditional-position body: `if (a OP b) {7} else {9}` — the taken and
/// fall-through shapes of the fused branch, one body per (op, polarity).
fn cond_body(op: &str, a: u128, b: u128, swap_out: bool) -> (String, u8) {
    let holds = match op {
        "<" => a < b,
        ">" => a > b,
        "<=" => a <= b,
        ">=" => a >= b,
        "==" => a == b,
        "!=" => a != b,
        _ => unreachable!(),
    };
    let (t, f) = if swap_out { (9u8, 7u8) } else { (7u8, 9u8) };
    let want = if holds { t } else { f };
    let body = format!(
        "var a = {a}u128\nvar b = {b}u128\nvar r = 0u64\nif (a {op} b) {{ r = {t}u64 }} else {{ r = {f}u64 }}\nreturn r as bytes"
    );
    (body, want)
}

/// Value-position body: all six ordered/equality directions materialized
/// and bit-packed (the unchanged `emit_u128_cmp` path).
fn value_body(a: u128, b: u128) -> (String, u64) {
    let w = (a < b) as u64
        + (a > b) as u64 * 10
        + (a <= b) as u64 * 100
        + (a >= b) as u64 * 1000
        + (a == b) as u64 * 10000
        + (a != b) as u64 * 100000;
    let body = format!(
        "var a = {a}u128\nvar b = {b}u128\nvar lt = (a < b) as u64\nvar gt = (a > b) as u64\nvar le = (a <= b) as u64\nvar ge = (a >= b) as u64\nvar eq = (a == b) as u64\nvar ne = (a != b) as u64\nreturn (lt + gt * 10u64 + le * 100u64 + ge * 1000u64 + eq * 10000u64 + ne * 100000u64) as bytes"
    );
    (body, w)
}

/// The u128 edge neighborhood: cross-half boundaries around 2^64 and the
/// all-F corners (each operand register pair's lo/hi halves get both the
/// compare-decides and the-fall-through roles).
const EDGE_PAIRS: [(u128, u128); 14] = [
    (0, 0),
    (0, 1),
    (1, 0),
    (1, 1),
    (MAX64, MAX64),
    (MAX64, MAX64 + 1),  // 2^64-1 vs 2^64: lo all-F vs 0, hi 0 vs 1
    (MAX64 + 1, MAX64),  // 2^64 vs 2^64-1
    (MAX64 + 1, MAX64 + 1),
    (MAX64 + 1, MAX64 + 2),
    (MAX128, MAX128),
    (MAX128, MAX128 - 1),
    (MAX128 - 1, MAX128),
    (MAX128, 0),
    (0, MAX128),
];

#[test]
fn d8_u128_cmp_cond_position_fusion_three_modes() {
    let ops = ["<", ">", "<=", ">=", "==", "!="];
    let mut cases: Vec<Case> = Vec::new();
    for &(a, b) in &EDGE_PAIRS {
        for &op in &ops {
            for swap_out in [false, true] {
                let (body, want) = cond_body(op, a, b, swap_out);
                cases.push(Case {
                    id: "d8.cond",
                    body,
                    input: vec![],
                    want: Want::Ok(be8(want as u64)),
                });
            }
        }
    }
    // mixed u64 × u128 widths: the narrower side zero-extends into its pair
    // — the same fused arm decides. u64 2^64-1 (all-F lo, zero hi) against
    // the u128 boundary values.
    let mixed = [
        ("18446744073709551615u64", MAX64),
        ("0u64", 0),
        ("1u64", 1),
        ("18446744073709551616u128", MAX64 + 1),
    ];
    for &(lit_b, b) in &mixed {
        for &op in &ops {
            let holds = match op {
                "<" => MAX64 < b,
                ">" => MAX64 > b,
                "<=" => MAX64 <= b,
                ">=" => MAX64 >= b,
                "==" => MAX64 == b,
                "!=" => MAX64 != b,
                _ => unreachable!(),
            };
            let body = format!(
                "var a = 18446744073709551615u64\nvar b = {lit_b}\nvar r = 0u64\nif (a {op} b) {{ r = 7u64 }} else {{ r = 9u64 }}\nreturn r as bytes"
            );
            cases.push(Case {
                id: "d8.cond.mixed",
                body,
                input: vec![],
                want: Want::Ok(be8(if holds { 7 } else { 9 })),
            });
        }
    }
    assert!(cases.len() >= 150, "corpus shrank: {}", cases.len());
    let mut ran = 0usize;
    for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        for c in &cases {
            run_case(c, m);
            ran += 1;
        }
    }
    println!("d8 cond differential: {ran} case-mode executions over {} cases", cases.len());
}

#[test]
fn d8_u128_cmp_value_position_three_modes() {
    let mut cases: Vec<Case> = Vec::new();
    for &(a, b) in &EDGE_PAIRS {
        let (body, w) = value_body(a, b);
        cases.push(Case { id: "d8.value", body, input: vec![], want: Want::Ok(be8(w)) });
    }
    for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        for c in &cases {
            run_case(c, m);
        }
    }
}

#[test]
fn d8_u128_cmp_fused_while_backedge_three_modes() {
    // loop-carried fused compare: `while (a < b)` with a growing a — the
    // fused branch is the backedge condition, taken and exhausted both.
    // The interpreter's u128 mul is checked: an overflow step aborts 90
    // mid-loop (the oracle mirrors that face).
    let body = "var a = (byte(data, 0) as u128) * 18446744073709551616u128 + (byte(data, 1) as u128) * 1000003u128\nvar b = (byte(data, 2) as u128) * 18446744073709551616u128\nvar n = 0u64\nwhile (a < b) {\n    a = a * 3u128 + 1u128\n    n += 1u64\n    if n == 200u64 { break }\n}\nreturn n as bytes";
    // (hi, lo) seed a; data[2] seeds b's hi half: 1 → a crosses 2^64 and
    // the loop exits normally; 0 → the loop never runs (a < 0 false).
    for &(hi, lo, bh) in &[(0u8, 0u8, 1u8), (1, 0, 0), (0, 0, 0), (1, 255, 1)] {
        let mut a = (hi as u128) * (1u128 << 64) + (lo as u128) * 1000003u128;
        let b = (bh as u128) * (1u128 << 64);
        let mut n = 0u64;
        let mut want = {
            let mut out = None;
            loop {
                if a >= b || n == 200 {
                    out = Some(Want::Ok(be8(n)));
                    break;
                }
                match a.checked_mul(3).and_then(|v| v.checked_add(1)) {
                    None => {
                        out = Some(Want::Exit(ARITH));
                        break;
                    }
                    Some(v) => a = v,
                }
                n += 1;
            }
            out.unwrap()
        };
        for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            run_case(
                &Case { id: "d8.while", body: body.to_string(), input: vec![hi, lo, bh], want: want.clone() },
                m,
            );
        }
    }
}

#[test]
fn d8_u128_cmp_nil_face_preserved_three_modes() {
    // Nil-mixing compare body: the out-of-range byte read faults Cast 71
    // before the compare itself on every engine (the interpreter's
    // extract_u128 face) — the compare-level Nil guard
    // (`nil_guard_u128_cmp`, Arithmetic 90) sits behind it and is emitted
    // IDENTICALLY in the fused shape (same call, still before the pops),
    // so the observable face chain of this body — 71 — must not move.
    let body = "var a = byte(data, 64) as u128\nvar b = 5u128\nvar r = 0u64\nif (a < b) { r = 7u64 } else { r = 9u64 }\nreturn r as bytes";
    for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        run_case(
            &Case { id: "d8.nil.lt", body: body.to_string(), input: vec![1, 2], want: Want::Exit(EXTRACT) },
            m,
        );
    }
}
