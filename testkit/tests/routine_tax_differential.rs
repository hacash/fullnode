//! D1/D4 (fin-control.md "routine taxes") targeted differential corpus.
//!
//! Every shared-arithmetic-routine family through the REAL pipeline —
//! fitsh source → `compile_rv64im_body_kernel` (per mode) → container →
//! consensus verify (`verify_container_with_cap`, so every instruction
//! passes the consensus decoder) → calc-interpreter execution — in all
//! three lowering modes, against NATIVE Rust expectations that mirror the
//! interpreter's own u128/u256 arithmetic faces:
//!
//!   * u128 DIV / MOD / DIVUP / DIVEXACT (the shared u128 divmod routine,
//!     both call shapes), including the A6 quotient-≥-2^64 region that
//!     must NOT fault;
//!   * u128 ADDMOD / MULMOD (the shared mulmod routine + the divmod
//!     mod_pass shape);
//!   * u64/u128 MULDIV / MULDIVUP (the shared 256/128 routine: z == 0,
//!     N_hi ≥ V quotient-overflow → 90) and MULADD (the u256 workspace
//!     family that reserves the frame area WITHOUT calling a routine);
//!   * the FIN division family (bare `mul_div` = MULDIV = Floor,
//!     `mul_div_half_up` = HalfUp, `mul_div_floor` / `mul_div_ceil` —
//!     the u256 routines behind FIN3);
//!   * a ≥3-routine coexistence body (divmod + mulmod + u256/128 in one
//!     kernel) proving interleaved routine calls do not cross-clobber;
//!   * a loop body with a routine call per iteration and an EARLY RETURN
//!     after routine calls — the hazard class for any design that lets a
//!     routine call touch RA (D1's a0 link must keep the body's `ret`
//!     intact);
//!   * in-routine fault paths (divide-by-zero 90 raised from INSIDE a
//!     routine, after earlier successful routine calls): the whole kernel
//!     exits 90 — the routine's skipped restores are unobservable, the
//!     fault face and order are unchanged.
//!
//! With `--features calc-aot` (x86-64) every case is additionally run
//! through the AOT backend and compared to the interpreter engine
//! (output / abort class / exit code / cycles).

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::SpaceCap;
use vm::fitshc::{compile_rv64im_body_kernel, Rv64imBodyMeta};
use vm::ir::rv64::Rv64Mode;
use vm::lang::Tokenizer;

const BUDGET: u64 = 50_000_000;
const ARITH: u64 = 90;
const MAX128: u128 = u128::MAX;
const MAX64: u128 = u64::MAX as u128;

/* ---------------- native oracles (the interpreter's math faces) ------- */

/// (x + y) mod m without u128 overflow (the interpreter reduces first).
fn addmod(x: u128, y: u128, m: u128) -> u128 {
    let (a, b) = (x % m, y % m);
    if a >= m - b {
        a - (m - b)
    } else {
        a + b
    }
}

/// (x * y) mod m via shift-add (the interpreter's mul_mod double-and-add).
fn mulmod(x: u128, y: u128, m: u128) -> u128 {
    let (mut lhs, mut rhs, mut out) = (x % m, y % m, 0u128);
    while rhs != 0 {
        if rhs & 1 == 1 {
            out = addmod(out, lhs, m);
        }
        rhs >>= 1;
        lhs = addmod(lhs, lhs, m);
    }
    out
}

/// mul_div HalfUp rounding (the FIN default): floor + (2r >= d).
fn half_up(p: u128, d: u128) -> u128 {
    p / d + if 2 * (p % d) >= d { 1 } else { 0 }
}

fn ceil_div(p: u128, d: u128) -> u128 {
    p.div_ceil(d)
}

/* ---------------- case model ------------------------------------------ */

#[derive(Debug)]
enum Want {
    Ok(Vec<u8>),
    Exit(u64),
}

