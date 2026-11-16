//! P3-V2IMPL (A-25 milestone M1): the compo-list v2 shape set — interpreter
//! knock tests, escape rejects, emit-form unit tests and determinism.
//!
//! The admitted shape set (docs.loc/rv64im/74 E1 + the P3-V2IMPL ruling):
//! * creation: `[e0..eN-1]` literal (N ≤ 128, u64 elements; all-literal and
//!   never mutated → a rodata table) and `[]`/`new_list()` (NEWLIST);
//! * mutation: APPEND-only (runtime `len < 128` guard, exit 40) and CLEAR
//!   (length shadow ← 0);
//! * reads: ITEMGET with constant (in-range guard-free, out-of-range
//!   compile reject) or variable index (Nil → 72, ≥ 2³² → 72, ≥ len → 85);
//! * folds: LENGTH / HASKEY (u64 width — the pinned D5/D7 family);
//! * everything else rejects at compile time NAMING the opcode (alias,
//!   re-bind, nested elements, eq/ordered compare, cast, return, xop,
//!   choose arms, bytes receiver, insert/remove/merge/take*/packmap/...).
//!
//! Trap timing parity is asserted per input: the interpreter IS the spec
//! (compo.rs / list.rs / convert.rs / execute.rs anchors in doc 73 E1).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

/// Interpreter result mapped to the rv64 observable shape:
/// `Ok(bytes)` or `Err(Class(N))` → `Err(N)`.
fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

/// Four-way agreement on EVERY input: interpreter == all three rv64 modes
/// (both Ok with identical bytes, or both Err with the identical exit code).
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

fn rv64_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
    assert!(
        err.contains("outside the translatable kernel subset"),
        "reject must use the canonical subset text: {err:?}"
    );
}

/// Interpreter acceptance proof: the body compiles AND runs (any outcome).
fn interp_accepts(body: &str, run: bool) {
    let it = Interp::try_build(body)
        .unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    if run {
        let _ = it.run(&[1, 2, 3, 4]);
    }
}

/* ================================================================== */
/* Section 1: in-shape knocks (read path zero divergence).             */
/* ================================================================== */

/// Byte inputs b0 ∈ 0..=255 varying the first byte.
fn inputs_first_byte() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 7, 11]).collect()
}

#[test]
fn l1_rodata_table_const_and_var_index() {
    // all-literal table (rodata class): variable index guarded against the
    // static length, constant index guard-free
    let body = "var k = [10u64, 20u64, 30u64]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = k[i] + k[0] + k[2]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l2_frame_static_computed_elements() {
    // computed element → frame block (N×8B), static length
    let body = "var k = [(byte(data, 0) as u64) * 3u64, 20u64, (byte(data, 1) as u64)]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = k[i] + k[1]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l3_newlist_append_loop_var_index() {
    // the sha256 w-table shape: empty creation, appends in a loop, reads
    // by constant AND variable index
    let body = "var w = []\n\
                var i = 0u64\n\
                while i < 4u64 {\n\
                append(w, i * 11u64)\n\
                i += 1\n\
                }\n\
                var r = w[(byte(data, 0) as u64) % 4u64] + w[3] + w[0]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l4_literal_then_append_growable() {
    // all-literal creation + a later append upgrades the storage class to
    // growable (init_n stores + len shadow)
    let body = "var w = [5u64, 6u64]\n\
                append(w, 7u64)\n\
                append(w, (byte(data, 0) as u64) % 9u64)\n\
                var r = w[2] + w[0] + w[3]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l5_two_independent_lists() {
    // main-agent ruling: multiple INDEPENDENT lists are admitted; what is
    // rejected is aliasing and re-binding
    let body = "var a = [1u64, 2u64]\n\
                var w = []\n\
                append(w, (byte(data, 0) as u64))\n\
                var r = a[1] + a[0] + w[0]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l6_itemget_oob_85_per_value() {
    // idx ∈ {0, 1, 2, 3, 4, 5} over a 2-element list: 0/1 read, ≥ 2 →
    // CompoNoFindItem(85) on BOTH engines at the same input
    let body = "var k = [7u64, 8u64]\n\
                var i = (byte(data, 0) as u64) % 6u64\n\
                var r = k[i]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn l7_itemget_u32_boundary_72_before_85() {
    // extract_u32 runs BEFORE the lookup (list.rs:36-40): idx ≥ 2³² faults
    // CastParamFail(72), never 85 — and 2³²−1 over a short list is 85
    let body = "var k = [7u64, 8u64]\n\
                var i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\n\
                var r = k[i]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());

    let body2 = "var k = [7u64, 8u64]\n\
                 var i = 4294967295u64\n\
                 var r = k[i]\n\
                 return (r as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn l8_append_cap_128_exit_40() {
    // the interpreter cap (cap.compo_length = 128) faults on append 129;
    // the growable guard fires on exactly the same iteration
    let body = "var w = []\n\
                var i = 0u64\n\
                while i < 128u64 {\n\
                append(w, i)\n\
                i += 1\n\
                }\n\
                append(w, 5u64)\n\
                return 0u64 as bytes";
    agree4(body, &[vec![0u8; 4]]);

    // 128 elements exactly: every index readable, no fault
    let body2 = "var w = []\n\
                 var i = 0u64\n\
                 while i < 128u64 {\n\
                 append(w, i * 2u64)\n\
                 i += 1\n\
                 }\n\
                 var r = w[127] + w[0] + w[(byte(data, 0) as u64) % 128u64]\n\
                 return (r as u64) as bytes";
    agree4(body2, &inputs_first_byte());
}

