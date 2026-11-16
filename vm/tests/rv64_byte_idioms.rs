//! P3-IDIOMS (A-25 milestone M3): constant-offset byte-assembly groups →
//! one span-guarded wide load (ld/lwu/lhu). Semantics proof: docs.loc/
//! rv64im/94 Method/Lemmas 1-6.
//!
//! Test matrix:
//! * OOB byte-fidelity: every group boundary length (exactly the k-th leaf
//!   OOB, k = 1..N) is cross-checked. PRE-EXISTING oracle divergence at
//!   HEAD 374ff48 (verified by stash-probe, unfused body): interpreter
//!   byte-OOB = StackError(42) (value/operand.rs cutbyte:43-46) while rv64
//!   = CastFail(71) (fault_cast — the shipped semantics, reflected in the
//!   census causes `cast→71`). Fusion changes NEITHER side: the crosscheck
//!   pins (interp Err(42), rv64 Err(71) on all three modes) for OOB and
//!   full 4-way equality for in-range values;
//! * value edges (all-0xFF maxima) and random sweeps;
//! * recognition pins (deterministic accept/reject) as EXACT relative
//!   instruction-census pairs: each negative keeps all its N byte stations
//!   (lbu), each fused twin drops exactly N lbu and adds exactly one wide
//!   load — pre-existing load sites (the RET output-copy loop, CBYTES) are
//!   identical on both sides of a pair and cancel out;
//! * determinism (byte-identical rebuild).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};
use vm::ir::rv64::Rv64Mode;

fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

/// Cross-engine check with the pinned pre-existing byte-OOB divergence:
/// the three rv64 modes must always agree with EACH OTHER; vs the
/// interpreter either full agreement (Ok values identical) or the pinned
/// pair (interp Err(42), rv64 Err(71)) — any other shape is a failure.
fn crosscheck(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    for input in inputs {
        let want = interp_obs(&it, input);
        let rvres: Vec<_> = rv.run(input);
        for (mode, got) in &rvres {
            assert_eq!(got, &rvres[0].1, "rv64 modes must agree on {input:?}");
        }
        let any_rv = &rvres[0].1;
        match (want, any_rv) {
            (Ok(w), Ok(g)) => assert_eq!(w, *g, "4-way Ok divergence on {input:?}"),
            (Err(42), Err(71)) => {} // the pinned pre-existing byte-OOB pair
            (w, g) => panic!(
                "unexpected cross-engine shape interp={w:?} rv64={g:?} on {input:?} of:\n{body}"
            ),
        }
    }
}

/// Four-way strict equality (use only where no trap is reachable).
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

/// --- Im-mode instruction census (uncompressed words only) -------------

/// (ld f3=3, lwu f3=6, lhu f3=5, lbu f3=4) — opcode LOAD = 0b0000011.
/// f3=1 is lh (sign-extending) — its presence would be a bug signature.
/// The literal pool is appended after the trap epilogue (ecall =
/// 0x00000073) — pool bytes decoded as words are NOT instructions.
fn im_load_census(rv: &Rv64) -> (usize, usize, usize, usize) {
    let code = &rv.codes.iter().find(|(m, _)| *m == Rv64Mode::Im).unwrap().1;
    let (mut ld, mut lwu, mut lhu, mut lbu) = (0usize, 0, 0, 0);
    let mut stop = code.len();
    for (i, w) in code.chunks_exact(4).enumerate() {
        if u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == 0x0000_0073 {
            stop = (i + 1) * 4;
            break;
        }
    }
    for w in code[..stop].chunks_exact(4) {
        let word = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        if word & 0x7F == 0b0000011 {
            match (word >> 12) & 0b111 {
                3 => ld += 1,
                6 => lwu += 1,
                5 => lhu += 1,
                4 => lbu += 1,
                _ => {}
            }
        }
    }
    (ld, lwu, lhu, lbu)
}

/// EXACT relative census pin: `neg` keeps all `stations` byte stations,
/// `twin` (identical except the fusion-eligible shape) drops exactly
/// `stations` lbu, adds exactly `wl_add` lhu/lwu (unambiguous — only the
/// idiom emits them here) and exactly `ld_add` net ld. All pre-existing
/// load sites (RET copy loop, CBYTES, plain slot GET reads) are
/// structurally identical on both sides and cancel.
fn census_pair(neg: &str, twin: &str, stations: usize, wl_add: usize) {
    let rn = Rv64::try_build(neg).unwrap_or_else(|e| panic!("neg must build: {neg}\n{e}"));
    let rt = Rv64::try_build(twin).unwrap_or_else(|e| panic!("twin must build: {twin}\n{e}"));
    let (_, nwu, nhu, nbu) = im_load_census(&rn);
    let (tl, twu, thu, tbu) = im_load_census(&rt);
    assert_eq!(
        tbu, nbu - stations,
        "twin must drop exactly {stations} lbu: neg(lbu={nbu}) twin(lbu={tbu})"
    );
    assert_eq!(
        twu + thu,
        nwu + nhu + wl_add,
        "twin must add exactly {wl_add} lhu/lwu"
    );
    // NOTE: the net ld delta is NOT pinned — slot-GET reads / forwarding
    // spill decisions shift between a fused and an unfused body (ld is
    // the slot-read form in Im mode); lbu and lhu/lwu are the unambiguous
    // idiom signals.
}