struct Case {
    id: &'static str,
    body: &'static str,
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

fn compile_container(body: &str, m: Rv64Mode) -> CalcContainer {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let bytes = compile_rv64im_body_kernel(
        "routine_tax_kernel",
        tokens,
        Some("data"),
        &[],
        &[],
        &[],
        &Rv64imBodyMeta { mode: m, ..Rv64imBodyMeta::default() },
        &SpaceCap::new(0),
    )
    .unwrap_or_else(|e| panic!("body must compile ({m:?}): {e}\n{body}"));
    let (c, _) = CalcContainer::decode(&bytes).expect("container decodes");
    c
}

#[test]
fn routine_tax_differential_three_modes() {
    let cases = build_cases();
    assert!(cases.len() >= 40, "corpus shrank: {}", cases.len());
    let mut ran = 0usize;
    for &m in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
        for c in &cases {
            run_case(c, m);
            ran += 1;
        }
    }
    println!("routine-tax differential: {} case-mode executions over {} cases", ran, cases.len());
}

fn run_case(c: &Case, m: Rv64Mode) {
    let container = compile_container(c.body, m);
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

/* ---------------- corpus ------------------------------------------------ */

const COEXIST_BODY: &str = "var s = byte(data, 0)\n\
var x = ((byte(data, 1) as u128) * 97u128) + 1u128\n\
var y = ((byte(data, 2) as u128) * 13u128) + 1u128\n\
var mm = ((byte(data, 3) as u128) % 50u128) + 50u128\n\
var q = x / y\n\
var m = mul_mod(x, y, mm)\n\
var md = mul_div((byte(data, 4) as u64), 7u64, ((byte(data, 5) as u64) % 30u64) + 3u64)\n\
var am = add_mod(x, y, mm)\n\
if s == 0 { return ((q + m) % 1000003u128) as bytes }\n\
if s == 1 { return (am + (md as u128)) as bytes }\n\
return ((q + m + am) % 1000003u128) as bytes";

fn build_cases() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();

    // ---- u128 DIV / MOD / DIVUP / DIVEXACT: byte-built operands --------
    // x = b1*251+1, y = (b2%13)+1 (y >= 1); s==4 → divide by zero.
    let body1 = "var s = byte(data, 0)\n\
var x = ((byte(data, 1) as u128) * 251u128) + 1u128\n\
var y = ((byte(data, 2) as u128) % 13u128) + 1u128\n\
if s == 0 { return (x / y) as bytes }\n\
if s == 1 { return (x % y) as bytes }\n\
if s == 2 { return div_up(x, y) as bytes }\n\
if s == 3 { return div_exact_op(x, y) as bytes }\n\
if s == 5 { return div_up(x, 0u128) as bytes }\n\
return (x / 0u128) as bytes";
    for &(b1, b2) in &[(0u8, 0u8), (1, 2), (77, 5), (200, 12), (255, 255)] {
        let (x, y) = (b1 as u128 * 251 + 1, (b2 as u128) % 13 + 1);
        let inp = |s: u8| vec![s, b1, b2];
        v.push(Case { id: "div128.div", body: body1, input: inp(0), want: Want::Ok(be(x / y)) });
        v.push(Case { id: "div128.mod", body: body1, input: inp(1), want: Want::Ok(be(x % y)) });
        v.push(Case {
            id: "div128.divup",
            body: body1,
            input: inp(2),
            want: Want::Ok(be(x.div_ceil(y))),
        });
        v.push(Case {
            id: "div128.divexact",
            body: body1,
            input: inp(3),
            want: if x % y == 0 { Want::Ok(be(x / y)) } else { Want::Exit(ARITH) },
        });
    }
    v.push(Case { id: "div128.div.zero", body: body1, input: vec![4, 9, 9], want: Want::Exit(ARITH) });
    v.push(Case { id: "div128.divup.zero", body: body1, input: vec![5, 9, 9], want: Want::Exit(ARITH) });

    // ---- u128 extremes (runtime-derived denominators; A6 region) -------
    // s==0: MAX/3 · s==1: 2^64 / 1 (quotient >= 2^64, must NOT fault) ·
    // s==2: MAX % 7 · s==3: (MAX-1)/MAX · s==4: MAX64/MAX64 · s==5: x/0.
    let body2 = "var s = byte(data, 0)\n\
var one = (byte(data, 1) as u128) * 0u128 + 1u128\n\
var r = 0u128\n\
if s == 0 { r = 340282366920938463463374607431768211455u128 / (one + 2u128) }\n\
if s == 1 { r = 18446744073709551616u128 / one }\n\
if s == 2 { r = 340282366920938463463374607431768211455u128 % (one + 6u128) }\n\
if s == 3 { r = 340282366920938463463374607431768211454u128 / 340282366920938463463374607431768211455u128 }\n\
if s == 4 { r = 18446744073709551615u128 / 18446744073709551615u128 }\n\
if s == 5 { r = 340282366920938463463374607431768211455u128 / (one - one) }\n\
return r as bytes";
    v.push(Case { id: "div128.ext.max/3", body: body2, input: vec![0, 1], want: Want::Ok(be(MAX128 / 3)) });
    v.push(Case { id: "div128.ext.2^64/1", body: body2, input: vec![1, 1], want: Want::Ok(be(1u128 << 64)) });
    v.push(Case { id: "div128.ext.max%7", body: body2, input: vec![2, 1], want: Want::Ok(be(MAX128 % 7)) });
    v.push(Case { id: "div128.ext.max-1/max", body: body2, input: vec![3, 1], want: Want::Ok(be(0)) });
    v.push(Case { id: "div128.ext.max64/max64", body: body2, input: vec![4, 1], want: Want::Ok(be(1)) });
    v.push(Case { id: "div128.ext.max/0", body: body2, input: vec![5, 1], want: Want::Exit(ARITH) });

    // ---- ADDMOD / MULMOD u128 ------------------------------------------
    // mm = (b3 % 50) + 50 ∈ [50, 99]; s==5 → z = 0 → 90.
    let body3 = "var s = byte(data, 0)\n\
var mm = ((byte(data, 3) as u128) % 50u128) + 50u128\n\
var x = 340282366920938463463374607431768211455u128\n\
var y = 170141183460469231731687303715884105727u128\n\
if s == 0 { return add_mod(x, y, mm) as bytes }\n\
if s == 1 { return mul_mod(x, y, mm) as bytes }\n\
if s == 2 { return add_mod(x, x, x) as bytes }\n\
if s == 3 { return mul_mod(x, y, (byte(data, 3) as u128) * 0u128 + 1u128) as bytes }\n\
if s == 4 { return mul_mod(x, 12345u64, mm) as bytes }\n\
return add_mod(x, y, 0u128) as bytes";
    for b3 in [0u8, 7, 49, 99] {
        let mm = (b3 as u128) % 50 + 50;
        let x = MAX128;
        let y = (1u128 << 127) - 1;
        v.push(Case {
            id: "addmod128",
            body: body3,
            input: vec![0, 0, 0, b3],
            want: Want::Ok(be(addmod(x, y, mm))),
        });
        v.push(Case {
            id: "mulmod128",
            body: body3,
            input: vec![1, 0, 0, b3],
            want: Want::Ok(be(mulmod(x, y, mm))),
        });
        v.push(Case {
            id: "mulmod128.mixed",
            body: body3,
            input: vec![4, 0, 0, b3],
            want: Want::Ok(be(mulmod(x, 12345, mm))),
        });
    }
    v.push(Case { id: "addmod128.self", body: body3, input: vec![2, 0, 0, 3], want: Want::Ok(be(0)) });
    v.push(Case { id: "mulmod128.mod1", body: body3, input: vec![3, 0, 0, 9], want: Want::Ok(be(0)) });
    v.push(Case { id: "addmod128.z0", body: body3, input: vec![5, 0, 0, 3], want: Want::Exit(ARITH) });

    // ---- MULDIV / MULDIVUP (u64 + u128 common widths; 256/128 routine) --
    // a = b1, b = b2+1, c = b3*7919+1; s==4 → z = 0 → 90; s==5 →
    // quotient-overflow face (product hi >= 1) → 90.
    let body4 = "var s = byte(data, 0)\n\
var a = byte(data, 1) as u64\n\
var b = (byte(data, 2) as u64) + 1u64\n\
var c = (byte(data, 3) as u64) * 7919u64 + 1u64\n\
if s == 0 { return mul_div(a, b, c) as bytes }\n\
if s == 1 { return mul_div_up(a, b, c) as bytes }\n\
if s == 2 { return mul_div(18446744073709551615u128, 18446744073709551615u128, ((byte(data, 1) as u128) % 11u128) + 3u128) as bytes }\n\
if s == 3 { return mul_div_up(18446744073709551615u128, 18446744073709551615u128, ((byte(data, 1) as u128) % 11u128) + 3u128) as bytes }\n\
if s == 4 { return mul_div(a, b, 0u64) as bytes }\n\
return mul_div(340282366920938463463374607431768211455u128, 340282366920938463463374607431768211455u128, 1u128) as bytes";
    for &(b1, b2, b3) in &[(0u8, 0u8, 0u8), (3, 2, 100), (255, 254, 255), (17, 40, 9)] {
        let (a, b, c) = (b1 as u128, b2 as u128 + 1, b3 as u128 * 7919 + 1);
        let p = a * b; // u64×u64 always fits u128
        v.push(Case {
            id: "muldiv64.floor",
            body: body4,
            input: vec![0, b1, b2, b3],
            want: Want::Ok(be8((p / c) as u64)),
        });
        v.push(Case {
            id: "muldiv64.ceil",
            body: body4,
            input: vec![1, b1, b2, b3],
            want: Want::Ok(be8(ceil_div(p, c) as u64)),
        });
    }
    for b1 in [0u8, 5, 10, 255] {
        let d = (b1 as u128) % 11 + 3;
        let p = MAX64 * MAX64; // (2^64-1)^2 < 2^128 — fits the u128 result
        v.push(Case {
            id: "muldiv128.floor",
            body: body4,
            input: vec![2, b1, 0, 0],
            want: Want::Ok(be(p / d)),
        });
        v.push(Case {
            id: "muldiv128.ceil",
            body: body4,
            input: vec![3, b1, 0, 0],
            want: Want::Ok(be(ceil_div(p, d))),
        });
    }
    v.push(Case { id: "muldiv.z0", body: body4, input: vec![4, 3, 2, 0], want: Want::Exit(ARITH) });
    v.push(Case {
        id: "muldiv.qovf",
        body: body4,
        input: vec![5, 0, 0, 0],
        want: Want::Exit(ARITH),
    });

    // ---- MULADD (u256 workspace, NO division routine call) -------------
    // The 256-bit intermediate must fit u128 AND the result range-checks
    // to the common width: u64 rows fault 90 unless the final value fits
    // u64 (the interpreter's cast_uint_result3 face).
    let body5 = "var s = byte(data, 0)\n\
if s == 0 { return mul_add(4u64, 5u64, 3u64) as bytes }\n\
if s == 1 { return mul_add(18446744073709551615u64, 18446744073709551615u64, 1u64) as bytes }\n\
if s == 2 { return mul_add(340282366920938463463374607431768211455u128, 2u128, 1u128) as bytes }\n\
return mul_add(340282366920938463463374607431768211455u128, 2u128, 3u128) as bytes";
    v.push(Case { id: "muladd.u64.fit", body: body5, input: vec![0], want: Want::Ok(be8(23)) });
    v.push(Case { id: "muladd.u64.cast-ovf", body: body5, input: vec![1], want: Want::Exit(ARITH) });
    v.push(Case { id: "muladd.u128.ovf", body: body5, input: vec![2], want: Want::Exit(ARITH) });
    v.push(Case { id: "muladd.u128.ovf2", body: body5, input: vec![3], want: Want::Exit(ARITH) });

    // ---- FIN division family (FIN3 behind the u256 routines) -----------
    // mul_div = HalfUp (the default); _floor/_ceil explicit policies.
    let body6 = "var s = byte(data, 0)\n\
var a = byte(data, 1) as u64\n\
if s == 0 { return mul_div(10u64 * a, 4u64, 3u64) as bytes }\n\
if s == 1 { return mul_div_floor(10u64 * a, 4u64, 3u64) as bytes }\n\
if s == 2 { return mul_div_ceil(10u64 * a, 4u64, 3u64) as bytes }\n\
if s == 3 { return mul_div(4294967295u64, 4294967295u64, ((byte(data, 1) as u64) % 11u64) + 3u64) as bytes }\n\
if s == 4 { return mul_div(18446744073709551615u64, 18446744073709551615u64, 1u64) as bytes }\n\
if s == 6 { return mul_div_half_up(10u64 * a, 4u64, 3u64) as bytes }\n\
return mul_div_floor(10u64 * a, 4u64, 0u64) as bytes";
    for a in [0u8, 1, 2, 3, 7, 255] {
        let p = 40u128 * a as u128; // (10*a) * 4
        v.push(Case {
            id: "fin3.muldiv-floor",
            body: body6,
            input: vec![0, a],
            want: Want::Ok(be8((p / 3) as u64)),
        });
        v.push(Case {
            id: "fin3.floor",
            body: body6,
            input: vec![1, a],
            want: Want::Ok(be8((p / 3) as u64)),
        });
        v.push(Case {
            id: "fin3.ceil",
            body: body6,
            input: vec![2, a],
            want: Want::Ok(be8(ceil_div(p, 3) as u64)),
        });
        v.push(Case {
            id: "fin3.halfup",
            body: body6,
            input: vec![6, a],
            want: Want::Ok(be8(half_up(p, 3) as u64)),
        });
    }
    for a in [0u8, 5, 10] {
        let d = (a as u128) % 11 + 3;
        // (2^32-1)^2 / d fits the u64 result width; bare mul_div = MULDIV
        // = Floor (the interpreter's muldiv_checked), NOT HalfUp
        v.push(Case {
            id: "fin3.u64wide",
            body: body6,
            input: vec![3, a],
            want: Want::Ok(be8((((MAX64 >> 32) * (MAX64 >> 32)) / d) as u64)),
        });
    }
    v.push(Case { id: "fin3.u64.cast-ovf", body: body6, input: vec![4, 0], want: Want::Exit(ARITH) });
    v.push(Case { id: "fin3.z0", body: body6, input: vec![5, 3], want: Want::Exit(ARITH) });

    // ---- ≥3-routine coexistence (divmod + mulmod + 256/128 + addmod) ---
    for input in [
        vec![0u8, 5, 3, 7, 11, 2],
        vec![1, 200, 9, 49, 40, 29],
        vec![2, 42, 1, 0, 255, 3],
        vec![2, 128, 128, 99, 77, 66],
    ] {
        let b = |i: usize| input[i] as u128;
        let (x, y) = (b(1) * 97 + 1, b(2) * 13 + 1);
        let mm = b(3) % 50 + 50;
        let md = (b(4) * 7) / (b(5) % 30 + 3); // bare mul_div = floor
        let want = match input[0] {
            0 => be((x / y + mulmod(x, y, mm)) % 1000003),
            1 => be(addmod(x, y, mm) + md),
            _ => be((x / y + mulmod(x, y, mm) + addmod(x, y, mm)) % 1000003),
        };
        v.push(Case { id: "coexist", body: COEXIST_BODY, input, want: Want::Ok(want) });
    }

    // ---- loop + early return after routine calls (RA preservation) -----
    let body8 = "var n = (byte(data, 0) as u64) % 40u64\n\
var i = 0u64\n\
var acc = 1u128\n\
while i < n {\n\
acc = acc + (acc / 3u128) + 1u128\n\
if acc > 1000000000000u128 { return acc as bytes }\n\
i += 1u64\n\
}\n\
return acc as bytes";
    for n in [0u8, 1, 5, 20, 39, 255] {
        let n = (n as u64) % 40;
        let mut acc = 1u128;
        let mut early = None;
        for _ in 0..n {
            acc = acc + (acc / 3) + 1;
            if acc > 1_000_000_000_000 {
                early = Some(acc);
                break;
            }
        }
        let want = early.unwrap_or(acc);
        v.push(Case { id: "loop.ret", body: body8, input: vec![n as u8], want: Want::Ok(be(want)) });
    }

    // ---- in-routine fault AFTER successful routine calls ----------------
    // q = x/5 and m = x%4 run the divmod routine successfully; the final
    // division by (m - m) = 0 traps 90 from INSIDE the routine — the whole
    // kernel exits, the skipped restores are unobservable.
    let body9 = "var x = ((byte(data, 0) as u128) * 7u128) + 3u128\n\
var q = x / 5u128\n\
var m = x % 4u128\n\
var bad = q / (m - m)\n\
return (q + m) as bytes";
    for b0 in [0u8, 1, 9, 255] {
        v.push(Case { id: "fault.after.ok.calls", body: body9, input: vec![b0], want: Want::Exit(ARITH) });
    }

    v
}