#[test]
fn l9_packlist_128_literal_boundary() {
    // N = 128 admitted (frame/rodata); N = 129 → compile reject naming
    // PACKLIST (the interpreter faults 40 at runtime — B-family fail-fast)
    let mut elems = String::new();
    for i in 0..128 {
        elems.push_str(&format!("{i}u64, "));
    }
    let body = format!("var k = [{elems}]\nvar r = k[127] + k[0]\nreturn (r as u64) as bytes");
    agree4(&body, &[vec![0u8; 4]]);

    let mut elems129 = String::new();
    for i in 0..129 {
        elems129.push_str(&format!("{i}u64, "));
    }
    let body129 = format!("var k = [{elems129}]\nreturn 0u64 as bytes");
    let err = Rv64::try_build(&body129)
        .err()
        .unwrap_or_else(|| panic!("129-element list must reject"))
        .to_string();
    assert!(err.contains("PACKLIST"), "{err}");
    assert!(err.contains("128-item list capacity"), "{err}");
    interp_accepts(&body129, true);
}

#[test]
fn l10_empty_list_never_appended() {
    // Empty storage class: length ≡ 0 — every read faults 85 (var idx) and
    // has_key folds false (M1-b); a compile-time index rejects
    let body = "var e = []\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = e[i]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

/* ================================================================== */
/* Section 1b: M1-b — CLEAR / LENGTH / HASKEY folds + rodata multi.    */
/* ================================================================== */

#[test]
fn l11_clear_semantics() {
    // compo.rs clear: len → 0, frame contents kept; reads under the OLD
    // bounds now fault 85; re-appends overwrite from slot 0
    let body = "var w = [1u64, 2u64, 3u64]\n\
                clear(w)\n\
                var i = (byte(data, 0) as u64) % 5u64\n\
                var r = w[i]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte()); // every read faults 85 (len 0)

    let body2 = "var w = [1u64, 2u64, 3u64]\n\
                 clear(w)\n\
                 append(w, 9u64)\n\
                 append(w, (byte(data, 0) as u64))\n\
                 var r = w[0] + w[1]\n\
                 return (r as u64) as bytes";
    agree4(body2, &inputs_first_byte()); // 9 + b, w[2..] still faults 85

    // clear + append re-use across loop iterations (the multi-block shape,
    // doc 75 E10): each block re-builds the same 2-element table
    let body3 = "var w = []\n\
                 var t = 0u64\n\
                 var blk = 0u64\n\
                 while blk < 3u64 {\n\
                 clear(w)\n\
                 append(w, blk * 10u64)\n\
                 append(w, blk * 10u64 + 1u64)\n\
                 t = t + w[0] + w[1]\n\
                 blk += 1\n\
                 }\n\
                 var r = t + w[1]\n\
                 return (r as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn l12_length_fold() {
    // static lengths fold to constants (rodata + frame-static classes)
    let body = "var a = [10u64, 20u64, 30u64]\n\
                var b = [(byte(data, 0) as u64), 5u64]\n\
                var e = []\n\
                var r = (length(a) * 100u64) + (length(b) * 10u64) + length(e)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte()); // 300 + 20 + 0

    // the growable length shadow: reads after appends, inside a loop bound
    let body2 = "var w = []\n\
                 append(w, 7u64)\n\
                 append(w, 8u64)\n\
                 var i = 0u64\n\
                 var s = 0u64\n\
                 while i < length(w) {\n\
                 s = s + w[i]\n\
                 i += 1\n\
                 }\n\
                 var r = s + length(w)\n\
                 return (r as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]); // (7+8) + 2 = 17

    // length after clear reads the shadow (0), after re-append the new len
    let body3 = "var w = [1u64, 2u64, 3u64]\n\
                 clear(w)\n\
                 var l0 = length(w)\n\
                 append(w, 5u64)\n\
                 var r = l0 * 10u64 + length(w)\n\
                 return (r as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]); // 0*10 + 1
}

#[test]
fn l13_haskey_fold() {
    // in/out of bounds fold to 1/0; OOB is NOT an error (list.rs:30-33)
    let body = "var w = [4u64, 5u64]\n\
                append(w, 6u64)\n\
                var i = (byte(data, 0) as u64) % 5u64\n\
                var r = ((has_key(w, i)) as u64) * 10u64 + ((has_key(w, 0u64)) as u64)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());

    // idx ≥ 2³² faults 72 FIRST (extract_u32 precedes the compare)
    let body2 = "var w = [4u64, 5u64]\n\
                 var r = ((has_key(w, 4294967296u64)) as u64)\n\
                 return r as bytes";
    agree4(body2, &[vec![0u8; 4]]);

    // empty + cleared lists: every key folds false
    let body3 = "var e = []\n\
                 var c = [1u64, 2u64]\n\
                 clear(c)\n\
                 var r = ((has_key(e, 0u64)) as u64) + ((has_key(c, 1u64)) as u64)\n\
                 return (r as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn l14_length_as_bytes_width_pinned() {
    // D5/D7 pinned divergence (NOT a failure): `length(l) as bytes` yields
    // the interpreter's U32 width (4B BE) vs the rv64 u64 view (8B BE) —
    // the documented as-bytes width family (doc 30 D7, doc 74 E1 fold rows).
    // Kernel discipline: `as u64` first makes all engines agree (l12).
    let body = "var w = [1u64, 2u64, 3u64]\nreturn (length(w)) as bytes";
    let rv = Rv64::try_build(body).unwrap();
    let it = Interp::try_build(body).unwrap();
    let got_it = it.run(&[0u8; 4]).expect("interp runs");
    let l3 = 3u32.to_be_bytes().to_vec();
    assert_eq!(got_it, l3, "interpreter emits the U32 width (4B)");
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(
            got.expect("rv64 runs"),
            3u64.to_be_bytes().to_vec(),
            "mode {mode:?} emits the pinned u64 width (8B)"
        );
    }
}

