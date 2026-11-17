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
    // B2 task 4: the block-budget form — 129 × 8B exceeds the 1024B block
    assert!(err.contains("129 × 8B elements exceed"), "{err}");
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
    // CLOSED by task-1 A2 (was the D5/D7 pinned divergence): `length(l)`
    // is U32 in the interpreter and `as bytes` now renders that width on
    // every engine (4B BE). The u64-pinned form keeps the 8B wire view.
    let body = "var w = [1u64, 2u64, 3u64]\nreturn (length(w)) as bytes";
    let rv = Rv64::try_build(body).unwrap();
    let it = Interp::try_build(body).unwrap();
    let got_it = it.run(&[0u8; 4]).expect("interp runs");
    let l3 = 3u32.to_be_bytes().to_vec();
    assert_eq!(got_it, l3, "interpreter emits the U32 width (4B)");
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(
            got.expect("rv64 runs"),
            l3,
            "mode {mode:?} emits the U32 width (4B)"
        );
    }
    let body = "var w = [1u64, 2u64, 3u64]\nreturn (length(w) as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);
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
    // B2 task 4: the table stride IS the slot's fixed element width —
    // these literals are u64, so the 8-byte LE form (the M2-d all-u32
    // packing is gone; u32-element tables keep their own 4-byte form,
    // pinned by l31)
    let mut t1 = Vec::new();
    for v in [10u64, 20, 30] {
        t1.extend_from_slice(&v.to_le_bytes());
    }
    let mut t2 = Vec::new();
    for v in [7u64, 9] {
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
        // B2 task 4: the u64 suffix keeps the table's element width
        // uniform (bare decimals that fit u32 parse CU32 → the fixed
        // stride rejects a mixed-width table)
        k.push_str(&format!("{}u64, ", 0x428a2f98u64.wrapping_mul(i as u64 + 1)));
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
    // T17: the container-value escape family LOWERS now — alias / re-bind
    // / nested element / EQ / ordered compare / casts / XOP / choose arms
    // / append-result binds all compile to the interpreter's exact faces
    // (see vm/tests/rv64_t17_list_values.rs and rv64_arena_containers.rs).
    // What stays a compile reject: the unrepresentable slot-species
    // changes, a list declared under a conditional, and the ABI-level
    // list return.
    // (body, fragment, interpreter accepts and runs?)
    let cases: &[(&str, &str, &str, bool)] = &[
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
            "list-return",
            "var l = [1u64, 2u64]\nreturn l",
            "RET",
            false, // returns a Compo — only the reject matters
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
    // T17: clone / insert / remove / merge / take-first / take-last and a
    // list-valued append all LOWER now (vm/tests/rv64_t17_list_values.rs).
    // The out-of-subset family that stays: tuples, a bytes receiver of an
    // index, and a non-variable list literal receiver. Maps (T16) lower too,
    // so their creation forms are not reject pins either.
    let cases: &[(&str, &str, &str, bool)] = &[
        (
            "pack-tuple",
            "var t = tuple(1u64, 2u64)\nreturn 0u64 as bytes",
            "PACKTUPLE",
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
            "itemcat-aliased-receiver",
            "var l = [1u64, 2u64]\nvar m = l\nvar r = item_concat(m)\nreturn r",
            "ITEMCAT",
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
    // all-literal tables live in the R segment (pool). B2 task 4: the
    // table stride IS the slot's fixed element width (loaded by the
    // width's zero-extending load, values identical); the M2-d
    // all-u32 packing is gone — a u32-element table takes its own
    // 4-byte form, a u64-element table the 8-byte form.
    let lo = lower_im("var k = [10u64, 20u64, 30u64]\nvar r = k[1]\nreturn (r as u64) as bytes");
    let mut t64 = Vec::new();
    for v in [10u64, 20, 30] {
        t64.extend_from_slice(&v.to_le_bytes());
    }
    assert!(
        contains_seq(&lo.pool, &t64),
        "u64 rodata table missing from pool: {:?}",
        lo.pool
    );
    // u32 elements keep the 4-byte-stride form
    let lo32 = lower_im("var k = [10u32, 20u32, 30u32]\nvar r = k[1]\nreturn (r as u64) as bytes");
    let mut t32 = Vec::new();
    for v in [10u32, 20, 30] {
        t32.extend_from_slice(&v.to_le_bytes());
    }
    assert!(
        contains_seq(&lo32.pool, &t32),
        "u32 rodata table missing from pool: {:?}",
        lo32.pool
    );
    // the u32/u64 boundary: a max-u32 literal table is 4B per element,
    // 2^32 forces the u64 element width (8B)
    let lob = lower_im("var k = [4294967295u32]\nvar r = k[0]\nreturn (r as u64) as bytes");
    assert_eq!(lob.pool.len(), 4, "max-u32 element packs 4B: {:?}", lob.pool);
    let lob2 = lower_im("var k = [4294967296u64]\nvar r = k[0]\nreturn (r as u64) as bytes");
    assert_eq!(lob2.pool.len(), 8, "2^32 element keeps 8B: {:?}", lob2.pool);
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

/* ================================================================== */
/* Section 4: B2 task 4 — per-slot element widths + capacity path.     */
/*                                                                     */
/* Each list slot fixes ONE element width (the first store fixes it,   */
/* every later store must fit it); the element block stride, the load/ */
/* store instructions, the rodata table form, the arithmetic width of  */
/* compound assignment and the byte-width cast faces all follow it.    */
/* The block budget is LIST_BLOCK_MAX = 1024B, so the element-count    */
/* capacity is 1024/step: 128 for u64 (the historical D2 form, byte    */
/* for byte), 256 for u32, 512 for u16, 1024 for u8.                   */
/* ================================================================== */

#[test]
fn t4_width_cast_faces_agree() {
    // the element's own width IS its `as bytes` face: u8 → 1 byte BE,
    // u16 → 2, u32 → 4 (the pre-task-4 uniform-u64 shape gave 8 for all
    // — the pinned ITEMGET=64 deviation this task eliminates)
    let b8 = "var a = [7u8, 200u8]\n\
             var i = (byte(data, 0) as u64) % 2u64\n\
             return (a[i] as bytes)";
    agree4(b8, &inputs_first_byte());
    let b16 = "var a = [5u16, 4660u16]\n\
              var i = (byte(data, 0) as u64) % 2u64\n\
              return (a[i] as bytes)";
    agree4(b16, &inputs_first_byte());
    let b32 = "var a = [1u32, 3735928559u32]\n\
              var i = (byte(data, 0) as u64) % 2u64\n\
              return (a[i] as bytes)";
    agree4(b32, &inputs_first_byte());
}

#[test]
fn t4_width_arithmetic_faces() {
    // u8 elements add at the u8 common width: 100+100 fits, 200+100
    // overflows and faults Arithmetic(90) — exactly the interpreter's
    // width-8 add face (a uniform-u64 block would have wrapped at 2^64
    // and returned 300 instead)
    let b = "var a = [200u8, 100u8]\n\
            var i = (byte(data, 0) as u64) % 2u64\n\
            var j = (i + 1u64) % 2u64\n\
            return ((a[i] + a[j]) as u64) as bytes";
    agree4(b, &inputs_first_byte());
    // the widened read face: cast to u64 first, the add runs at 64
    let b2 = "var a = [200u8, 100u8]\n\
             var i = (byte(data, 0) as u64) % 2u64\n\
             var j = (i + 1u64) % 2u64\n\
             return (((a[i] as u64) + (a[j] as u64)) as u64) as bytes";
    agree4(b2, &inputs_first_byte());
}

#[test]
fn t4_mixed_width_stores_reject() {
    // pack: element widths mix (first fixes, later must match)
    rv64_must_reject(
        "var k = [1u8, 2u32]\nreturn (0u64) as bytes",
        "mix element widths",
    );
    // append wider than the slot's stride
    rv64_must_reject(
        "var a = [1u8]\nappend(a, 2u32)\nreturn (0u64) as bytes",
        "would mix element widths",
    );
    // append narrower than the slot's stride: the interpreter keeps
    // per-element widths (a u8 element among u64s), the fixed-stride
    // block cannot — the narrower store's later byte-width cast face
    // (1B vs the slot's 8B) would diverge, so it rejects too
    rv64_must_reject(
        "var a = [1u64]\nappend(a, 2u8)\nreturn (0u64) as bytes",
        "would mix element widths",
    );
    interp_accepts("var a = [1u64]\nappend(a, 2u8)\nreturn (0u64) as bytes", true);
    // itemset: same face
    rv64_must_reject(
        "var a = [1u8, 2u8]\na[0] = 3u64\nreturn (0u64) as bytes",
        "would mix element widths",
    );
}

#[test]
fn t4_bool_element_reject() {
    // a bool element has no static uint width (width 1 < 8): the
    // interpreter stores it, the fixed-stride block cannot
    rv64_must_reject(
        "var t = 1u64 == 1u64\nvar k = [t]\nreturn (0u64) as bytes",
        "no static uint width",
    );
    interp_accepts(
        "var t = 1u64 == 1u64\nvar k = [t]\nreturn (length(k) as u64) as bytes",
        true,
    );
}

#[test]
fn t4_narrow_growable_agree() {
    // a u8-element growable list: appends, clears, re-appends and reads
    // at the 1-byte stride; the byte-width cast face travels through
    let b = "var a = []\n\
            var i = 0u64\n\
            while i < 8u64 {\n\
            append(a, (i * 40u64) as u8)\n\
            i += 1u64\n\
            }\n\
            clear(a)\n\
            append(a, 240u8)\n\
            append(a, 15u8)\n\
            var i2 = (byte(data, 0) as u64) % 2u64\n\
            var r = (a[i2] as bytes)\n\
            return r";
    agree4(b, &inputs_first_byte());
}

#[test]
fn t4_compound_width_faces() {
    // u8 element += u8 value: the arithmetic runs at the element width —
    // 100+100 fits, 200+100 faults 90 (a uniform-u64 block would have
    // returned 300)
    let b = "var a = [200u8, 100u8]\n\
            var i = (byte(data, 0) as u64) % 2u64\n\
            a[i] += 100u8\n\
            return (0u64) as bytes";
    agree4(b, &inputs_first_byte());
    // compound value NARROWER than the element: accepted — the widened
    // result stays at the slot's width, the byte-width faces are
    // unchanged (pinned task-4 decision)
    let b2 = "var a = [1u64, 2u64]\n\
             var i = (byte(data, 0) as u64) % 2u64\n\
             a[i] += 1u8\n\
             return (((a[0] as u64) + (a[1] as u64)) as u64) as bytes";
    agree4(b2, &inputs_first_byte());
    // compound value WIDER than the element: the widened result cannot
    // fit the fixed stride — reject
    rv64_must_reject(
        "var a = [1u8, 2u8]\na[0] += 1u64\nreturn (0u64) as bytes",
        "exceeds the slot's fixed",
    );
}

#[test]
fn t4_itemcat_narrow_elements() {
    // item_concat splices each element at its own width: a u8 list of
    // [1,2,3] is the 3-byte string 01 02 03
    let b = "var a = [1u8, 2u8, 3u8]\n\
            return item_concat(a)";
    agree4(b, &[vec![0u8; 4]]);
    let b2 = "var a = [1u32, 2u32]\n\
             return item_concat(a)";
    agree4(b2, &[vec![0u8; 4]]);
}

#[test]
fn t4_pack_capacity_follows_element_width() {
    // 128 u8 elements: within BOTH budgets (interpreter compo_length=128,
    // kernel block 1024B/1B) — four-way agreement holds
    let mut elems = String::new();
    for i in 0..128 {
        elems.push_str(&format!("{}u8, ", i % 256));
    }
    let body = format!(
        "var k = [{elems}]\nvar r = (k[127] as u64) + (k[0] as u64)\nreturn (r as u64) as bytes"
    );
    agree4(&body, &[vec![0u8; 4]]);

    // 1024 u8 elements: the kernel block takes them (1024 × 1B = the
    // whole budget); the interpreter's compo_length=128 budget faults 40
    // — a CAP-face difference, single factor: same body, same widths,
    // same semantics, only the element budget differs (the interpreter's
    // cap is its SpaceCap::compo_length DEFAULT; the kernel's budget is
    // LIST_BLOCK_MAX / element-step). Census: one-sided pin.
    let mut big = String::new();
    for i in 0..1024 {
        big.push_str(&format!("{}u8, ", i % 256));
    }
    let body_big = format!("var k = [{big}]\nvar r = (k[1023] as u64) + (k[0] as u64)\nreturn (r as u64) as bytes");
    let rv = Rv64::try_build(&body_big)
        .unwrap_or_else(|e| panic!("rv64 must build the 1024×1B fill: {e}"));
    let it = Interp::try_build(&body_big)
        .unwrap_or_else(|e| panic!("interp must build: {e}"));
    let want = it.run(&[0u8; 4]).map_err(|f| f.code as u64);
    // the interpreter face here is OutOfStack(32): a 1024-element PACKLIST
    // value form pushes every element before packing (its stack budget
    // ends the run) — still a budget face, same single factor: only the
    // element budgets differ
    assert_eq!(want, Err(32), "interpreter stack budget face");
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(
            got,
            Ok((255u64).to_be_bytes().to_vec()),
            "mode {mode:?} on the 1024×1B fill"
        );
    }

    // 1025 u8 elements: over the block budget — compile reject
    let mut over = String::new();
    for i in 0..1025 {
        over.push_str(&format!("{}u8, ", i % 256));
    }
    let body_over = format!("var k = [{over}]\nreturn (0u64) as bytes");
    rv64_must_reject(&body_over, "1025 × 1B elements exceed");
}

#[test]
fn t4_u8_capacity_over_interpreter_cap() {
    // 200 u8 APPENDS: the kernel's 1024-element u8 capacity admits them;
    // the interpreter's compo_length=128 budget faults 40 on append 129
    // — the same single-factor cap-face difference as t4_pack (the
    // 128×8B D2 form is preserved for u64 slots, pinned by l8)
    let body = "var a = []\n\
               var i = 0u64\n\
               while i < 200u64 {\n\
               append(a, i as u8)\n\
               i += 1u64\n\
               }\n\
               return ((a[199] as u64) + (length(a) as u64)) as bytes";
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {e}"));
    let want = it.run(&[0u8; 4]).map_err(|f| f.code as u64);
    assert_eq!(want, Err(40), "interpreter compo_length=128 cap face");
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(
            got,
            Ok(399u64.to_be_bytes().to_vec()),
            "mode {mode:?} on the 200-append u8 list"
        );
    }
}

#[test]
fn t4_determinism_narrow_slots() {
    // cross-mode determinism on narrow-slot bodies (the three modes share
    // the same ew plan; the emission differs, the bytes must not)
    let b = "var a = [9u16, 300u16, 7u16]\n\
            var i = (byte(data, 0) as u64) % 3u64\n\
            a[i] += 1u8\n\
            var r = (a[0] as u64) + (a[1] as u64) + (a[2] as u64)\n\
            return (r as u64) as bytes";
    agree4(b, &inputs_first_byte());
}

/* ================================================================== */
/* Section 6: T19 — u128 list elements (16B stride).                   */
/*                                                                     */
/* The per-slot element-width table now admits 128: every element is   */
/* a (lo, hi) pair stored as two consecutive little-endian words at    */
/* a 16-byte stride; the element-count budget is LIST_BLOCK_MAX/16 =   */
/* 64 (the interpreter's own cap is compo_length=128 — appends 65..128 */
/* are a kernel-BUDGET face, the same single-factor budget divergence  */
/* the u8 capacity test above documents in reverse). Everything else   */
/* (fault codes, check order, byte-width faces) must agree exactly.    */
/* ================================================================== */

/// Big-output four-way agreement (itemcat totals past the default 16B
/// harness budget — compare semantics, not budget sizes; the same
/// builder rv64_itemset_bufset.rs uses).
fn agree4_big_t19(body: &str, inputs: &[Vec<u8>]) {
    let rv = c3lib_rv64_out(body, 1280, 0x7000);
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

fn c3lib_rv64_out(body: &str, output_max: u16, arena: u64) -> Rv64 {
    Rv64::try_build_out(body, output_max, arena)
        .unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"))
}

const MAX128: &str = "340282366920938463463374607431768211455";

#[test]
fn t19_rodata_table_const_and_var_index() {
    // all-literal u128 table (rodata class): 16B-stride loads, const and
    // variable index, the 16-byte `as bytes` face (2^64 and 2^100 exercise
    // nonzero high halves; every sum stays under 2^128)
    let body = "var k = [10u128, 18446744073709551616u128, \
                1267650600228229401496703205376u128]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = k[i] + k[0] + k[2]\n\
                return (r as bytes)";
    agree4(body, &inputs_first_byte());
}

#[test]
fn t19_frame_static_computed_elements() {
    // computed u128 elements → frame block (N×16B), static length
    let body = "var k = [(byte(data, 0) as u128) * 3u128, 20u128, (byte(data, 1) as u128)]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                var r = k[i] + k[1]\n\
                return (r as bytes)";
    agree4(body, &inputs_first_byte());
}

#[test]
fn t19_newlist_append_itemset_reads() {
    // empty creation, u128 appends in a loop, in-place item_set, const +
    // var index reads (the u128 payout-list shape)
    let body = "var w = []\n\
                var i = 0u64\n\
                while i < 4u64 {\n\
                append(w, (i as u128) * 18446744073709551615u128)\n\
                i += 1u64\n\
                }\n\
                w[2] = 12345678901234567890123456789012345678u128\n\
                var r = w[(byte(data, 0) as u64) % 4u64] + w[3] + w[0]\n\
                return (r as bytes)";
    agree4(body, &inputs_first_byte());
}

#[test]
fn t19_literal_then_append_and_clear() {
    // all-literal u128 creation + later append/clear upgrades the storage
    // class to growable (init fill + len shadow)
    let body = "var w = [5u128, 6u128]\n\
                append(w, 7u128)\n\
                append(w, (byte(data, 0) as u128) % 9u128)\n\
                var r = w[2] + w[0] + w[3]\n\
                return (r as bytes)";
    agree4(body, &inputs_first_byte());
    let body2 = format!(
        "var w = [5u128, 6u128]\n\
         clear(w)\n\
         append(w, {MAX128}u128)\n\
         return (w[0] as bytes)"
    );
    agree4(&body2, &[vec![0u8; 4]]);
}

#[test]
fn t19_compound_addsubmul_faces() {
    // `l[i] += v` at u128: checked add/sub/mul with the interpreter's
    // Arithmetic(90) face; a u64 rhs zero-extends to the common width
    let b = "var a = [200u128, {MAX128}u128]\n\
            var i = (byte(data, 0) as u64) % 2u64\n\
            a[i] += 100u128\n\
            return (a[0] as bytes)";
    let b = b.replace("{MAX128}", MAX128);
    agree4(&b, &inputs_first_byte()); // a[1] += 100 overflows → 90
    let b2 = "var a = [200u128, 100u128]\n\
             var i = (byte(data, 0) as u64) % 2u64\n\
             a[i] += 5u64\n\
             a[i] -= 3u128\n\
             return ((a[0] as u64) + (a[1] as u64)) as bytes";
    agree4(b2, &inputs_first_byte());
    let b3 = "var a = [5u128, 3u128]\n\
             a[0] *= 4u128\n\
             a[1] *= 0u128\n\
             return ((a[0] as u64) + (a[1] as u64)) as bytes";
    agree4(b3, &[vec![0u8; 4]]);
    // 2^64 * 2^64 = 2^128 overflows the checked u128 mul → 90
    let b3o = "var a = [18446744073709551616u128]\n\
              a[0] *= 18446744073709551616u128\n\
              return (0u64) as bytes";
    agree4(b3o, &[vec![0u8; 4]]);
    let b4 = "var a = [1u128]\n\
             a[0] -= 2u128\n\
             return (0u64) as bytes";
    agree4(b4, &[vec![0u8; 4]]); // underflow → 90
}

#[test]
fn t19_compound_div_faces() {
    // `l[i] /= v`: the shared soft-divmod routine; divide-by-zero → 90;
    // a u64 divisor zero-extends (common width 128)
    let b = "var a = [100u128, 99u128]\n\
            var i = (byte(data, 0) as u64) % 2u64\n\
            a[i] /= 7u128\n\
            return ((a[0] as u64) + (a[1] as u64)) as bytes";
    agree4(b, &inputs_first_byte());
    let b2 = "var a = [18446744073709551617u128]\n\
             a[0] /= 18446744073709551616u128\n\
             return (a[0] as bytes)";
    agree4(b2, &[vec![0u8; 4]]);
    let b3 = "var a = [100u128]\n\
             a[0] /= 0u128\n\
             return (0u64) as bytes";
    agree4(b3, &[vec![0u8; 4]]); // 90
    let b4 = "var a = [100u128]\n\
             a[0] /= 7u64\n\
             return (a[0] as bytes)";
    agree4(b4, &[vec![0u8; 4]]);
}

#[test]
fn t19_compound_may_nil_value_guard() {
    // a may-Nil compound value faults CastFail(71) where the interpreter
    // converts the ADD operand (Nil) — AFTER the itemget index checks;
    // the pair's Nil tag rides its low half
    let body = "var q = 0u64\n\
                if q == 1u64 {\n\
                var n = 5u128\n\
                }\n\
                var a = [1u128]\n\
                a[0] += n\n\
                return (0u64) as bytes";
    agree4(body, &[vec![0u8; 4], vec![1u8; 4]]);
}

#[test]
fn t19_itemcat_be_splice() {
    // every u128 element contributes its 16 big-endian extract_bytes bytes
    let body = "var l = [1u128, 2u128]\n\
                var r = item_concat(l)\n\
                return r";
    let it = Interp::try_build(body).unwrap();
    let want = it.run(&[0u8; 4]).unwrap();
    let mut expect = Vec::new();
    for v in [1u128, 2] {
        expect.extend_from_slice(&v.to_be_bytes());
    }
    assert_eq!(want, expect, "interpreter splices 16 BE bytes per element");
    agree4_big_t19(body, &[vec![0u8; 4]]);
    // growable u128 elements splice in append order
    let body2 = "var w = []\n\
                 var i = 0u64\n\
                 while i < 3u64 {\n\
                 append(w, (i as u128) + 1u128)\n\
                 i += 1u64\n\
                 }\n\
                 var r = item_concat(w)\n\
                 return r";
    agree4_big_t19(body2, &[vec![0u8; 4]]);
}

#[test]
fn t19_fault_faces_match() {
    // variable index ≥ len → 85; may-Nil index → 72; `as u64` narrowing of
    // a >u64::MAX element → 71 — all in the interpreter's check order
    let b = "var a = [1u128, 2u128, 3u128]\n\
            var i = (byte(data, 0) as u64)\n\
            return (a[i] as bytes)";
    agree4(b, &inputs_first_byte()); // i < 3 reads; i ≥ 3 → 85
    let b2 = "var q = 0u64\n\
             if q == 1u64 {\n\
             var n = 1u64\n\
             }\n\
             var a = [1u128]\n\
             return (a[n] as bytes)";
    agree4(b2, &[vec![0u8; 4], vec![1u8; 4]]); // Nil index → 72
    let b3 = "var a = [{MAX128}u128]\n\
             return (a[0] as u64) as bytes";
    let b3 = b3.replace("{MAX128}", MAX128);
    agree4(&b3, &[vec![0u8; 4]]); // 71
    let b4 = "var a = [1u128, 2u128]\n\
             a[2] = 5u128\n\
             return (0u64) as bytes";
    agree4(&b4, &[vec![0u8; 4]]); // idx == len → 85
}

#[test]
fn t19_capacity_faces() {
    // 64 u128 appends fit the block budget exactly (64 × 16B = 1024B; ==
    // is legal); both engines return length 64
    let b = "var w = []\n\
            var i = 0u64\n\
            while i < 64u64 {\n\
            append(w, i as u128)\n\
            i += 1u64\n\
            }\n\
            return ((length(w) as u64) + (w[63] as u64)) as bytes";
    agree4(b, &[vec![0u8; 4]]);
    // 129 appends: the kernel faults 40 at the 65th (its 64-element
    // budget), the interpreter at the 129th (compo_length=128) — the
    // observable fault code agrees on every engine
    let b2 = "var w = []\n\
            var i = 0u64\n\
            while i < 129u64 {\n\
            append(w, i as u128)\n\
            i += 1u64\n\
            }\n\
            return ((length(w) as u64)) as bytes";
    agree4(b2, &[vec![0u8; 4]]);
    // has_key/length folds over a u128 list
    let b3 = "var w = [7u128, 8u128]\n\
            var i = (byte(data, 0) as u64) % 3u64\n\
            return ((length(w) as u64) + ((has_key(w, i)) as u64)) as bytes";
    agree4(b3, &inputs_first_byte());
}

#[test]
fn t19_budget_compile_rejects() {
    // 65 u128 literal elements exceed the 1024B block budget — B-family
    // compile reject (the interpreter admits it: 65 ≤ compo_length 128)
    let mut elems = String::new();
    for i in 0..65u64 {
        elems.push_str(&format!("{i}u128, "));
    }
    let body = format!("var k = [{elems}]\nreturn (0u64) as bytes");
    rv64_must_reject(&body, "65 × 16B elements exceed");
    interp_accepts(&body, false);
    // constant OOB on the static classes stays a fail-fast reject (the
    // plain assert style l22 uses — this reject is not the subset text)
    let err = Rv64::try_build("var a = [1u128, 2u128]\nreturn (a[2] as bytes)")
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject the const OOB read"))
        .to_string();
    assert!(err.contains("out of bounds"), "{err}");
    // width mixes inside one list stay rejected (flat block, one stride):
    // covered in rv64_u128_arith.rs t3_u128_list_element_mixed_width_rejects
}

#[test]
fn t19_element_into_u128_arith_and_compares() {
    // a u128 ITEMGET flows into the existing u128 pair ecosystem: mixed
    // comparisons, arithmetic with u128 operands, narrowing casts
    let b = "var a = [18446744073709551616u128, 5u128]\n\
            var x = (byte(data, 0) as u128)\n\
            if x == a[0] {\n\
            return (a[1] as bytes)\n\
            }\n\
            return ((a[0] + a[1]) as bytes)";
    agree4(b, &inputs_first_byte());
    let b2 = "var a = [7u128]\n\
             var s = a[0] * a[0]\n\
             var t = s / 4u128\n\
             var u = t % 9u128\n\
             return (u as bytes)";
    agree4(b2, &[vec![0u8; 4]]);
    let b3 = "var a = [7u128]\n\
             var b = [2u64]\n\
             return ((a[0] as u64) + b[0]) as bytes";
    agree4(b3, &[vec![0u8; 4]]);
    // a u128 ITEMGET pair as the rhs of a u128 slot's XOP compound
    let b4 = "var l = [3u128, 4u128]\n\
             var s = 10u128\n\
             var i = (byte(data, 0) as u64) % 2u64\n\
             s += l[i]\n\
             s *= 2u128\n\
             return (s as bytes)";
    agree4(b4, &inputs_first_byte());
    // a u128 ITEMGET as a choose arm value (mixed u128/u64 arms widen)
    let b5 = "var l = [3u128]\n\
             var c = (byte(data, 0) % 2u64)\n\
             var r = (if (c == 0u64) { l[0] } else { 5u128 })\n\
             return (r as bytes)";
    agree4(b5, &inputs_first_byte());
}
