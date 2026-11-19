//! A8 (docs.loc/fitsh-rv64/survey/arith.md): the shared sqrt routines' gate
//! and differential corpus.
//!
//! SQRT/SQRTUP have no fitsh source syntax (no irfn entry — the frontend
//! cannot produce them), so like rv64_misc_ops' T1 channel the corpus is
//! HAND-BUILT IR: the same `IRNodeArray` drives BOTH channels — the
//! consensus interpreter (its own `codegen` bytecode) and the rv64
//! lowering in all three modes (`lower_body_mode_arena` → container →
//! CalcFnObj) — so semantics are identical by construction and every
//! emitted instruction passes the consensus decoder at pack/verify time.
//!
//! Coverage:
//! * single-site sqrt / sqrt_up (gate OFF — the inline form must keep the
//!   exact interpreter face), u8/u32/u64/u128 widths, the edge matrix
//!   (0 / 1 / 2 / 3 / 2^32±1 / 2^62 / 2^64−1 / 2^64+1 / all-F / MAX128 /
//!   MAX128−1 / MAX128−2^64+1 …) including the SQRTUP ceil face at
//!   MAX128 (ceil = 2^64 — the result-pair carry that wraps the low
//!   half);
//! * multi-site bodies (N = 2/3/5 sites of one variant): the routine is
//!   emitted ONCE per gated variant and every additional site costs only
//!   its staging shape (sd + jal + ld [+ one result ADD]) instead of the
//!   full inline body — pinned by per-site instruction marginals;
//! * mixed-variant bodies (3 floor + 2 ceil): exactly the ≥-threshold
//!   variant gates on;
//! * the fault faces that stay at the site: the operand's CastParamFail
//!   72 (may-Nil byte operand AND the bytes-operand compile reject) and
//!   the 90 face of a faulting operand evaluation (div-by-zero inside the
//!   operand) — fault order preserved (all before any routine call);
//! * determinism: two lowerings of the same body are byte-identical.
//!
//! FIXED DIVERGENCE (founding era, disclosed by the D8+A8 acceptance
//! report): the inline u64-domain ceil tail used to consume the loop's
//! LAST square (sc1), which is stale by one step whenever the final bit
//! was cleared — rv64 sqrt_up returned floor+1 where the interpreter
//! returns floor, exactly for the EVEN perfect squares (x = f² with f
//! even: 0→1, 4→3, 16→5, 2^62→2^31+1; odd f keeps its last bit so
//! sc1 = f² was fresh and those agreed). The u128 variant always
//! recomputed r·r and was correct. The fix recomputes the square fresh in
//! the ceil tail (`mul d,d`, the u128 variant's shape) in BOTH the inline
//! emitter and the A8 routine body; the former `ceil64_known_div`
//! stale-square pins are FLIPPED to three-way agreement
//! (interp == Im == Imc == ImcRemap) — see the even-perfect-square corpus
//! test below.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, InterpFault, Rng};
use vm::ir::rv64::{lower_body_mode_arena, Rv64Lowered, Rv64Mode, ARENA_SIZE_MAX};
use vm::ir::{IRNode, IRNodeArray, IRNodeDouble, IRNodeLeaf, IRNodeParam1, IRNodeParam1Single, IRNodeSingle};
use vm::Bytecode;
use vm::calc::{CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/* ---------------- node builders --------------------------------------- */

fn get_node(para: u8) -> Box<dyn IRNode> {
    Box::new(IRNodeParam1 {
        hrtv: true,
        inst: Bytecode::GET,
        para,
        text: String::new(),
    })
}

fn lit(k: u8) -> Box<dyn IRNode> {
    Box::new(IRNodeLeaf {
        hrtv: true,
        inst: match k {
            0 => Bytecode::P0,
            1 => Bytecode::P1,
            2 => Bytecode::P2,
            _ => Bytecode::P3,
        },
        text: String::new(),
    })
}

fn add(a: Box<dyn IRNode>, b: Box<dyn IRNode>) -> Box<dyn IRNode> {
    Box::new(IRNodeDouble { hrtv: true, inst: Bytecode::ADD, subx: a, suby: b })
}

fn sqrt_node(inst: Bytecode, operand: Box<dyn IRNode>) -> Box<dyn IRNode> {
    Box::new(IRNodeSingle { hrtv: true, inst, subx: operand })
}

/// left-associated ADD tree over k identical sqrt sites (result-only
/// combination — the per-site instruction cost stays attributable to the
/// site's own staging shape)
fn sqrt_chain(inst: Bytecode, slot: u8, k: usize) -> Box<dyn IRNode> {
    let mut acc = sqrt_node(inst, get_node(slot));
    for _ in 1..k {
        acc = add(acc, sqrt_node(inst, get_node(slot)));
    }
    acc
}

/* ---------------- templates ------------------------------------------- */

/// slots: 0 = data, 1 = a (u64 from the first 8 input bytes), 2 = i (the
/// loop counter), 3 = r (u64)
const TPL64: &str = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nvar r = 0u64\nreturn (r as u64) as bytes";

/// slots: 0 = data, 1 = a, 2 = i, 3 = x (u128), 4 = r (u128 → bytes)
fn tpl128(x_expr: &str) -> String {
    format!(
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 {{ a = a * 256 + byte(data, i)\ni += 1 }}\nvar x = {x_expr}\nvar r = 0u128\nreturn r as bytes"
    )
}

const X_A: &str = "(a as u128)";
const X_B: &str = "(a as u128) * 18446744073709551616u128 + (a as u128)";
const X_C: &str = "340282366920938463463374607431768211455u128 - (a as u128)";

/// narrow-width operand casts (the result keeps the input width)
fn cast_node(inst: Bytecode, slot: u8) -> Box<dyn IRNode> {
    Box::new(IRNodeSingle { hrtv: true, inst, subx: get_node(slot) })
}

fn swap_r(repl: Box<dyn IRNode>, tpl: &str, r_slot: u8) -> IRNodeArray {
    let (mut ir, _) = c3lib::frontend(tpl).expect("template frontend");
    let mut hit = false;
    for i in 0..ir.subs.len() {
        if let Some(p) = ir.subs[i].as_any().downcast_ref::<IRNodeParam1Single>() {
            if p.inst == Bytecode::PUT && p.para == r_slot {
                let np = IRNodeParam1Single {
                    hrtv: p.hrtv,
                    inst: p.inst,
                    para: p.para,
                    subx: repl,
                };
                ir.subs[i] = Box::new(np);
                hit = true;
                break;
            }
        }
    }
    assert!(hit, "template PUT {r_slot} not found");
    ir
}

/* ---------------- engines --------------------------------------------- */

fn run_ir_interp(ir: &IRNodeArray, input: &[u8]) -> Result<Vec<u8>, InterpFault> {
    let codes = ir.codegen().expect("ir codegen");
    Interp::from_body_codes(codes).run(input)
}

fn lower_ir(ir: &IRNodeArray, mode: Rv64Mode) -> Rv64Lowered {
    lower_body_mode_arena(ir, true, mode, ARENA_SIZE_MAX)
        .unwrap_or_else(|e| panic!("rv64 lower {mode:?}: {e}"))
}

fn run_ir_rv64(ir: &IRNodeArray, input: &[u8]) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
    let cap = SpaceCap::new(0);
    let mut out = Vec::new();
    for &mode in MODES.iter() {
        let lo = lower_ir(ir, mode);
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let c = lo
            .pack_segments_profile(&cap, 64, 4800, 16, profile)
            .expect("pack");
        let fno = CalcFnObj::build(c, &cap, CalcHostConfig::new()).expect("build");
        let ex = fno.run(input, CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None });
        out.push((mode, ex.result.map_err(|_| ex.exit_code)));
    }
    out
}