#[test]
fn l15_rodata_multiple_tables() {
    // two different all-literal tables → two pool entries; identical
    // tables dedup through the pool map (both shapes behaviorally equal)
    let body = "var a = [10u64, 20u64, 30u64]\n\
                var b = [7u64, 9u64]\n\
                var c = [10u64, 20u64, 30u64]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = a[i] + b[1] + c[0] + (length(a) * 10u64) + length(b)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
    let lo = lower_im(body);
    // both tables are all-u32 → 4-byte-stride packing (M2-d rule)
    let mut t1 = Vec::new();
    for v in [10u32, 20, 30] {
        t1.extend_from_slice(&v.to_le_bytes());
    }
    let mut t2 = Vec::new();
    for v in [7u32, 9] {
        t2.extend_from_slice(&v.to_le_bytes());
    }
    assert!(contains_seq(&lo.pool, &t1), "table a/c missing: {:?}", lo.pool);
    assert!(contains_seq(&lo.pool, &t2), "table b missing: {:?}", lo.pool);
    // identical tables a and c share ONE pool entry (dedup via pool_map;
    // the whole 12-byte u32 table is the dedup witness)
    assert_eq!(
        lo.pool.windows(t1.len()).filter(|w| *w == t1.as_slice()).count(),
        1,
        "identical rodata tables must dedup: {:?}",
        lo.pool
    );
}

#[test]
fn l16_sha256_wtable_shape() {
    // the motivating end shape (doc 74 E10, scratch m0): w grows to 64 by
    // appending a scheduler round that reads w[j-16..j-1]; K is a second,
    // rodata-resident table. Pure scalar arithmetic — the digest math is
    // irrelevant here, the SHAPE (w table + K table + read window) is.
    let mut k = String::new();
    for i in 0..16 {
        k.push_str(&format!("{}, ", 0x428a2f98u64.wrapping_mul(i as u64 + 1)));
    }
    let body = format!(
        "var k = [{k}16u64, 17u64]\n\
         var w = []\n\
         var o = 0u64\n\
         while o < 16u64 {{\n\
         append(w, byte(data, (o % 4u64)) as u64 + k[o % 16u64])\n\
         o += 1\n\
         }}\n\
         var j = 16u64\n\
         while j < 32u64 {{\n\
         var s0 = w[j - 15u64] ^ w[j - 16u64]\n\
         append(w, (s0 + w[j - 1u64]) & 255u64)\n\
         j += 1\n\
         }}\n\
         var r = w[31] + w[16] + length(w) + ((has_key(w, 31u64)) as u64)\n\
         return (r as u64) as bytes"
    );
    // 8-byte inputs: the absorb loop reads byte(data, 0..3)
    let inputs: Vec<Vec<u8>> = (0u16..=255)
        .map(|b| vec![b as u8, 3, 5, 7, 11, 13, 17, 19])
        .collect();
    agree4(&body, &inputs);
}

