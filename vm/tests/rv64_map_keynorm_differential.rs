//! B2 — targeted differential corpus for the `emit_ctr_map_key_uint`
//! rewrite ("write-first + thin-arm": the full-width big-endian form is
//! stored into the reserved arena slot first, then the first-nonzero-byte
//! scan only shrinks the (kptr, klen) descriptor — `kptr` may now point
//! into the MIDDLE of the reserved region).
//!
//! What this file pins, beyond the standing `rv64_map_family` suite:
//!   * the canonical key BYTES themselves, byte for byte: `keys(m)[0]` of
//!     a map keyed by a DYNAMIC uint is the normalized minimal-BE form
//!     (`0 → [0x00]`, `1 → [0x01]`, `0x100 → [01,00]`, `U64(1) ≡ U8(1)`)
//!     — checked against the interpreter AND against a local re-statement
//!     of the `uint_key_bytes` spec (value/util.rs) as a third oracle;
//!   * every cascade arm: the boundary corpus below puts the first
//!     nonzero byte at EVERY big-endian position of every width
//!     (u8/u16/u32/u64/u128 → wbytes 1/2/4/8/16), including 0, 1,
//!     2^k±1 for k = 8/16/32/64, all-ones, and every pattern
//!     {0x01, 0x80, 0xFF} at the leading byte (with and without a
//!     nonzero tail);
//!   * descriptor-shrink safety: consecutive dynamic normalizations in
//!     one map (the second key's arena bump sits right above the first
//!     key's skipped leading zeros), remove/re-insert, overwrite, and
//!     KEYS/VALUES snapshots after MERGE — none may observe the dead
//!     leading-zero bytes;
//!   * fault faces with dynamic keys: missing key 85, MERGE duplicate 84,
//!     PACKMAP duplicate 84, cross-width collision (u8(1) vs u64(1), and
//!     uint vs raw-bytes `[0x01]`) — identical codes on every engine.
//!
//! Every case is four-way (consensus interpreter == rv64 {Im, Imc,
//! ImcRemap}); the `expect` runners additionally pin the exact bytes.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

/// The canonical map-key spec, restated locally (the interpreter's
/// `uint_key_bytes`, value/util.rs — minimal big-endian, zero → `[0x00]`).
fn key_bytes(v: u128) -> Vec<u8> {
    let be = v.to_be_bytes();
    let first = be.iter().position(|b| *b != 0).unwrap_or(be.len());
    match first {
        16 => vec![0u8],
        f => be[f..].to_vec(),
    }
}

/// The exact-width big-endian input carrying `v` (`wb` bytes).
fn be_input(v: u128, wb: usize) -> Vec<u8> {
    v.to_be_bytes()[16 - wb..].to_vec()
}

/// Boundary corpus for a `wb`-byte width: 0, 1, all-ones, 2^k±1, and the
/// first nonzero byte at every BE position in {0x01, 0x80, 0xFF} (with and
/// without a nonzero tail) — every thin arm j of the rewrite.
fn boundary_values(wb: usize) -> Vec<u128> {
    let bits = wb * 8;
    let max: u128 = if bits >= 128 { u128::MAX } else { (1u128 << bits) - 1 };
    let mut vs: Vec<u128> = vec![0, 1, max];
    for k in [8usize, 16, 32, 64] {
        if k < bits {
            vs.extend_from_slice(&[(1u128 << k) - 1, 1u128 << k, (1u128 << k) + 1]);
        }
    }
    for pos in 0..wb {
        let sh = 8 * (wb - 1 - pos);
        for pat in [1u128, 0x80, 0xff] {
            vs.push(pat << sh);
            vs.push((pat << sh) | 1);
        }
    }
    vs.retain(|v| *v <= max);
    vs.sort_unstable();
    vs.dedup();
    vs
}

fn build(body: &str) -> (Interp, Rv64) {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    (it, rv)
}