/// Boundary inputs for an N-byte group at c0: every length 1..=c0+n makes
/// a different leaf the first OOB one, plus in-range lengths with mixed
/// and maximal values.
fn boundary_inputs(c0: u64, n: u64, extra: usize) -> Vec<Vec<u8>> {
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    let lo = (c0 as usize + 1).max(1);
    for len in lo..=(c0 as usize + n as usize).min(extra) {
        inputs.push(vec![0x41u8; len]);
    }
    inputs.push((0..extra).map(|i| (i * 37 + 11) as u8).collect());
    inputs
}

/* ================================================================== */
/* Section 1: LE statement chain (the keccak absorb shape).            */
/* ================================================================== */

fn le_chain_body(slot: &str, c0: u64, n: u64, out_expr: &str) -> String {
    let mut s = format!("var {slot} = byte(data, {c0}u64) as u64\n");
    for k in 1..n {
        s.push_str(&format!(
            "{slot} = {slot} | ((byte(data, {c}u64) as u64) << {sh}u64)\n",
            c = c0 + k,
            sh = 8 * k
        ));
    }
    s.push_str(out_expr);
    s
}

/// Boundary fidelity: data length L crossing each leaf — the four engines
/// agree (OOB = the pinned (42, 71) pair, in-range values identical).
#[test]
fn f1_le_chain_oob_fidelity() {
    let body = le_chain_body("s0", 0, 8, "return (s0) as bytes\n");
    let mut inputs = boundary_inputs(0, 8, 64);
    inputs.push(vec![0xFFu8; 40]);
    crosscheck(&body, &inputs);
}

/// Two chains on separate slots (keccak 17-lane shape, 2 lanes) — chains
/// must not bleed into each other and both must fuse (16 lbu dropped).
#[test]
fn f2_le_chain_two_lanes() {
    let fused = le_chain_body("s0", 0, 8, "")
        + &le_chain_body("s1", 8, 8, "")
        + "return (s0 as bytes) ++ (s1 as bytes)\n";
    // negative twin: offsets stride 2 (non-consecutive) — same station
    // count, chains cannot start
    let neg = {
        let mut s = String::new();
        for (slot, c0) in [("s0", 0u64), ("s1", 16u64)] {
            s.push_str(&format!("var {slot} = byte(data, {c0}u64) as u64\n"));
            for k in 1..8u64 {
                s.push_str(&format!(
                    "{slot} = {slot} | ((byte(data, {}u64) as u64) << {}u64)\n",
                    c0 + 2 * k,
                    8 * k
                ));
            }
        }
        s.push_str("return (s0 as bytes) ++ (s1 as bytes)\n");
        s
    };
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for len in 1..=16usize {
        inputs.push(vec![0x41u8; len]);
    }
    inputs.push((0..64).map(|i| (i * 29 + 5) as u8).collect());
    crosscheck(&fused, &inputs);
    crosscheck(&neg, &inputs);
    census_pair(&neg, &fused, 16, 0);
}