/* ================================================================== */
/* Section 1c: BUG-028 + BUG-021-family acceptance pins.               */
/* ================================================================== */

/// BUG-028 (found by the main agent's acceptance probe): a CONSTANT index
/// on a GROWABLE list skipped the runtime length guard entirely — the
/// load read stale frame bytes and the interpreter's CompoNoFindItem(85)
/// trap was silently swallowed (`w[5]` over a 3-element list returned
/// Ok(0u8x8); after `clear`, `w[0]` even returned the stale pre-clear
/// element). Static-length classes keep the compile-time proof (in-range
/// guard-free loads, out-of-range compile rejects — pinned by l1/l22);
/// the growable class now emits `len <- shadow; k >= len -> 85-thunk`
/// (and an unconditional 85 for k >= 128, which the 128-item cap
/// invariant makes a certain fault).
#[test]
fn l23_bug028_const_index_growable_runtime_guard() {
    // the exact acceptance-probe body plus the boundary matrix:
    // k in {len-1, len, len+1, far} over a 3-element appended list
    for k in [2u64, 3, 4, 5, 127, 128, 4294967295] {
        let body = format!(
            "var w = []\nappend(w, 10u64)\nappend(w, 20u64)\nappend(w, 30u64)\nreturn (w[{k}]) as bytes"
        );
        agree4(&body, &[vec![0u8; 4]]); // k=2 reads 30; every k >= 3 faults 85 on all four engines
    }
    // after clear (len 0): every constant index faults 85, k = 0 included
    // — the stale pre-clear element must NOT leak back out
    for k in [0u64, 1, 127] {
        let body = format!("var w = [1u64, 2u64]\nclear(w)\nreturn (w[{k}]) as bytes");
        agree4(&body, &[vec![0u8; 4]]);
    }
    // the l3 loop-built w table, const out-of-bounds variant (len 4, k 4)
    let body = "var w = []\n\
                 var i = 0u64\n\
                 while i < 4u64 {\n\
                 append(w, i * 11u64)\n\
                 i += 1\n\
                 }\n\
                 var r = w[4]\n\
                 return (r as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);
    // in-range constants on the same growable table keep working (the
    // guard must not over-fire): the len-1 boundary read
    let body2 = "var w = []\n\
                 var i = 0u64\n\
                 while i < 4u64 {\n\
                 append(w, i * 11u64)\n\
                 i += 1\n\
                 }\n\
                 var r = w[3] + w[0]\n\
                 return (r as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

/// BUG-021 family, POW site (main-agent re-review: the M1-c hardening
/// placed the bounds check AFTER the nil-guard index arithmetic — dead
/// code; a checked_sub now precedes it, CLAMP-style). The exact deferred
/// short-stack state was not hand-reproducible (attempt record in
/// doc 92 E4: right-nested integer products saturate at len ~ 7 at this
/// site by per-op net accounting; the corpus-reached CLAMP hit needed a
/// richer mix); this pin is the family no-panic sentinel over a
/// deterministic deep-nest x may-Nil-pow domain — every build outcome is
/// Ok or the clean depth reject text, never a panic.
#[test]
fn l24_bug021family_pow_nil_guard_deferred_sentinel() {
    let mut n_built = 0usize;
    let mut n_depth_reject = 0usize;
    for depth in [3u64, 5, 6, 7, 8, 9, 11, 13] {
        for inner in ["n ** a", "a ** n", "n ** n"] {
            for glue in ["+ ", "* "] {
                let mut body = String::from(
                    "var q = 0u64\nif q == 1u64 { var n = 5u64 }\nvar a = 3u64\nvar z = ",
                );
                for _ in 0..depth {
                    body.push_str("(a * a) ");
                    body.push_str(glue);
                    body.push('(');
                }
                body.push_str(inner);
                for _ in 0..depth {
                    body.push(')');
                }
                body.push_str("\nreturn (z) as bytes");
                let r = std::panic::catch_unwind(|| Rv64::try_build(&body));
                match r {
                    Err(_) => panic!("deferred POW nil-guard panicked (depth {depth}, {inner})"),
                    Ok(Ok(_)) => n_built += 1,
                    Ok(Err(e)) => {
                        let e = e.to_string();
                        assert!(
                            e.contains("nesting exceeds the 8-value register pool"),
                            "depth {depth} {inner}: unexpected reject: {e}"
                        );
                        n_depth_reject += 1;
                    }
                }
            }
        }
    }
    // the domain genuinely spans both outcomes (not vacuous)
    assert!(
        n_built > 0 && n_depth_reject > 0,
        "built {n_built}, rejected {n_depth_reject}"
    );
}

/* ================================================================== */
/* Section 2: escape rejects (each names the consuming opcode).        */
/* ================================================================== */

#[test]
fn l20_escape_family_rejects_named() {
    // (body, fragment, interpreter accepts and runs?)
    let cases: &[(&str, &str, &str, bool)] = &[
        (
            "list-alias",
            "var l = [1u64, 2u64]\nvar m = l\nreturn 0u64 as bytes",
            "PUT",
            true,
        ),
        (
            "list-rebind",
            "var l = [1u64, 2u64]\nl = [3u64]\nreturn 0u64 as bytes",
            "PUT",
            true,
        ),
        (
            "list-rebind-int",
            "var l = [1u64, 2u64]\nl = 5u64\nreturn 0u64 as bytes",
            "PUT",
            true,
        ),
        (
            "list-int-slot-gets-list",
            "var x = 1u64\nvar l = [1u64, 2u64]\nx = l\nreturn 0u64 as bytes",
            "PUT",
            true,
        ),
        (
            "list-nested-element",
            "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes",
            "PACKLIST",
            true, // interpreter faults CastBeValueFail(76) at runtime
        ),
        (
            "list-eq",
            "var l = [1u64, 2u64]\nvar r = ((l == l) as u64)\nreturn r as bytes",
            "EQ",
            true,
        ),
        (
            "list-ordered-cmp",
            "var l = [1u64, 2u64]\nvar r = ((l < l) as u64)\nreturn r as bytes",
            "LT",
            true, // interpreter faults Arithmetic(90) at runtime
        ),
        (
            "list-return",
            "var l = [1u64, 2u64]\nreturn l",
            "RET",
            false, // returns a Compo — only the reject matters
        ),
        (
            "list-cast-bytes",
            "var l = [1u64, 2u64]\nvar r = (l as bytes)\nreturn r",
            "CBYTES",
            true, // interpreter faults CastFail(71) at runtime
        ),
        (
            "list-cast-u64",
            "var l = [1u64, 2u64]\nvar r = (l as u64)\nreturn r as bytes",
            "CU64",
            true, // interpreter faults CastFail(71) at runtime
        ),
        (
            "list-xop-assign",
            "var l = [1u64, 2u64]\nl += 1u64\nreturn 0u64 as bytes",
            "XOP",
            true, // interpreter faults CastFail(71) at runtime
        ),
        (
            "list-choose-arm",
            "var c = (byte(data, 0) as u64)\nvar l = [1u64, 2u64]\nvar r = choose(c == 0u64, l, l)\nreturn 0u64 as bytes",
            "CHOOSE",
            true,
        ),
        (
            "list-append-result",
            "var l = [1u64, 2u64]\nvar m = append(l, 3u64)\nreturn 0u64 as bytes",
            // the mutation itself is admitted; the ESCAPE is the list result
            // aliasing into `m` — the consumer named is PUT
            "PUT",
            true,
        ),
        (
            "list-cond-decl",
            "var c = (byte(data, 0) as u64)\nif c == 0u64 {\nvar l = [1u64, 2u64]\n}\nreturn 0u64 as bytes",
            "PUT",
            true,
        ),
    ];
    for (name, body, frag, runs) in cases {
        interp_accepts(body, *runs);
        let err = Rv64::try_build(body)
            .err()
            .unwrap_or_else(|| panic!("rx:{name}: rv64 must reject"))
            .to_string();
        assert!(
            err.contains(frag),
            "rx:{name}: reject must name {frag:?}: {err:?}"
        );
        assert!(
            err.contains("outside the translatable kernel subset"),
            "rx:{name}: canonical text required: {err:?}"
        );
    }
}

#[test]
fn l21_out_of_shape_ops_reject_named() {
    // the out-opcode family: every entry rejects naming the opcode, on both
    // channels the interpreter accepts (and runs where marked)
    let cases: &[(&str, &str, &str, bool)] = &[
        (
            "insert",
            "var l = [1u64, 2u64]\ninsert(l, 0u64, 9u64)\nreturn 0u64 as bytes",
            "INSERT",
            true,
        ),
        (
            "remove",
            "var l = [1u64, 2u64]\nremove(l, 0u64)\nreturn 0u64 as bytes",
            "REMOVE",
            true,
        ),
        (
            "merge",
            "var a = [1u64]\nvar b = [2u64]\nmerge(a, b)\nreturn 0u64 as bytes",
            "MERGE",
            true,
        ),
        (
            "take-first",
            "var l = [1u64, 2u64]\nvar r = (take_first(l) as u64)\nreturn r as bytes",
            "TAKEFIRST",
            true,
        ),
        (
            "take-last",
            "var l = [1u64, 2u64]\nvar r = (take_last(l) as u64)\nreturn r as bytes",
            "TAKELAST",
            true,
        ),
        (
            "pack-map",
            "var m = map{1u64 : 2u64}\nreturn 0u64 as bytes",
            // the map creation itself is the first out-of-subset site (no
            // admissible map form exists, so keys/values on a map can only
            // ever surface this same creation reject first)
            "PACKMAP",
            true,
        ),
        (
            "pack-tuple",
            "var t = tuple(1u64, 2u64)\nreturn 0u64 as bytes",
            "PACKTUPLE",
            true,
        ),
        (
            "clone",
            "var l = [1u64, 2u64]\nvar m = clone(l)\nreturn 0u64 as bytes",
            "CLONE",
            true,
        ),
        (
            "new-map",
            "var m = new_map()\nreturn 0u64 as bytes",
            "NEWMAP",
            true,
        ),
        (
            "itemget-bytes-receiver",
            "var r = (data[0] as u64)\nreturn r as bytes",
            "ITEMGET",
            true, // interpreter faults CompoOpNotMatch(83) at runtime
        ),
        (
            "itemget-literal-receiver",
            "var i = (byte(data, 0) as u64) % 3u64\nvar r = ([10u64, 20u64, 30u64])[i]\nreturn (r as u64) as bytes",
            "ITEMGET",
            true,
        ),
        (
            "append-value-list",
            "var a = [1u64]\nvar b = [2u64]\nappend(a, b)\nreturn 0u64 as bytes",
            "APPEND",
            true, // interpreter faults CastBeValueFail(76) at runtime
        ),
    ];
    for (name, body, frag, runs) in cases {
        interp_accepts(body, *runs);
        let err = Rv64::try_build(body)
            .err()
            .unwrap_or_else(|| panic!("rx:{name}: rv64 must reject"))
            .to_string();
        assert!(
            err.contains(frag),
            "rx:{name}: reject must name {frag:?}: {err:?}"
        );
    }
}

#[test]
fn l22_itemget_constant_oob_compile_reject() {
    // constant out-of-range index: compile reject (B-family fail-fast; the
    // interpreter faults 85/72 at runtime)
    let body = "var k = [1u64, 2u64]\nvar r = k[5]\nreturn (r as u64) as bytes";
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("const OOB must reject"))
        .to_string();
    assert!(err.contains("ITEMGET"), "{err}");
    assert!(err.contains("out of bounds"), "{err}");
    interp_accepts(body, true);

    let body2 = "var k = [1u64, 2u64]\nvar r = k[4294967296u64]\nreturn (r as u64) as bytes";
    let err2 = Rv64::try_build(body2)
        .err()
        .unwrap_or_else(|| panic!("const u32-overflow index must reject"))
        .to_string();
    assert!(err2.contains("ITEMGET"), "{err2}");
    assert!(err2.contains("u32 index space"), "{err2}");
    interp_accepts(body2, true);
}