fn agree_ir(ir: &IRNodeArray, input: &[u8], label: &str) {
    let want = run_ir_interp(ir, input);
    for (mode, got) in run_ir_rv64(ir, input) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{label} [{mode:?}] input {input:?}"),
            (Err(w), Err(code)) => {
                assert_eq!(w.code as u64, *code, "{label} [{mode:?}] input {input:?}")
            }
            _ => panic!("{label} [{mode:?}] input {input:?}: interp={want:?} rv64={got:?}"),
        }
    }
}

fn insts(ir: &IRNodeArray, mode: Rv64Mode) -> i64 {
    lower_ir(ir, mode).stats.insts as i64
}

fn u64_word(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be(v: u128) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be8(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// isqrt without u128::isqrt (deterministic bit scan — the same algorithm
/// family the emitters use)
fn isqrt(x: u128) -> u128 {
    if x == 0 {
        return 0;
    }
    let mut bit = 1u128 << 126;
    while bit > x {
        bit >>= 2;
    }
    let (mut r, mut t) = (0u128, x);
    while bit != 0 {
        if t >= r + bit {
            t -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r
}

fn ceilsqrt(x: u128) -> u128 {
    let f = isqrt(x);
    if f * f < x {
        f + 1
    } else {
        f
    }
}

/* ------------- the ceil64 machine models (stale vs fixed) ------------- */

/// The founding-era inline ceil tail's stale-square model: ceil =
/// f + (x − (f|1)² != 0) with wrapping — the loop's last square belongs to
/// the TESTED value f|1, not the final f. This is what rv64 emitted BEFORE
/// the sqrtup-fix (gate ON and OFF alike — the routine replicated the
/// inline body bug-for-bug); kept as the historical record the model test
/// below proves the divergence set against.
fn ceil64_stale_model(x: u64) -> u64 {
    let f = isqrt(x as u128) as u64;
    let stale = (f | 1) * (f | 1);
    f + u64::from(x.wrapping_sub(stale) != 0)
}

/// The emitter's u64 isqrt LOOP, modeled bit-exactly: 32 iterations from
/// bit = 2^31, r |= bit, square = r·r (u64-exact: r < 2^32 so no wrap),
/// keep the bit iff n >= square. Returns (r, last_square) — last_square
/// is the sc1 register state after the loop, the value the founding-era
/// tail consumed.
fn isqrt64_machine(x: u64) -> (u64, u64) {
    let (mut r, mut bit, mut sq) = (0u64, 1u64 << 31, 0u64);
    while bit != 0 {
        r |= bit;
        sq = r.wrapping_mul(r);
        if x < sq {
            r ^= bit;
        }
        bit >>= 1;
    }
    (r, sq)
}

/// The FIXED ceil tail: recompute r·r fresh (the u128 variant's shape —
/// `mul d,d` replacing the consumption of the possibly-stale sc1), then
/// ceil = r + (x − r·r != 0). This matches the interpreter's sqrt_up
/// semantics (interpreter/instruction.rs `sqrt_up_checked`: n ≤ 1 → n;
/// f·f == n → f; else f+1, i.e. the minimal r with r·r ≥ x).
fn sqrt_up64_fixed_model(x: u64) -> u64 {
    let (r, _) = isqrt64_machine(x);
    let sq = r.wrapping_mul(r);
    r + u64::from(x.wrapping_sub(sq) != 0)
}

/// MODEL TEST (pre-emitter-fix gate): the FIXED tail equals the
/// interpreter's sqrt_up semantics for EVERY u64 input (boundary matrix +
/// ≥10^4 seeded randoms), and the stale model differs from it EXACTLY on
/// the even perfect squares (f even, x = f², 0 included).
#[test]
fn model_sqrt_up64_fixed_matches_interp_semantics() {
    // boundary matrix: small squares, every 2^k neighborhood, the squares
    // of every power-of-two root (even and odd roots both), u64 corners
    let mut xs: Vec<u64> = vec![
        0, 1, 2, 3, 4, 8, 9, 15, 16, 17, 24, 25, 26, 35, 36, 37, 48, 49, 63, 64, 65, 80, 81,
        u64::MAX, u64::MAX - 1, u32::MAX as u64, u32::MAX as u64 - 1, u32::MAX as u64 + 1,
        4_611_686_018_427_387_904, // 2^62 = (2^31)²
        4_611_686_018_427_387_903, // 2^62 − 1
        4_611_686_018_427_387_905, // 2^62 + 1
    ];
    for k in 0..64u32 {
        let p = 1u64.wrapping_shl(k);
        xs.push(p.wrapping_sub(1));
        xs.push(p);
        xs.push(p.wrapping_add(1));
        // the square of 2^ceil(k/2) and its ±1 — even-root squares are the
        // historical divergence set
        for root in [1u64 << (k / 2), 1u64 << ((k + 1) / 2)] {
            let sq = root.wrapping_mul(root);
            xs.push(sq.wrapping_sub(1));
            xs.push(sq);
            xs.push(sq.wrapping_add(1));
        }
        // odd roots too (the agreeing side)
        let r_odd = (1u64 << (k / 2)) | 1;
        let sq = r_odd.wrapping_mul(r_odd);
        xs.push(sq);
        xs.push(sq.wrapping_add(1));
    }
    // deterministic randoms (≥10^4 total cases)
    let mut rng = Rng::new(0x5C1A_11E5);
    for _ in 0..12_000 {
        xs.push(rng.next());
    }
    assert!(xs.len() >= 10_000, "model corpus too small: {}", xs.len());
    for &x in &xs {
        assert_eq!(
            sqrt_up64_fixed_model(x),
            ceilsqrt(x as u128) as u64,
            "fixed model vs interp sqrt_up({x})"
        );
        let f = isqrt(x as u128) as u64;
        let even_sq = f % 2 == 0 && x == f.wrapping_mul(f);
        assert_eq!(
            ceil64_stale_model(x) != sqrt_up64_fixed_model(x),
            even_sq,
            "stale-vs-fixed divergence set must be exactly the even perfect squares (x={x})"
        );
    }
}

/* ================= single-site faces + edge matrix ==================== */

#[test]
fn a8_sqrt_single_site_u64_edges_agree() {
    let vals: [u64; 13] = [
        0,
        1,
        2,
        3,
        4,
        15,
        16,
        17,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        4_611_686_018_427_387_904, // 2^62 = (2^31)^2 — even perfect square
        4_611_686_018_427_387_905, // 2^62 + 1
        u64::MAX,
    ];
    for &v in &vals {
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_r(sqrt_node(inst, get_node(1)), TPL64, 3);
            let interp = run_ir_interp(&ir, &u64_word(v));
            // the interpreter channel is the consensus oracle
            let want_interp = match inst {
                Bytecode::SQRT => isqrt(v as u128) as u64,
                _ => ceilsqrt(v as u128) as u64,
            };
            assert_eq!(
                interp.as_ref().map(|b| u64::from_be_bytes(b.as_slice().try_into().unwrap())),
                Ok(want_interp),
                "interp oracle {inst:?}({v})"
            );
            // rv64: exact agreement in ALL modes — the founding-era
            // ceil64 divergence (even perfect squares returned floor+1)
            // is fixed by the fresh r·r recompute; the even-perfect-square
            // faces here (0, 4, 16, 2^62) are the flipped pins
            for (mode, got) in run_ir_rv64(&ir, &u64_word(v)) {
                assert_eq!(
                    got.as_ref().map(|b| u64::from_be_bytes(b.as_slice().try_into().unwrap())),
                    Ok(want_interp),
                    "{inst:?}({v}) [{mode:?}]"
                );
            }
        }
    }
}

#[test]
fn a8_sqrt_single_site_narrow_widths_agree() {
    // u8 / u32 operands: the result keeps the input width; isqrt of the
    // width's max stays inside it (the w<64 range-check face can never
    // fire for a w-bit input — it stays at the site as the exact
    // interpreter fault shape). The formerly divergent ceil64 faces
    // (16 = 4², 0) agree three-way since the fresh-square fix.
    for &(inst, cast) in &[
        (Bytecode::SQRT, Bytecode::CU8),
        (Bytecode::SQRTUP, Bytecode::CU8),
        (Bytecode::SQRT, Bytecode::CU32),
        (Bytecode::SQRTUP, Bytecode::CU32),
    ] {
        // the CU* casts carry the interpreter's range check (a value above
        // the cast width faults 90 on BOTH engines — out of range here)
        let vals: &[u64] = if cast == Bytecode::CU8 {
            &[0, 1, 2, 3, 4, 15, 16, 17, 100, 255]
        } else {
            &[0, 1, 2, 3, 4, 15, 16, 17, 65535, 65536, u32::MAX as u64 - 1, u32::MAX as u64]
        };
        for &v in vals {
            let ir = swap_r(sqrt_node(inst, cast_node(cast, 1)), TPL64, 3);
            let interp = run_ir_interp(&ir, &u64_word(v));
            let want = match inst {
                Bytecode::SQRT => isqrt(v as u128) as u64,
                _ => ceilsqrt(v as u128) as u64,
            };
            assert!(interp.is_ok(), "interp {cast:?} {inst:?}({v})");
            for (mode, got) in run_ir_rv64(&ir, &u64_word(v)) {
                assert_eq!(
                    got.as_ref().map(|b| u64::from_be_bytes(b.as_slice().try_into().unwrap())),
                    Ok(want),
                    "{cast:?} {inst:?}({v}) [{mode:?}]"
                );
            }
        }
    }
}

#[test]
fn a8_sqrt_single_site_u128_edges_agree() {
    // x from the templates: A covers 0..2^64-1, B covers the 2^64
    // neighborhood and hi/lo splits, C covers MAX128 downward (all-F lo,
    // all-F hi boundary) — the MAX128 SQRTUP ceil = 2^64 wraps the low
    // half and carries into the high half. The u128 ceil RECOMPUTES the
    // square (no stale-square divergence — correct in the baseline).
    let cases: [(&str, &str, u64); 10] = [
        ("A", X_A, 0),
        ("A", X_A, 1),
        ("A", X_A, 2),
        ("A", X_A, 3),
        ("A", X_A, u32::MAX as u64),
        ("A", X_A, u64::MAX),
        ("B", X_B, 1),        // 2^64 + 1
        ("B", X_B, u64::MAX), // MAX128
        ("C", X_C, 0),        // MAX128
        ("C", X_C, u64::MAX), // 2^128 - 2^64 (all-F hi, zero lo)
    ];
    for (tag, x_expr, a) in &cases {
        let tpl = tpl128(x_expr);
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_r(sqrt_node(inst, get_node(3)), &tpl, 4);
            agree_ir(&ir, &u64_word(*a), &format!("sqrt128 {tag} {inst:?}(a={a})"));
            let x = match *tag {
                "A" => *a as u128,
                "B" => (*a as u128) << 64 | (*a as u128),
                _ => u128::MAX - *a as u128,
            };
            let want = match inst {
                Bytecode::SQRT => isqrt(x),
                _ => ceilsqrt(x),
            };
            let got = run_ir_interp(&ir, &u64_word(*a)).expect("sqrt128 value");
            assert_eq!(got, be(want), "golden sqrt128 {tag} {inst:?}(a={a}) x={x}");
        }
    }
    // the explicit ceil-wrap pin: sqrt_up(MAX128) = 2^64 exactly — the
    // result pair is (lo = 0, hi = 1)
    let ir = swap_r(sqrt_node(Bytecode::SQRTUP, get_node(3)), &tpl128(X_C), 4);
    let got = run_ir_interp(&ir, &u64_word(0)).expect("sqrt_up MAX128");
    assert_eq!(got, be(1u128 << 64), "sqrt_up(MAX128) = 2^64 (lo wraps, hi carries)");
}

#[test]
fn a8_sqrt_fault_faces_stay_at_site() {
    // bytes operand: a compile reject (the operand has no static uint
    // width) — the interpreter would fault CastParamFail 72 at runtime;
    // the rv64 surface is the up-front kernel-subset rejection
    let ir = swap_r(sqrt_node(Bytecode::SQRT, get_node(0)), TPL64, 3);
    let interp = run_ir_interp(&ir, &u64_word(1));
    assert!(interp.is_err(), "interp must fault on a bytes operand");
    assert_eq!(interp.as_ref().unwrap_err().code, 72, "CastParamFail 72");
    for mode in MODES {
        let e = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX).unwrap_err();
        assert!(
            e.contains("outside the translatable kernel subset"),
            "bytes operand must compile-reject in {mode:?}: {e}"
        );
    }

    // (nil-72 note: every runtime-reachable may-Nil operand route in the
    // kernel subset faults BEFORE the sqrt site on at least one engine —
    // byte(data, k) OOB faults 42/71 divergently (a pre-existing byte-op
    // face, out of A8 scope), map misses fault 85, casts on Nil fault 71 —
    // so no input reaches the site's nil-72 guard. The guard emission is
    // byte-identical between the gated and inline shapes in any case: it
    // runs in emit_sqrt BEFORE the emitter dispatch, before the pops.)

    // faulting operand evaluation (1/0 inside the operand) → 90, both
    // engines, fault order preserved (the operand fault precedes sqrt)
    let div0 = Box::new(IRNodeDouble {
        hrtv: true,
        inst: Bytecode::DIV,
        subx: lit(1),
        suby: lit(0),
    });
    let ir = swap_r(sqrt_node(Bytecode::SQRT, div0), TPL64, 3);
    // real input: the template's own byte(data, i) reads must stay in
    // range so the swapped operand's fault is the face under test
    agree_ir(&ir, &u64_word(0), "sqrt(1/0 operand) → 90");
    let interp = run_ir_interp(&ir, &u64_word(0));
    assert_eq!(interp.as_ref().unwrap_err().code, 90, "operand div-by-zero → 90");
}

/// The FLIPPED pin (founding-era divergence set, disclosed by the D8+A8
/// acceptance report): the even perfect squares — where the stale tail
/// returned floor+1 — now agree THREE-WAY (interp == Im == Imc ==
/// ImcRemap) on the u64 sqrt_up channel. Boundary set 0/4/16/2^62 plus
/// ≥10^3 seeded random even perfect squares; the container is built once
/// per mode and replayed per input.
#[test]
fn a8_sqrt_up64_even_perfect_squares_three_way_agree() {
    let mut roots: Vec<u64> = vec![0, 2, 4, 1 << 31]; // → 0, 4, 16, 2^62
    let mut rng = Rng::new(0xE1E7_50D1);
    while roots.len() < 1_100 {
        let f = (rng.next() >> 1) % (1u64 << 32); // < 2^32
        roots.push(f & !1u64); // even root
    }
    let xs: Vec<u64> = roots.iter().map(|&f| f.wrapping_mul(f)).collect();
    assert!(xs.len() >= 1_000, "even-square corpus too small: {}", xs.len());

    let ir = swap_r(sqrt_node(Bytecode::SQRTUP, get_node(1)), TPL64, 3);
    let interp = |input: &[u8]| -> u64 {
        let out = run_ir_interp(&ir, input).expect("interp sqrt_up even square");
        u64::from_be_bytes(out.as_slice().try_into().unwrap())
    };
    let cap = SpaceCap::new(0);
    let mut engines = Vec::new();
    for &mode in MODES.iter() {
        let lo = lower_ir(&ir, mode);
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let c = lo
            .pack_segments_profile(&cap, 64, 4800, 16, profile)
            .expect("pack");
        let fno = CalcFnObj::build(c, &cap, CalcHostConfig::new()).expect("build");
        engines.push((mode, fno));
    }
    for (i, &x) in xs.iter().enumerate() {
        let input = u64_word(x);
        let want = interp(&input);
        // the interpreter oracle: sqrt_up of a perfect square is isqrt
        assert_eq!(want, isqrt(x as u128) as u64, "interp oracle x={x}");
        for (mode, fno) in &engines {
            let ex = fno.run(
                &input,
                CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
            );
            let out = ex
                .result
                .unwrap_or_else(|_| panic!("sqrt_up({x}) [{mode:?}] case {i} exited"));
            let got = u64::from_be_bytes(out.as_slice().try_into().unwrap());
            assert_eq!(got, want, "sqrt_up({x}) [{mode:?}] case {i} — three-way flip pin");
            // and NOT the stale model's floor+1
            assert_ne!(got, ceil64_stale_model(x), "stale model resurfaced at x={x} [{mode:?}]");
        }
    }
}

/* ================= multi-site gate ==================================== */

#[test]
fn a8_sqrt_gate_amortizes_and_agrees_u64() {
    // N sites of one variant: N ≤ 2 → gate OFF (inline per site), N ≥ 3 →
    // the routine is emitted once and each extra site costs only its
    // staging shape (sd + jal + ld + one result ADD). The interpreter
    // channel must agree at EVERY N.
    let base = swap_r(lit(0), TPL64, 3);
    for mode in MODES {
        let b0 = insts(&base, mode);
        let mut deltas = Vec::new();
        for n in [1usize, 2, 3, 5] {
            let ir = swap_r(sqrt_chain(Bytecode::SQRT, 1, n), TPL64, 3);
            agree_ir(&ir, &u64_word(1_000_003), &format!("sqrt64 chain N={n}"));
            agree_ir(&ir, &u64_word(u64::MAX), &format!("sqrt64 chain N={n} max (operand 90 face)"));
            let d = insts(&ir, mode) - b0;
            deltas.push(d);
            println!("u64 floor N={n} mode {mode:?}: delta={d}");
        }
        let inline_marginal = deltas[1] - deltas[0];
        // gate OFF: every site is a full inline body (≥ 10 slots over glue)
        assert!(inline_marginal >= 10, "inline marginal {inline_marginal} too small");
        // turning the gate on REDUCES the marginal site cost
        assert!(
            deltas[2] - deltas[1] < inline_marginal,
            "the gated N=3 marginal {} must beat the inline {inline_marginal} in {mode:?}",
            deltas[2] - deltas[1]
        );
        // steady state: each extra gated site = sd + jal + ld + ADD ≤ 6
        let gated = (deltas[3] - deltas[2]) / 2;
        assert!(
            gated <= 6,
            "gated per-extra-site cost {gated} too large in {mode:?} — the routine did not amortize"
        );
    }
}

#[test]
fn a8_sqrt_gate_amortizes_and_agrees_u64_ceil() {
    let base = swap_r(lit(0), TPL64, 3);
    for mode in MODES {
        let b0 = insts(&base, mode);
        let mut deltas = Vec::new();
        for n in [1usize, 2, 3, 4] {
            let ir = swap_r(sqrt_chain(Bytecode::SQRTUP, 1, n), TPL64, 3);
            agree_ir(&ir, &u64_word(1_000_003), &format!("sqrt_up64 chain N={n}"));
            let d = insts(&ir, mode) - b0;
            deltas.push(d);
            println!("u64 ceil N={n} mode {mode:?}: delta={d}");
        }
        // the gate turns on at N=3; the marginal site drops from the
        // inline 14+ slots to the staging shape, and the steady state is
        // sd + jal + ld + ADD ≤ 6
        assert!(
            deltas[2] - deltas[1] < deltas[1] - deltas[0],
            "the gated ceil N=3 marginal {} must beat the inline {} in {mode:?}",
            deltas[2] - deltas[1],
            deltas[1] - deltas[0]
        );
        let gated = deltas[3] - deltas[2];
        assert!(gated <= 6, "gated ceil per-site cost {gated} too large in {mode:?}");
    }
}

#[test]
fn a8_sqrt_gate_amortizes_and_agrees_u128() {
    // u128 floor chain over x (slot 3 of TPL128_A): N=1..5, gate ON from 3
    let tpl = tpl128(X_A);
    let base = swap_r(lit(0), &tpl, 4);
    for mode in MODES {
        let b0 = insts(&base, mode);
        let mut deltas = Vec::new();
        for n in [1usize, 2, 3, 5] {
            let ir = swap_r(sqrt_chain(Bytecode::SQRT, 3, n), &tpl, 4);
            agree_ir(&ir, &u64_word(1_000_003), &format!("sqrt128 chain N={n}"));
            agree_ir(&ir, &u64_word(u64::MAX), &format!("sqrt128 chain N={n} max"));
            let d = insts(&ir, mode) - b0;
            deltas.push(d);
            println!("u128 floor N={n} mode {mode:?}: delta={d}");
        }
        let inline_marginal = deltas[1] - deltas[0];
        assert!(inline_marginal >= 13, "u128 inline marginal {inline_marginal} too small");
        assert!(
            deltas[2] - deltas[1] < inline_marginal,
            "the gated u128 N=3 marginal {} must beat the inline {inline_marginal} in {mode:?}",
            deltas[2] - deltas[1]
        );
        // each extra gated site = sd pair + jal + ld pair + one u128
        // result ADD (7 slots) + the pair handback mvs ≤ 16
        let gated = (deltas[3] - deltas[2]) / 2;
        assert!(
            gated <= 16,
            "u128 gated per-extra-site cost {gated} too large in {mode:?}"
        );
    }
    // ceil128 chain (N=3): the 36-slot routine + 3 staged sites beats the
    // three 25-slot inline bodies
    let tplc = tpl128(X_C);
    for mode in MODES {
        let base = swap_r(lit(0), &tplc, 4);
        let b0 = insts(&base, mode);
        let mut deltas = Vec::new();
        for n in [1usize, 2, 3] {
            let ir = swap_r(sqrt_chain(Bytecode::SQRTUP, 3, n), &tplc, 4);
            agree_ir(&ir, &u64_word(0), &format!("sqrt_up128 chain N={n}"));
            agree_ir(&ir, &u64_word(1), &format!("sqrt_up128 chain N={n} a=1"));
            let d = insts(&ir, mode) - b0;
            deltas.push(d);
            println!("u128 ceil N={n} mode {mode:?}: delta={d}");
        }
        assert!(
            deltas[2] - deltas[1] < deltas[1] - deltas[0],
            "the gated ceil128 N=3 marginal {} must beat the inline {} in {mode:?}",
            deltas[2] - deltas[1],
            deltas[1] - deltas[0]
        );
    }
}

#[test]
fn a8_sqrt_mixed_variants_gate_per_variant() {
    // 3 floor + 2 ceil sites in one body: the floor variant gates on (its
    // routine appears once), the ceil variant stays inline (2 < 3). Both
    // channels must agree. Inputs cover both sides of the former ceil64
    // divergence set: 1000003 is square-free, 2^62+1 sits one above the
    // even perfect square 2^62 (now agreeing like every other input)
    // (even perfect squares): 1000003 (+0) is square-free, 2^62+1
    // likewise.
    let expr = add(
        add(
            sqrt_node(Bytecode::SQRT, get_node(1)),
            sqrt_node(Bytecode::SQRT, get_node(1)),
        ),
        add(
            sqrt_node(Bytecode::SQRT, get_node(1)),
            add(
                sqrt_node(Bytecode::SQRTUP, get_node(1)),
                sqrt_node(Bytecode::SQRTUP, get_node(1)),
            ),
        ),
    );
    let ir = swap_r(expr, TPL64, 3);
    let inputs: [u64; 2] = [1_000_003, 4_611_686_018_427_387_905];
    for &v in &inputs {
        agree_ir(&ir, &u64_word(v), &format!("mixed sqrt/sqrt_up 3+2 body a={v}"));
        let want = 3 * isqrt(v as u128) + 2 * ceilsqrt(v as u128);
        let got = run_ir_interp(&ir, &u64_word(v)).expect("mixed value");
        assert_eq!(got, be8(want as u64), "golden mixed a={v}");
    }
    for mode in MODES {
        let d = insts(&ir, mode);
        println!("mixed 3+2 body {mode:?}: insts={d}");
    }
}

#[test]
fn a8_sqrt_lowering_is_deterministic() {
    for n in [1usize, 3] {
        let ir = swap_r(sqrt_chain(Bytecode::SQRT, 1, n), TPL64, 3);
        for mode in MODES {
            let a = lower_ir(&ir, mode);
            let b = lower_ir(&ir, mode);
            assert_eq!(
                (a.stats.insts, &a.code[..]),
                (b.stats.insts, &b.code[..]),
                "sqrt N={n} {mode:?}: lowering must be deterministic"
            );
        }
    }
}