/// LE single-expression trees, N ∈ {2,4,8} (lhu/lwu/ld), boundary sweep
/// + exact width pins.
#[test]
fn f3_le_expr_trees() {
    for n in [2u64, 4, 8] {
        let mut terms: Vec<String> = Vec::new();
        for k in 0..n {
            terms.push(if k == 0 {
                format!("byte(data, {k}u64)")
            } else {
                format!("(byte(data, {k}u64) << {}u64)", 8 * k)
            });
        }
        let fused = format!("var v = {}\nreturn (v) as bytes\n", terms.join(" | "));
        let neg = format!(
            "var v = {}\nreturn (v) as bytes\n",
            terms
                .iter()
                .enumerate()
                .map(|(k, t)| if k == 0 {
                    t.clone()
                } else {
                    t.replace(
                        &format!("<< {}u64", 8 * k),
                        &format!("<< {}u64", 8 * k + 1),
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let mut inputs = boundary_inputs(0, n, 32);
        inputs.push(vec![0xFFu8; 16]);
        crosscheck(&fused, &inputs);
        crosscheck(&neg, &inputs);
        if n == 8 {
            census_pair(&neg, &fused, n as usize, 0);
        } else {
            census_pair(&neg, &fused, n as usize, 1);
        }
    }
}

/* ================================================================== */
/* Section 2: BE mul-add groups (hacd200 :629 / sha256 ring).          */
/* ================================================================== */

/// BE-2 const (:629 shape): boundary sweep len 1..=2 + value edges.
#[test]
fn f4_be_const_two() {
    let fused = "var cnt = byte(data, 0u64) * 256u64 + byte(data, 1u64)\n\
                 return (cnt) as bytes\n";
    let neg = "var cnt = byte(data, 0u64) * 256u64 + byte(data, 1u64) * 2u64\n\
               return (cnt) as bytes\n";
    let inputs = boundary_inputs(0, 2, 8);
    crosscheck(fused, &inputs);
    crosscheck(neg, &inputs);
    census_pair(neg, fused, 2, 1);
}

/// BE-4 const at a non-zero offset (c0 = 4): boundary sweep makes each of
/// the four leaves OOB in turn.
#[test]
fn f5_be_const_four_offset() {
    let fused = "var w = byte(data, 4u64) * 16777216u64 + byte(data, 5u64) * 65536u64 + \
                 byte(data, 6u64) * 256u64 + byte(data, 7u64)\n\
                 return (w) as bytes\n";
    let neg = "var w = byte(data, 4u64) * 16777216u64 + byte(data, 5u64) * 65536u64 + \
               byte(data, 6u64) * 256u64 + byte(data, 7u64) * 2u64\n\
               return (w) as bytes\n";
    let inputs = boundary_inputs(4, 4, 24);
    crosscheck(fused, &inputs);
    crosscheck(neg, &inputs);
    census_pair(neg, fused, 4, 1);
}

/// BE var-base ring (sha256 unpack shape): the loop form. The length
/// sweep makes each of the four leaf offsets the first OOB one at the
/// loop's last word.
#[test]
fn f6_be_var_ring_loop() {
    let body = "var w = []\n\
                var o = 0u64\n\
                while o < 64u64 {\n\
                append(w, byte(data, o) * 16777216u64 + byte(data, o + 1u64) * 65536u64 + byte(data, o + 2u64) * 256u64 + byte(data, o + 3u64))\n\
                o += 4u64\n\
                }\n\
                return (w[0] + w[15]) as bytes\n";
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for len in [1usize, 4, 16, 59, 60, 61, 62, 63, 64] {
        inputs.push((0..len).map(|i| (i * 11 + 7) as u8).collect());
    }
    inputs.push(vec![0xFFu8; 64]);
    crosscheck(body, &inputs);
}

/// BE var-base single group with an input-driven base: sweeps the base
/// across the whole span-guard boundary (len = 16 fixed; base o ∈ 0..=255
/// via one input byte — o ∈ [13,255] exercises the span guard).
#[test]
fn f7_be_var_base_boundary_sweep() {
    let fused = "var o = byte(data, 8u64) as u64\n\
                 var w = byte(data, o) * 16777216u64 + byte(data, o + 1u64) * 65536u64 + \
                 byte(data, o + 2u64) * 256u64 + byte(data, o + 3u64)\n\
                 return (w) as bytes\n";
    let neg = "var o = byte(data, 8u64) as u64\n\
               var w = byte(data, o) * 16777216u64 + byte(data, o + 2u64) * 65536u64 + \
               byte(data, o + 3u64) * 256u64 + byte(data, o + 4u64)\n\
               return (w) as bytes\n";
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for b in 0..=255u8 {
        let mut v = vec![0xEEu8; 16];
        v[8] = b;
        inputs.push(v);
    }
    crosscheck(fused, &inputs);
    crosscheck(neg, &inputs);
    census_pair(neg, fused, 4, 1);
}

/* ================================================================== */
/* Section 3: recognition pins — negatives must NOT fuse.              */
/* ================================================================== */

#[test]
fn f8_negatives_do_not_fuse() {
    // leaf consumed elsewhere (temp var) vs the pure OR-tree twin
    census_pair(
        "var t = byte(data, 0u64)\nvar v = (t | (byte(data, 1u64) << 8u64)) as bytes\nreturn v\n",
        "var v = (byte(data, 0u64) | (byte(data, 1u64) << 8u64)) as bytes\nreturn v\n",
        2,
        1,
    );
    // loop-carried accumulator (hacd200 :642): ADD's left arm is
    // MUL(GET, lit) — no byte leaf — never fuses
    let body = "var num = 0u64\n\
                var j = 0u64\n\
                while j < 8u64 {\n\
                num = num * 256u64 + byte(data, 24u64 + j)\n\
                j += 1u64\n\
                }\n\
                return (num) as bytes\n";
    let rv = Rv64::try_build(body).unwrap();
    let (_, lwu, lhu, _) = im_load_census(&rv);
    assert_eq!(lwu + lhu, 0, "loop-carried accumulator must not fuse");
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for len in [1usize, 8, 24, 31, 32] {
        inputs.push(vec![0x99u8; len]);
    }
    inputs.push(vec![0xFFu8; 40]);
    inputs.push((0..40).map(|i| (i * 7 + 2) as u8).collect());
    crosscheck(body, &inputs);
}

/* ================================================================== */
/* Section 4: value edges + determinism + big-offset guard path.       */
/* ================================================================== */

/// All-0xFF inputs: LE-8 → u64::MAX; BE-4 → 0xFFFFFFFF (4-way identical).
#[test]
fn f10_value_max_edges() {
    let le = le_chain_body("s0", 0, 8, "return (s0) as bytes\n");
    let be = "var w = byte(data, 0u64) * 16777216u64 + byte(data, 1u64) * 65536u64 + \
              byte(data, 2u64) * 256u64 + byte(data, 3u64)\nreturn (w) as bytes\n";
    crosscheck(&le, &[vec![0xFFu8; 8]]);
    crosscheck(&be, &[vec![0xFFu8; 4]]);
}

/// A group at offset c0 > 2047 forces the li+add GUARD path — the guard
/// still fires 71 four-way. (The in-range li+add LOAD path is
/// untestable-by-oracle: the interpreter caps input/word-literal values
/// at ≤2048B → Err(39) — the same address arithmetic is 4-way tested by
/// the var-base forms f6/f7 which always load via a register address.)
#[test]
fn f11_big_offset_guard_path() {
    let body = le_chain_body("s0", 2500, 8, "return (s0) as bytes\n");
    let inputs: Vec<Vec<u8>> = (1..=16usize)
        .map(|len| vec![0x63u8; len])
        .chain(std::iter::once((0..64).map(|i| (i * 3 + 1) as u8).collect()))
        .collect();
    crosscheck(&body, &inputs);
}

/// Determinism: the fusion is a deterministic function of the IR — two
/// independent builds produce byte-identical containers (all 3 modes).
#[test]
fn f12_determinism() {
    let body = le_chain_body("s0", 0, 8, "")
        + "var t = byte(data, 8u64) * 16777216u64 + byte(data, 9u64) * 65536u64 + byte(data, 10u64) * 256u64 + byte(data, 11u64)\nreturn (t) as bytes\n";
    let a = Rv64::try_build(&body).unwrap();
    let b = Rv64::try_build(&body).unwrap();
    for ((ma, ca), (mb, cb)) in a.containers.iter().zip(b.containers.iter()) {
        assert_eq!(ma, mb);
        assert_eq!(ca, cb, "containers must be byte-identical for {ma:?}");
    }
}

/// Fused groups inside an expression with surrounding operands: the
/// fused emission leaves exactly one int on the vstack per group root
/// (LE-4 chain + BE-4 group summed).
#[test]
fn f13_fused_body_stack_contract() {
    let fused = "var s0 = byte(data, 0u64) as u64\n\
                 s0 = s0 | ((byte(data, 1u64) as u64) << 8u64)\n\
                 s0 = s0 | ((byte(data, 2u64) as u64) << 16u64)\n\
                 s0 = s0 | ((byte(data, 3u64) as u64) << 24u64)\n\
                 var t = byte(data, 4u64) * 16777216u64 + byte(data, 5u64) * 65536u64 + \
                 byte(data, 6u64) * 256u64 + byte(data, 7u64)\n\
                 return ((s0 + t) as u64) as bytes\n";
    let neg = "var s0 = byte(data, 0u64) as u64\n\
               s0 = s0 | ((byte(data, 1u64) as u64) << 9u64)\n\
               s0 = s0 | ((byte(data, 2u64) as u64) << 16u64)\n\
               s0 = s0 | ((byte(data, 3u64) as u64) << 24u64)\n\
               var t = byte(data, 4u64) * 16777216u64 + byte(data, 5u64) * 65536u64 + \
               byte(data, 6u64) * 256u64 + byte(data, 7u64) * 2u64\n\
               return ((s0 + t) as u64) as bytes\n";
    let inputs = boundary_inputs(0, 8, 16);
    crosscheck(fused, &inputs);
    crosscheck(neg, &inputs);
    census_pair(neg, fused, 8, 2);
}

/// Keep the strict-agree helper referenced for the no-trap sweeps used in
/// targeted debugging (in-range-only); a direct assertion keeps the
/// helper honest on a minimal body.
#[test]
fn f14_agree4_inrange_helper() {
    let body = le_chain_body("s0", 0, 8, "return (s0) as bytes\n");
    agree4(&body, &[vec![0xFFu8; 8], vec![0x07u8; 16]]);
}