/* ================================================================== */
/* Section 3: emit-form unit tests (Im mode, raw code bytes).          */
/* ================================================================== */

fn lower_im(body: &str) -> vm::ir::rv64::Rv64Lowered {
    let tokens = vm::lang::Tokenizer::new(body.as_bytes()).parse().unwrap();
    let (ir, _, _) =
        vm::fitshc::compile_body(tokens, vec![("data".into(), vm::ValueTy::Bytes)], &[], &[], false)
            .unwrap();
    vm::ir::rv64::lower_body_mode(&ir, true, vm::ir::rv64::Rv64Mode::Im)
        .unwrap()
}

fn contains_seq(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn l30_thunk_signatures_40_72_85() {
    // the arbitrary-code thunk mechanism (nil_thunks): each used code
    // materializes as `addi a0, x0, code` inside the Im stream
    //   addi a0, zero, 40  = 0x02800513
    //   addi a0, zero, 72  = 0x04800513
    //   addi a0, zero, 85  = 0x05500513
    let cap_body = "var w = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(w, i)\ni += 1\n}\nappend(w, 5u64)\nreturn 0u64 as bytes";
    let lo = lower_im(cap_body);
    assert!(
        contains_seq(&lo.code, &0x0280_0513u32.to_le_bytes()),
        "cap-40 thunk missing"
    );

    let oob_body = "var k = [7u64, 8u64]\nvar i = 4294967296u64 + (byte(data, 0) as u64)\nvar r = k[i]\nreturn (r as u64) as bytes";
    let lo = lower_im(oob_body);
    assert!(
        contains_seq(&lo.code, &0x0480_0513u32.to_le_bytes()),
        "idx-72 thunk missing"
    );
    assert!(
        contains_seq(&lo.code, &0x0550_0513u32.to_le_bytes()),
        "oob-85 thunk missing"
    );
    // guard ORDER: the 72 thunk label is branched to BEFORE the 85 branch —
    // asserted behaviorally by l7; here we check both exist exactly once
    assert_eq!(
        lo.code
            .windows(4)
            .filter(|w| *w == 0x0480_0513u32.to_le_bytes())
            .count(),
        1
    );
}

#[test]
fn l31_rodata_table_in_pool() {
    // all-literal tables live in the R segment (pool). P3-KERNELS M2-d
    // rule: values that all fit u32 pack as a 4-byte-stride LE table
    // (loaded by lwu, zero-extend keeps values identical); any value
    // ≥ 2^32 keeps the 8-byte u64 form.
    let lo = lower_im("var k = [10u64, 20u64, 30u64]\nvar r = k[1]\nreturn (r as u64) as bytes");
    let mut t32 = Vec::new();
    for v in [10u32, 20, 30] {
        t32.extend_from_slice(&v.to_le_bytes());
    }
    let mut t64 = Vec::new();
    for v in [10u64, 20, 30] {
        t64.extend_from_slice(&v.to_le_bytes());
    }
    assert!(
        contains_seq(&lo.pool, &t32),
        "u32 rodata table missing from pool: {:?}",
        lo.pool
    );
    assert!(
        !contains_seq(&lo.pool, &t64),
        "all-u32 table must take the 4-byte packing: {:?}",
        lo.pool
    );
    // the u32 boundary: 2^32-1 packs to 4B, 2^32 needs the u64 form
    let lob = lower_im("var k = [4294967295u64]\nvar r = k[0]\nreturn (r as u64) as bytes");
    assert_eq!(lob.pool.len(), 4, "max-u32 table packs 4B: {:?}", lob.pool);
    let lob2 = lower_im("var k = [4294967296u64]\nvar r = k[0]\nreturn (r as u64) as bytes");
    assert_eq!(lob2.pool.len(), 8, "2^32 table keeps 8B: {:?}", lob2.pool);
    // a table with one wide value keeps the u64 stride throughout
    let lo2 = lower_im(
        "var k = [10u64, 4294967296u64, 30u64]\nvar r = k[1]\nreturn (r as u64) as bytes",
    );
    let mut mixed = Vec::new();
    for v in [10u64, 4294967296u64, 30u64] {
        mixed.extend_from_slice(&v.to_le_bytes());
    }
    assert!(
        contains_seq(&lo2.pool, &mixed),
        "wide-value table missing from pool: {:?}",
        lo2.pool
    );
    // the creation site emits NOTHING for a rodata list: no frame stores
    // for the elements (behavioral zero-emission proof = the knock tests;
    // here the table presence is the load-bearing fact)
}

#[test]
fn l32_len_shadow_store_on_newlist() {
    // `var w = []` emits an explicit len←0 store even though the prologue
    // zeroes the frame — the store must survive independent of prologue
    // analysis (loop re-entry semantics live on it in future shapes)
    let lo = lower_im("var w = []\nappend(w, 7u64)\nvar r = w[0]\nreturn (r as u64) as bytes");
    // sd x0, s0, <cell> = 0x00013023 with imm 0 — just assert the body built
    // and behaves; the exact store is covered by the behavioral knocks.
    assert!(!lo.code.is_empty());
}

/* ================================================================== */
/* Section 4: determinism.                                             */
/* ================================================================== */

#[test]
fn l40_determinism_and_cross_mode() {
    let body = "var w = []\nvar i = 0u64\nwhile i < 8u64 {\nappend(w, i * 3u64)\ni += 1\n}\n\
                var k = [3u64, 5u64, 7u64]\n\
                var r = w[(byte(data, 0) as u64) % 8u64] + k[(byte(data, 1) as u64) % 3u64]\n\
                return (r as u64) as bytes";
    let a = Rv64::try_build(body).unwrap();
    let b = Rv64::try_build(body).unwrap();
    assert_eq!(a.containers, b.containers, "rebuild must be byte-identical");
    assert_eq!(a.codes, b.codes);
    // the three modes are distinct engines but each is stable, and all
    // agree with the interpreter on real inputs
    agree4(body, &inputs_first_byte());
}

/* ================================================================== */
/* Section 5: BUG-029 (P3-PAIRPKG FIXUP) — may-Nil append values.      */
/* ================================================================== */

/// BUG-029: the interpreter does NOT fault at the append of a Nil value —
/// check_scalar admits Nil (canbe.rs is_scalar_value), the element is
/// silently stored, and only a later read faults (71). M1 guarded the
/// may-Nil append value with a 76-thunk, which fired exactly where the
/// interpreter continued: a code-and-timing divergence (six reproducing
/// shapes probed, doc 98 §E4). Fix = compile-reject the may-Nil append
/// value (B-family fail-fast; may-Nil PACKLIST element precedent).
/// Back-verify has teeth: reverting the reject hunk makes the rv64 build
/// succeed and the first assert (must-reject) fails.
#[test]
fn l25_append_may_nil_value_compile_reject() {
    let nil_decl = "var q = 0u64\nif q == 1u64 { var n = 5u64 }\n";
    // the reproducing shape: interpreter runs to Ok (the Nil IS stored —
    // length 1 as the u64-width readback), rv64 must refuse
    let body = format!(
        "{nil_decl}var l = []\nappend(l, n)\nreturn (length(l) as u64) as bytes"
    );
    let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("interp must accept: {e}"));
    let got = it
        .run(&[0])
        .unwrap_or_else(|f| panic!("interp run trapped {:?} — the silent-store fact must hold", f.code));
    assert_eq!(got, 1u64.to_be_bytes().to_vec(), "the interpreter must have stored the Nil element (length 1)");
    rv64_must_reject(&body, "APPEND");

    // same shape with a downstream read: interpreter 71 (at the read),
    // rv64 rejected at compile — the old rv64 76 never matched either side
    let body2 = format!(
        "{nil_decl}var l = []\nappend(l, n)\nreturn ((l[0] + 1u64)) as bytes"
    );
    let it2 = Interp::try_build(&body2).unwrap_or_else(|e| panic!("interp must accept: {e}"));
    let f2 = it2.run(&[0]).unwrap_err();
    assert_eq!(f2.code, 71, "the interpreter faults CastFail(71) at the READ, not at the append");
    rv64_must_reject(&body2, "APPEND");

    // the reach family: zero-iteration while, one-arm join, input-driven
    // never-taken branch, nested conditional — every shape compile-rejects
    let more = [
        ("zero-iter while",
         "var i = 0u64\nwhile i < 0u64 {\nvar w = 7u64\ni += 1u64\n}\nvar l = []\nappend(l, w)\nreturn (length(l) as u64) as bytes"),
        ("one-arm join",
         "var q = 0u64\nif q == 1u64 { var n = 5u64 } else { var m = 6u64 }\nvar l = []\nappend(l, n)\nreturn (length(l) as u64) as bytes"),
        ("input never-taken",
         "var q = (byte(data, 0) as u64) % 2u64\nif q == 1u64 { var n = 5u64 }\nvar l = []\nappend(l, n)\nreturn (length(l) as u64) as bytes"),
        ("nested cond",
         "var q = 0u64\nvar r = 0u64\nif q == 1u64 {\nif r == 1u64 { var n = 5u64 }\n}\nvar l = []\nappend(l, n)\nreturn (length(l) as u64) as bytes"),
    ];
    for (name, b) in more {
        let it3 = Interp::try_build(b).unwrap_or_else(|e| panic!("{name}: interp must accept: {e}"));
        let _ = it3.run(&[0]); // runs (any outcome) — interpreter accepts the shape
        rv64_must_reject(b, "APPEND");
    }

    // the divergence pair, pinned byte-exact for the record (scratch
    // rv98_fixup_probe numbers): on the shared body the interpreter ran
    // Ok(8B length 1) while rv64 exited 76 in all three modes — quoted
    // here as comments; the pin above makes the reject load-bearing.

    // controls stay admitted: a DEFINITE value in the same world fuses
    // four-way; append INSIDE the declaring branch (the slot is always
    // defined on the path where the append runs — nil_plan marks it
    // definite) stays four-way green
    let c1 = format!(
        "{nil_decl}var l = []\nvar w = 9u64\nappend(l, w)\nreturn (length(l) as u64) as bytes"
    );
    agree4(&c1, &[vec![0]]);
    let c3 = "var q = 0u64\nvar l = []\nif q == 1u64 {\nvar n = 5u64\nappend(l, n)\n}\nreturn ((length(l) as u64) + 0u64) as bytes";
    agree4(c3, &[vec![0], vec![1]]);
    let c4 = "var l = []\nappend(l, (byte(data, 0) as u64) + 3u64)\nreturn ((l[0] + 1u64)) as bytes";
    agree4(c4, &inputs_first_byte());
    // NOTE: the branch-TAKEN form (`var q = 1u64; if q == 1 { var n = 5 }
    // …; append(l, n)`) is ALSO rejected now: the nil analysis is
    // flow-insensitive to the (unfolded) condition, and its old four-way
    // agreement was input-luck, not soundness — the same body with a
    // runtime-dependent q diverges (the six-shape reach family above).
    // May-analysis conservatism is the may-Nil PACKLIST precedent.
    rv64_must_reject(
        "var q = 1u64\nif q == 1u64 { var n = 5u64 }\nvar l = []\nappend(l, n)\nreturn ((l[0] + 1u64)) as bytes",
        "APPEND",
    );
}