/// Four-way agreement with an exact expectation: the interpreter must
/// produce `want`, every rv64 mode must produce `want`.
fn expect_ok(body: &str, cases: &[(Vec<u8>, Vec<u8>)]) {
    let (it, rv) = build(body);
    for (input, want) in cases {
        let got = it.run(input).map_err(|f| f.code as u64);
        assert_eq!(
            got,
            Ok(want.clone()),
            "interpreter diverged from the canonical spec on input {input:?} of body:\n{body}"
        );
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got,
                Ok(want.clone()),
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

/// Four-way agreement with an exact FAULT expectation.
fn expect_fault(body: &str, cases: &[(Vec<u8>, u64)]) {
    let (it, rv) = build(body);
    for (input, want) in cases {
        let got = it.run(input).map_err(|f| f.code as u64);
        assert_eq!(
            got,
            Err(*want),
            "interpreter fault diverged on input {input:?} of body:\n{body}"
        );
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got,
                Err(*want),
                "mode {mode:?} fault diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

/// Plain four-way agreement (interpreter is the oracle).
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let (it, rv) = build(body);
    for input in inputs {
        let want = it.run(input).map_err(|f| f.code as u64);
        for (mode, got) in rv.run(input) {
            assert_eq!(got, want, "mode {mode:?} diverged on input {input:?} of body:\n{body}");
        }
    }
}

fn one() -> Vec<Vec<u8>> {
    vec![vec![1, 2, 3, 4]]
}

const WIDTHS: [usize; 5] = [1, 2, 4, 8, 16];

/* ================================================================== */
/* 1. The canonical key bytes themselves, every width, every arm.      */
/* ================================================================== */

#[test]
fn b2_key_bytes_canonical_every_width_every_arm() {
    for wb in WIDTHS {
        let bits = wb * 8;
        let body = format!(
            "var m = new_map()\ninsert(m, data as u{bits}, 5u64)\nvar ks = keys(m)\nreturn ks[0]"
        );
        let cases: Vec<(Vec<u8>, Vec<u8>)> =
            boundary_values(wb).into_iter().map(|v| (be_input(v, wb), key_bytes(v))).collect();
        assert!(cases.len() >= 2 * wb + 3, "corpus must cover every arm (wb={wb})");
        expect_ok(&body, &cases);
    }
}

/* ================================================================== */
/* 2. Dynamic insert / lookup / has_key / remove per width.            */
/* ================================================================== */

#[test]
fn b2_dynamic_roundtrip_every_width() {
    for wb in WIDTHS {
        let bits = wb * 8;
        // r=5, h1=1, h2=0, len=0 → 5100 for EVERY input
        let body = format!(
            "var m = new_map()\n\
             insert(m, data as u{bits}, 5u64)\n\
             var h1 = (has_key(m, data as u{bits}) as u64)\n\
             var r = (m[data as u{bits}] as u64)\n\
             remove(m, data as u{bits})\n\
             var h2 = (has_key(m, data as u{bits}) as u64)\n\
             return (r * 1000u64 + h1 * 100u64 + h2 * 10u64 + length(m)) as bytes"
        );
        let cases: Vec<(Vec<u8>, Vec<u8>)> = boundary_values(wb)
            .into_iter()
            .map(|v| (be_input(v, wb), 5100u64.to_be_bytes().to_vec()))
            .collect();
        expect_ok(&body, &cases);
    }
}

/* ================================================================== */
/* 3. Missing-key face 85 with a dynamic key, every width.             */
/* ================================================================== */

#[test]
fn b2_missing_dynamic_key_face85_every_width() {
    for wb in WIDTHS {
        let bits = wb * 8;
        // v>0: the stored key v/2 never equals the looked-up key v → 85
        let body = format!(
            "var m = new_map()\n\
             insert(m, (data as u{bits}) / 2u{bits}, 5u64)\n\
             var r = m[data as u{bits}]\n\
             return r as bytes"
        );
        let cases: Vec<(Vec<u8>, u64)> = boundary_values(wb)
            .into_iter()
            .filter(|v| *v > 0)
            .map(|v| (be_input(v, wb), 85))
            .collect();
        assert!(!cases.is_empty());
        expect_fault(&body, &cases);
    }
}

/* ================================================================== */
/* 4. Consecutive dynamic normalizations: the second key's arena bump  */
/*    sits right above the first key's skipped leading zeros.          */
/* ================================================================== */

#[test]
fn b2_two_dynamic_keys_adjacent_reservations() {
    for wb in [8usize, 16] {
        let bits = wb * 8;
        // b = a/256 + 1 equals a exactly at a=1 (the only fixed point of the
        // integer equation) — expect the overwrite value there, the pair
        // reading everywhere else
        let body = format!(
            "var a = data as u{bits}\n\
             var b = (a / 256u{bits}) + 1u{bits}\n\
             var m = new_map()\n\
             insert(m, a, 11u64)\n\
             insert(m, b, 22u64)\n\
             insert(m, a, 13u64)\n\
             return ((m[a] as u64) * 100u64 + (m[b] as u64)) as bytes"
        );
        let cases: Vec<(Vec<u8>, Vec<u8>)> = boundary_values(wb)
            .into_iter()
            .map(|a| {
                let b = a / 256 + 1;
                let v: u64 = if a == b { 13 * 100 + 13 } else { 13 * 100 + 22 };
                (be_input(a, wb), v.to_be_bytes().to_vec())
            })
            .collect();
        expect_ok(&body, &cases);
    }
    // remove + re-insert the same dynamic key (dead key bytes, fresh bump)
    let body = "var k = data as u64\n\
                var m = new_map()\n\
                insert(m, k, 1u64)\n\
                remove(m, k)\n\
                insert(m, k, 2u64)\n\
                return ((m[k] as u64) * 10u64 + length(m)) as bytes";
    let cases: Vec<(Vec<u8>, Vec<u8>)> = boundary_values(8)
        .into_iter()
        .map(|v| (be_input(v, 8), 21u64.to_be_bytes().to_vec()))
        .collect();
    expect_ok(body, &cases);
}

/* ================================================================== */
/* 5. Cross-width / cross-species canonical collisions.                */
/* ================================================================== */

#[test]
fn b2_cross_width_collision_dynamic() {
    // dynamic u64(1) overwrites the constant u8(1) entry — one slot
    expect_ok(
        "var m = new_map()\n\
         insert(m, 1u8, 5u64)\n\
         insert(m, (data as u8) as u64, 6u64)\n\
         return ((m[1u64] as u64) * 10u64 + length(m)) as bytes",
        &[(vec![1], 61u64.to_be_bytes().to_vec())],
    );
    // dynamic u8(1) then constant u64(1)
    expect_ok(
        "var m = new_map()\n\
         insert(m, (data as u8) as u64, 5u64)\n\
         insert(m, 1u8, 6u64)\n\
         return ((m[1u64] as u64) * 10u64 + length(m)) as bytes",
        &[(vec![1], 61u64.to_be_bytes().to_vec())],
    );
    // dynamic uint vs dynamic raw-bytes key `[0x01]` — same canonical slot
    expect_ok(
        "var m = new_map()\n\
         insert(m, (data as u8) as u64, 5u64)\n\
         insert(m, buf_left(1, data), 6u64)\n\
         return ((m[1u8] as u64) * 10u64 + length(m)) as bytes",
        &[(vec![1], 61u64.to_be_bytes().to_vec())],
    );
    // dynamic 0x100 → key bytes [01, 00] sorted after [01]
    expect_ok(
        "var m = new_map()\n\
         insert(m, data as u16, 1u64)\n\
         insert(m, 1u8, 2u64)\n\
         var ks = keys(m)\n\
         return ks[1]",
        &[(vec![1, 0], vec![0x01, 0x00])],
    );
}

/* ================================================================== */
/* 6. KEYS/VALUES sorted order with dynamic keys.                      */
/* ================================================================== */

#[test]
fn b2_keys_values_sorted_dynamic() {
    // b ∈ {1,2}: the map holds {b:2, b+1:3, b*256:1}; the canonical
    // byte-lex order is NOT the numeric order (b < b*256 < b+1 for b=1:
    // [01] < [01,00] < [02]) — a Rust BTreeMap over the canonical bytes is
    // the exact iteration-order oracle.
    use std::collections::BTreeMap;
    let oracle = |b: u64| -> BTreeMap<Vec<u8>, u64> {
        let mut t = BTreeMap::new();
        t.insert(key_bytes((b * 256) as u128), 1u64);
        t.insert(key_bytes(b as u128), 2u64);
        t.insert(key_bytes((b + 1) as u128), 3u64);
        t
    };
    let ds = [1u64, 2, 4, 5]; // b = d%3 ∈ {1,2} (b=0 self-collides: b == b*256)

    let body = "var b = (data as u64) % 3u64\n\
                var m = new_map()\n\
                insert(m, b * 256u64, 1u64)\n\
                insert(m, b, 2u64)\n\
                insert(m, b + 1u64, 3u64)\n\
                var ks = keys(m)\n\
                return ((ks[0] as u64) * 10000u64 + (ks[1] as u64) * 100u64 + (ks[2] as u64)) as bytes";
    let cases: Vec<(Vec<u8>, Vec<u8>)> = ds
        .into_iter()
        .map(|d| {
            let ks: Vec<u64> =
                oracle(d % 3).keys().map(|k| {
                    let mut w = [0u8; 8];
                    w[8 - k.len()..].copy_from_slice(k);
                    u64::from_be_bytes(w)
                }).collect();
            let v = ks[0] * 10000 + ks[1] * 100 + ks[2];
            (d.to_be_bytes().to_vec(), v.to_be_bytes().to_vec())
        })
        .collect();
    expect_ok(body, &cases);

    let body_v = "var b = (data as u64) % 3u64\n\
                  var m = new_map()\n\
                  insert(m, b * 256u64, 1u64)\n\
                  insert(m, b, 2u64)\n\
                  insert(m, b + 1u64, 3u64)\n\
                  var vs = values(m)\n\
                  return ((vs[0] as u64) * 10000u64 + (vs[1] as u64) * 100u64 + (vs[2] as u64)) as bytes";
    let cases_v: Vec<(Vec<u8>, Vec<u8>)> = ds
        .into_iter()
        .map(|d| {
            let vs: Vec<u64> = oracle(d % 3).values().copied().collect();
            let v = vs[0] * 10000 + vs[1] * 100 + vs[2];
            (d.to_be_bytes().to_vec(), v.to_be_bytes().to_vec())
        })
        .collect();
    expect_ok(body_v, &cases_v);

    // byte-level: the smallest dynamic key's bytes are the canonical form
    let body_k = "var b = (data as u64) % 3u64\n\
                  var m = new_map()\n\
                  insert(m, b * 256u64, 1u64)\n\
                  insert(m, b, 2u64)\n\
                  var ks = keys(m)\n\
                  return ks[0]";
    let cases_k: Vec<(Vec<u8>, Vec<u8>)> = ds
        .into_iter()
        .map(|d| {
            let first = oracle(d % 3).keys().next().unwrap().clone();
            (d.to_be_bytes().to_vec(), first)
        })
        .collect();
    expect_ok(body_k, &cases_k);
}

/* ================================================================== */
/* 7. MERGE order with dynamic keys (84 on duplicate).                 */
/* ================================================================== */

#[test]
fn b2_merge_order_and_duplicate_dynamic() {
    // x={b:1, b+2:2}, y={b+1:3} → merged keys b, b+1, b+2 (sorted)
    let body = "var b = (data as u64) % 2u64\n\
                var x = new_map()\n\
                var y = new_map()\n\
                insert(x, b, 1u64)\n\
                insert(x, b + 2u64, 2u64)\n\
                insert(y, b + 1u64, 3u64)\n\
                merge(x, y)\n\
                var ks = keys(x)\n\
                return ((ks[0] as u64) * 10000u64 + (ks[1] as u64) * 100u64 + (ks[2] as u64)) as bytes";
    let cases: Vec<(Vec<u8>, Vec<u8>)> = [0u64, 1, 2, 3]
        .into_iter()
        .map(|d| {
            let b = d % 2;
            let v = b * 10000 + (b + 1) * 100 + (b + 2);
            (d.to_be_bytes().to_vec(), v.to_be_bytes().to_vec())
        })
        .collect();
    expect_ok(body, &cases);

    // values keep the sorted-key pairing: 1, 3, 2
    let body_v = "var b = (data as u64) % 2u64\n\
                  var x = new_map()\n\
                  var y = new_map()\n\
                  insert(x, b, 1u64)\n\
                  insert(x, b + 2u64, 2u64)\n\
                  insert(y, b + 1u64, 3u64)\n\
                  merge(x, y)\n\
                  var vs = values(x)\n\
                  return ((vs[0] as u64) * 10000u64 + (vs[1] as u64) * 100u64 + (vs[2] as u64)) as bytes";
    let cases_v: Vec<(Vec<u8>, Vec<u8>)> = [0u64, 1, 4]
        .into_iter()
        .map(|d| (d.to_be_bytes().to_vec(), 10302u64.to_be_bytes().to_vec()))
        .collect();
    expect_ok(body_v, &cases_v);

    // MERGE duplicate dynamic key → 84
    expect_fault(
        "var k = data as u64\n\
         var x = new_map()\n\
         var y = new_map()\n\
         insert(x, k, 1u64)\n\
         insert(y, k, 2u64)\n\
         merge(x, y)\n\
         return 0u64 as bytes",
        &[(0u64.to_be_bytes().to_vec(), 84), (u64::MAX.to_be_bytes().to_vec(), 84)],
    );
    // PACKMAP duplicate with a dynamic key → 84
    expect_fault(
        "var m = map{1u64 : 2u64, (data as u64) : 3u64}\nreturn length(m) as bytes",
        &[(1u64.to_be_bytes().to_vec(), 84)],
    );
}

/* ================================================================== */
/* 8. u128 keys: hi/lo split across the arm boundary (byte 7 | 8).     */
/* ================================================================== */

#[test]
fn b2_u128_hi_lo_boundary_keys() {
    // first nonzero byte at u128 BE position 7 (top of the lo half) and
    // position 8 (bottom of the hi half) — the hi/lo register split
    let vs = [
        1u128 << 64,          // [01, 00×8] — first nonzero at BE index 7
        (1u128 << 64) + 1,    // …with a nonzero tail
        1u128 << 72,          // first nonzero at BE index 8 (hi half)
        (1u128 << 72) | 0xFF, // hi-half pattern with tail
        u128::MAX,
        (u128::MAX >> 8) << 8, // all ones except the last byte
    ];
    let body = "var m = new_map()\ninsert(m, data as u128, 5u64)\nvar ks = keys(m)\nreturn ks[0]";
    let cases: Vec<(Vec<u8>, Vec<u8>)> =
        vs.iter().map(|v| (be_input(*v, 16), key_bytes(*v))).collect();
    expect_ok(body, &cases);

    // lookup + remove roundtrip on the same boundary set
    let body2 = "var m = new_map()\n\
                 insert(m, data as u128, 7u64)\n\
                 var r = (m[data as u128] as u64)\n\
                 remove(m, data as u128)\n\
                 return (r * 10u64 + length(m)) as bytes";
    let cases2: Vec<(Vec<u8>, Vec<u8>)> = vs
        .iter()
        .map(|v| (be_input(*v, 16), 70u64.to_be_bytes().to_vec()))
        .collect();
    expect_ok(body2, &cases2);
}

/* ================================================================== */
/* 9. Interpreter-vs-lowered byte identity on a mixed dynamic map.     */
/* ================================================================== */

#[test]
fn b2_mixed_width_map_byte_identity() {
    // one map, five dynamic keys of five widths (plus zero and collisions)
    let body = "var m = new_map()\n\
                insert(m, (data as u8) as u64, 1u64)\n\
                insert(m, (data as u16) as u64, 2u64)\n\
                insert(m, (data as u32) as u64, 3u64)\n\
                insert(m, data as u64, 4u64)\n\
                insert(m, data as u128, 5u64)\n\
                var ks = keys(m)\n\
                return (length(m) * 100000u64 + (ks[0] as u64)) as bytes";
    // every width reads the SAME input prefix under its own cast: with
    // data = 8 identical bytes v (v != 0, v != 0xFF), the five casts give
    // five distinct keys except the u8/u16/u32 zero-extension families —
    // the interpreter decides; all four engines must match byte-for-byte.
    let inputs: Vec<Vec<u8>> = [
        vec![0u8; 16],                                // all zero everywhere
        {
            let mut d = vec![0u8; 16];
            d[15] = 1;
            d
        }, // 0x01 in every width → ONE canonical slot
        vec![1u8; 16],                                // 0x0101…01 in every width
        vec![0xFF; 16],                               // all-ones in every width
    ]
    .to_vec();
    agree4(body, &inputs);
    // and the full boundary sweep agreement on the plain u64 form
    let body2 = "var m = new_map()\ninsert(m, data as u64, 5u64)\nvar ks = keys(m)\nreturn ks[0]";
    let inputs2: Vec<Vec<u8>> = boundary_values(8).into_iter().map(|v| be_input(v, 8)).collect();
    agree4(body2, &inputs2);
    // smoke: the constant-key forms must keep agreeing too (the rewrite
    // also serves constant keys until B1 pools them)
    agree4("var m = new_map()\ninsert(m, 0u64, 1u64)\ninsert(m, 1u64, 2u64)\ninsert(m, 256u64, 3u64)\nvar ks = keys(m)\nreturn ks[2]", &one());
}
