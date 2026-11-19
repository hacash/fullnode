//! T16 — the fitsh→rv64 MAP family differential tests.
//!
//! Every test drives the DEFAULT lowering channel (`c3lib::Rv64`, all three
//! modes {Im, Imc, ImcRemap} round-tripped through the consensus container
//! decoder) against the consensus interpreter: identical Ok bytes or the
//! identical fault code. The arena map model lives in `vm/src/ir/rv64.rs`
//! ("T16: arena map descriptor emission"): a map slot's cell holds the bare
//! arena descriptor pointer (B6 — the historical low-bit species tag had no
//! runtime reader), entries are kept sorted by the canonical
//! key bytes (BTreeMap byte-lex order), and the key bytes are private arena
//! copies.
//!
//! Admitted shapes (T16): NEWMAP/PACKMAP creation bound to a local,
//! INSERT/REMOVE/ITEMSET/MERGE statements on map locals, HASKEY/ITEMGET/
//! LENGTH map reads, and KEYS/VALUES snapshots bound to a list local
//! (uint-valued snapshots are ordinary arena lists; Bytes-valued ones are
//! Bytes-element lists). Returning/copying/comparing a map value stays a
//! compile reject.

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
            assert_eq!(got, want, "mode {mode:?} diverged on input {input:?} of body:\n{body}");
        }
    }
}

fn one() -> Vec<Vec<u8>> {
    vec![vec![1, 2, 3, 4]]
}

fn interp_accepts(body: &str, runs: bool) {
    match Interp::try_build(body) {
        Ok(it) => {
            if runs {
                let _ = it.run(&[0u8; 4]);
            }
        }
        Err(e) => panic!("interpreter must accept {body}: {e}"),
    }
}

fn rv_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(err.contains(frag), "reject must name {frag:?}: {err:?} (body: {body})");
}

/* ================================================================== */
/* Section 1: creation + length.                                       */
/* ================================================================== */

#[test]
fn m1_new_map_and_pack_map_length() {
    agree4("var m = new_map()\nreturn length(m) as bytes", &one());
    agree4("var m = map{1u64 : 2u64}\nreturn length(m) as bytes", &one());
    agree4(
        "var m = map{1u64 : 2u64, 3u64 : 4u64, 5u64 : 6u64}\nreturn length(m) as bytes",
        &one(),
    );
    agree4("var m = map{}\nreturn length(m) as bytes", &one());
}

/* ================================================================== */
/* Section 2: insert / get / haskey / overwrite.                       */
/* ================================================================== */

#[test]
fn m2_insert_itemget_haskey() {
    agree4(
        "var m = new_map()\ninsert(m, 7u64, 40u64)\nreturn (m[7u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 7u64, 40u64)\nreturn (has_key(m, 7u64) as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 7u64, 40u64)\nreturn (has_key(m, 8u64) as u64) as bytes",
        &one(),
    );
    // an existing key overwrites in place; the length does not change
    agree4(
        "var m = map{1u64 : 2u64}\ninsert(m, 1u64, 9u64)\nreturn (m[1u64] * 10u64 + length(m)) as bytes",
        &one(),
    );
    // zero key: canonical [0x00]
    agree4(
        "var m = new_map()\ninsert(m, 0u64, 5u64)\nreturn (m[0u64] as u64) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 3: sorted iteration — KEYS / VALUES.                        */
/* ================================================================== */

#[test]
fn m3_keys_values_sorted_byte_lex() {
    // insert out of order; KEYS must come back in byte-lex order
    agree4(
        "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar ks = keys(m)\nreturn (ks[0] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 100u64 + (ks[2] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar vs = values(m)\nreturn ((vs[0] as u64) * 100u64 + (vs[1] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nvar ks = keys(m)\nreturn ks[0]",
        &one(),
    );
    // u8(1) and u64(1) share the canonical slot
    agree4(
        "var m = new_map()\ninsert(m, 1u8, 5u64)\nreturn (m[1u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 1u64, 5u64)\nreturn (m[1u8] as u64) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 4: remove / item_set / merge.                               */
/* ================================================================== */

#[test]
fn m4_remove_itemset_merge() {
    agree4(
        "var m = map{1u64 : 2u64, 3u64 : 4u64}\nremove(m, 1u64)\nreturn length(m) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64, 3u64 : 4u64}\nremove(m, 1u64)\nreturn (has_key(m, 1u64) as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nm[1u64] = 9u64\nreturn (m[1u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn ((a[1u64] as u64) * 10u64 + (a[3u64] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn length(b) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 5: fault faces (iron rule: identical codes).                */
/* ================================================================== */

#[test]
fn m5_fault_faces() {
    // missing key: item_get 85, remove 85, item_set 85 (never an insert)
    agree4(
        "var m = map{1u64 : 2u64}\nvar r = m[9u64]\nreturn r as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nremove(m, 9u64)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nm[9u64] = 3u64\nreturn 0u64 as bytes",
        &one(),
    );
    // Bool key → 73 (a statically bool-ish comparison key)
    agree4(
        "var m = new_map()\ninsert(m, (1u64 == 1u64), 5u64)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nvar r = m[(1u64 == 1u64)]\nreturn r as bytes",
        &one(),
    );
    // empty Bytes key → 73
    agree4(
        "var m = new_map()\nvar e = buf_left(0, data)\ninsert(m, e, 5u64)\nreturn 0u64 as bytes",
        &one(),
    );
    // duplicate key in pack_map → 84
    agree4(
        "var m = map{1u64 : 2u64, 1u64 : 3u64}\nreturn length(m) as bytes",
        &one(),
    );
    // duplicate key in merge → 84
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = map{1u64 : 3u64}\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
    // self merge → 80
    agree4(
        "var a = map{1u64 : 2u64}\nmerge(a, a)\nreturn length(a) as bytes",
        &one(),
    );
    // alias self merge → 80
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nmerge(a, b)\nreturn length(a) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 6: aliasing shares one object.                              */
/* ================================================================== */

#[test]
fn m6_alias_shares_mutation() {
    agree4(
        "var a = new_map()\nvar b = a\ninsert(a, 1u64, 2u64)\nreturn (b[1u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var a = new_map()\nvar b = a\ninsert(b, 1u64, 2u64)\nreturn (a[1u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nremove(b, 1u64)\nreturn length(a) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 7: capacity faces.                                          */
/* ================================================================== */

#[test]
fn m7_capacity_40() {
    // 128 distinct keys fill a map; the 129th NEW key is 40 on both
    let mut body = String::from("var m = new_map()\n");
    body.push_str("var i = 0u64\n");
    body.push_str("while i < 129u64 { insert(m, i, i)\ni += 1 }\n");
    body.push_str("return length(m) as bytes");
    agree4(&body, &one());
}

/* ================================================================== */
/* Section 8: map values stay rejected as values.                      */
/* ================================================================== */

#[test]
fn m8_map_value_escapes_reject() {
    interp_accepts("var m = new_map()\nreturn m", false);
    rv_must_reject("var m = new_map()\nreturn m", "RET");
    rv_must_reject("var m = new_map()\nvar r = ((m == m) as u64)\nreturn r as bytes", "EQ");
    rv_must_reject("var m = new_map()\nvar x = insert(m, 1u64, 2u64)\nreturn 0u64 as bytes", "PUT");
}


/* ================================================================== */
/* Section 9: bytes keys/values, u128, mixed-width normalization.      */
/* ================================================================== */

#[test]
fn m9_bytes_keys_and_values() {
    // a bytes key (input slice) — stored as a private arena copy
    agree4(
        "var m = new_map()\ninsert(m, buf_left(2, data), 5u64)\nreturn (m[buf_left(2, data)] as u64) as bytes",
        &[vec![7, 9, 3, 4]],
    );
    // different bytes prefixes are distinct keys
    agree4(
        "var m = new_map()\ninsert(m, buf_left(2, data), 5u64)\nreturn (has_key(m, buf_left(3, data)) as u64) as bytes",
        &[vec![7, 9, 3, 4]],
    );
    // a bytes VALUE round-trips through item_get / values()
    agree4(
        "var m = new_map()\ninsert(m, 1u64, buf_left(3, data))\nreturn m[1u64]",
        &[vec![7, 9, 3, 4]],
    );
    agree4(
        "var m = new_map()\ninsert(m, 1u64, buf_left(3, data))\nvar vs = values(m)\nreturn vs[0]",
        &[vec![7, 9, 3, 4]],
    );
    // a bytes key sorts by raw byte-lex order (0x03 < 0x0709)
    agree4(
        "var m = new_map()\ninsert(m, buf_left(2, data), 1u64)\ninsert(m, buf_right(1, data), 2u64)\nvar ks = keys(m)\nreturn ks[0]",
        &[vec![7, 9, 3, 4]],
    );
}

#[test]
fn m10_u128_keys_and_values() {
    let big = "340282366920938463463374607431768211455u128";
    agree4(
        &format!("var m = new_map()\ninsert(m, {big}, 7u64)\nreturn (m[{big}] as u64) as bytes"),
        &one(),
    );
    // u128 value width flows through item_get / values
    agree4(
        &format!("var m = new_map()\ninsert(m, 1u64, {big})\nreturn m[1u64] as bytes"),
        &one(),
    );
    agree4(
        &format!("var m = new_map()\ninsert(m, 1u64, {big})\nvar vs = values(m)\nreturn vs[0] as bytes"),
        &one(),
    );
    // u128(1) and u64(1) share one key slot
    agree4(
        "var m = new_map()\ninsert(m, 1u128, 9u64)\nreturn (m[1u64] as u64) as bytes",
        &one(),
    );
    // narrow uint keys zero-extend to the same canonical bytes
    agree4(
        "var m = new_map()\ninsert(m, 65536u32, 9u64)\nreturn (m[65536u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 1u16, 9u64)\nreturn (m[1u8] as u64) as bytes",
        &one(),
    );
}

#[test]
fn m11_key_byte_lex_order() {
    // 0x00 < 0x01 < 0x0100 < 0x010000 — prefix-shorter-first byte order
    agree4(
        "var m = new_map()\ninsert(m, 256u64, 3u64)\ninsert(m, 1u64, 2u64)\ninsert(m, 0u64, 1u64)\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 10000u64 + (ks[1] as u64) * 100u64 + (ks[2] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\ninsert(m, 65536u64, 3u64)\ninsert(m, 256u64, 2u64)\ninsert(m, 1u64, 1u64)\nvar vs = values(m)\nreturn ((vs[0] as u64) * 100u64 + (vs[2] as u64)) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 10: remove + re-insert, item_set, empty snapshots.          */
/* ================================================================== */

#[test]
fn m12_remove_reinsert_and_snapshots() {
    agree4(
        "var m = map{1u64 : 10u64, 2u64 : 20u64}\nremove(m, 1u64)\ninsert(m, 3u64, 30u64)\nreturn ((m[2u64] as u64) * 100u64 + (m[3u64] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\nvar ks = keys(m)\nreturn length(ks) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\nvar vs = values(m)\nreturn length(vs) as bytes",
        &one(),
    );
    // item_set is an in-place overwrite; a later item_get sees it
    agree4(
        "var m = map{1u64 : 2u64, 2u64 : 3u64}\nitem_set(m, 2u64, 9u64)\nreturn ((m[2u64] as u64) * 10u64 + length(m)) as bytes",
        &one(),
    );
    // the KEYS/VALUES snapshots do not observe later mutations
    agree4(
        "var m = map{1u64 : 2u64}\nvar ks = keys(m)\ninsert(m, 3u64, 4u64)\nreturn length(ks) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 11: map-flow loops.                                         */
/* ================================================================== */

#[test]
fn m13_loops_over_maps() {
    agree4(
        "var m = new_map()\nvar i = 0u64\nwhile i < 20u64 { insert(m, i, i * 2u64)\ni += 1u64 }\nreturn (length(m) as u64 + (m[7u64] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = new_map()\nvar i = 0u64\nwhile i < 10u64 { insert(m, i, i)\ni += 1u64 }\nvar j = 0u64\nvar s = 0u64\nwhile j < 10u64 { s += m[j]\nj += 1u64 }\nreturn s as bytes",
        &one(),
    );
    // insert in one loop, remove in another: the count nets out
    agree4(
        "var m = new_map()\nvar i = 0u64\nwhile i < 12u64 { insert(m, i, i)\ni += 1u64 }\nvar j = 0u64\nwhile j < 5u64 { remove(m, j)\nj += 1u64 }\nreturn length(m) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 12: rejects that stay (B-family compile-time).              */
/* ================================================================== */

#[test]
fn m14_mixed_value_species_reject() {
    interp_accepts(
        "var m = map{1u8 : 2u8}\ninsert(m, 2u64, 3u64)\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var m = map{1u8 : 2u8}\ninsert(m, 2u64, 3u64)\nreturn 0u64 as bytes",
        "INSERT",
    );
    interp_accepts(
        "var a = map{1u8 : 2u8}\nvar b = map{2u64 : 3u64}\nmerge(a, b)\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var a = map{1u8 : 2u8}\nvar b = map{2u64 : 3u64}\nmerge(a, b)\nreturn 0u64 as bytes",
        "MERGE",
    );
    // a compound map item assignment is not in the admitted map set
    interp_accepts("var m = map{1u64 : 2u64}\nm[1u64] += 5u64\nreturn 0u64 as bytes", true);
    rv_must_reject("var m = map{1u64 : 2u64}\nm[1u64] += 5u64\nreturn 0u64 as bytes", "ITEMSET");
}

/* ================================================================== */
/* Section 13: Nil key face, rebinds, non-scalar values.               */
/* ================================================================== */

#[test]
fn m15_nil_key_fault_73() {
    // an unassigned (Nil) local as the key: the interpreter faults 73 at
    // the key extraction; both inputs are covered (q == 1 binds 5)
    let body = "var q = (byte(data, 0) as u64)\nif q == 1u64 { var n = 5u64 }\nvar m = new_map()\ninsert(m, n, 1u64)\nreturn 0u64 as bytes";
    agree4(body, &[vec![1], vec![2]]);
}

#[test]
fn m16_map_rebind() {
    // an un-aliased map slot may be re-bound to a fresh object
    agree4(
        "var m = new_map()\ninsert(m, 1u64, 2u64)\nm = map{3u64 : 4u64}\nreturn ((m[3u64] as u64) * 10u64 + length(m)) as bytes",
        &one(),
    );
    // a re-bind while an alias is live rejects (the alias keeps the old
    // object; the kernel's one-species-per-group model cannot split it)
    interp_accepts(
        "var m = map{1u64 : 2u64}\nvar n = m\nm = map{3u64 : 4u64}\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var m = map{1u64 : 2u64}\nvar n = m\nm = map{3u64 : 4u64}\nreturn 0u64 as bytes",
        "PUT",
    );
    // an unconditional re-bind of the SAME value species is admitted
    agree4(
        "var m = map{1u64 : 2u64}\nvar c = (byte(data, 0) as u64)\nif c == 1u64 { m = map{3u64 : 4u64} }\nreturn length(m) as bytes",
        &[vec![0], vec![1]],
    );
    // a conditional re-bind that changes the species rejects
    interp_accepts(
        "var m = map{1u64 : 2u64}\nvar c = (byte(data, 0) as u64)\nif c == 1u64 { m = map{3u64 : 4u8} }\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var m = map{1u64 : 2u64}\nvar c = (byte(data, 0) as u64)\nif c == 1u64 { m = map{3u64 : 4u8} }\nreturn 0u64 as bytes",
        "PUT",
    );
}

#[test]
fn m16b_conditional_alias_rejects() {
    interp_accepts(
        "var m = map{1u64 : 2u64}\nvar c = (byte(data, 0) as u64)\nif c == 1u64 { var n = m }\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var m = map{1u64 : 2u64}\nvar c = (byte(data, 0) as u64)\nif c == 1u64 { var n = m }\nreturn 0u64 as bytes",
        "PUT",
    );
}

#[test]
fn m17_non_scalar_and_may_nil_values_reject() {
    interp_accepts(
        "var m = new_map()\ninsert(m, 1u64, [2u64])\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var m = new_map()\ninsert(m, 1u64, [2u64])\nreturn 0u64 as bytes",
        "INSERT",
    );
    interp_accepts(
        "var q = (byte(data, 0) as u64)\nif q == 1u64 { var n = 5u64 }\nvar m = new_map()\ninsert(m, 1u64, n)\nreturn 0u64 as bytes",
        true,
    );
    rv_must_reject(
        "var q = (byte(data, 0) as u64)\nif q == 1u64 { var n = 5u64 }\nvar m = new_map()\ninsert(m, 1u64, n)\nreturn 0u64 as bytes",
        "INSERT",
    );
}

/* ================================================================== */
/* Section 14: more shapes — narrow values, merge edges, address key.  */
/* ================================================================== */

#[test]
fn m18_narrow_value_lists_and_haskey() {
    agree4(
        "var m = map{1u64 : 2u8}\nvar vs = values(m)\nreturn (vs[0] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u8, 2u64 : 3u8}\nreturn ((m[1u64] as u64) * 10u64 + (m[2u64] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nvar ks = keys(m)\nreturn (has_key(ks, 0u64) as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u64 : 2u64}\nvar ks = keys(m)\nreturn (has_key(ks, 1u64) as u64) as bytes",
        &one(),
    );
}

#[test]
fn m19_merge_edges() {
    // merge an empty source (kind untouched) and an empty destination
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = new_map()\nmerge(a, b)\nreturn ((length(a) as u64) * 10u64 + (a[1u64] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var a = new_map()\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn ((length(a) as u64) * 10u64 + (a[3u64] as u64)) as bytes",
        &one(),
    );
    // interleaved keys keep the sorted order after the merge
    agree4(
        "var a = map{1u64 : 10u64, 3u64 : 30u64}\nvar b = map{2u64 : 20u64, 4u64 : 40u64}\nmerge(a, b)\nvar ks = keys(a)\nreturn ((ks[1] as u64) * 10u64 + (ks[2] as u64)) as bytes",
        &one(),
    );
    // a merge through an alias mutates the shared object
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nvar c = map{2u64 : 3u64}\nmerge(b, c)\nreturn (a[2u64] as u64) as bytes",
        &one(),
    );
}

#[test]
fn m20_address_keys() {
    agree4(
        "var a = 1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9\nvar m = new_map()\ninsert(m, a, 5u64)\nreturn (m[a] as u64) as bytes",
        &one(),
    );
    agree4(
        "var a = 1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9\nvar m = new_map()\ninsert(m, a, 5u64)\nreturn (has_key(m, a) as u64) as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 15: canonical-key collisions across species/widths.         */
/* ================================================================== */

#[test]
fn m21_canonical_key_collisions() {
    // u8(1) and u64(1) collide in pack_map → 84
    agree4(
        "var m = map{1u8 : 2u64, 1u64 : 3u64}\nreturn 0u64 as bytes",
        &one(),
    );
    // a Bytes key [0x01] collides with the uint key 1 (both canonical
    // minimal-BE [0x01]): insert overwrites, one entry remains
    agree4(
        "var m = new_map()\ninsert(m, 1u64, 5u64)\ninsert(m, buf_left(1, data), 6u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes",
        &[vec![1, 2, 3, 4]],
    );
    // [0x00, 0x01] is NOT canonical for uint 1 (uint 1 is [0x01])
    agree4(
        "var m = new_map()\ninsert(m, 1u64, 5u64)\ninsert(m, buf_left(2, data), 6u64)\nreturn ((has_key(m, 1u64) as u64) * 10u64 + length(m)) as bytes",
        &[vec![0, 1, 3, 4]],
    );
}

/* ================================================================== */
/* Section 16: B1/B3 — constant-key pool staging + compile-time        */
/* PACKMAP layout (docs.loc/fitsh-rv64/survey/container.md §3).       */
/* ================================================================== */

/// A big u128 literal shared by the key/value cases below.
const M_BIG128: &str = "340282366920938463463374607431768211455u128";

#[test]
fn m22_const_key_literals_sort_and_read() {
    // single pair
    agree4(
        "var m = map{7u64 : 70u64}\nreturn ((m[7u64] as u64) * 10u64 + length(m)) as bytes",
        &one(),
    );
    // multi pair, written OUT of sorted order: KEYS/VALUES must come
    // back in the canonical byte-lex order (= the compile-time sort)
    agree4(
        "var m = map{300u64 : 3u64, 2u64 : 1u64, 50u64 : 2u64}\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 10000u64 + (ks[1] as u64) * 100u64 + (ks[2] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = map{300u64 : 3u64, 2u64 : 1u64, 50u64 : 2u64}\nvar vs = values(m)\nreturn ((vs[0] as u64) * 10000u64 + (vs[1] as u64) * 100u64 + (vs[2] as u64)) as bytes",
        &one(),
    );
    agree4(
        "var m = map{300u64 : 3u64, 2u64 : 1u64, 50u64 : 2u64}\nreturn ((m[300u64] as u64) * 100u64 + (m[2u64] as u64)) as bytes",
        &one(),
    );
    // zero key: canonical [0x00], sorts first
    agree4(
        "var m = map{5u64 : 2u64, 0u64 : 1u64}\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 10u64 + length(m)) as bytes",
        &one(),
    );
    agree4(
        "var m = map{0u64 : 9u64}\nreturn (m[0u64] as u64) as bytes",
        &one(),
    );
    // byte-lex, not numeric: 0x00 < 0x01 < 0x0100 < 0x010000
    agree4(
        "var m = map{65536u64 : 4u64, 256u64 : 3u64, 1u64 : 2u64, 0u64 : 1u64}\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 1000u64 + (ks[1] as u64) * 100u64 + (ks[2] as u64) * 10u64 + (ks[3] as u64)) as bytes",
        &one(),
    );
    // u128 constant key (16-byte canonical span) and a u8 constant key
    agree4(
        &format!("var m = map{{{M_BIG128} : 7u64}}\nreturn (m[{M_BIG128}] as u64) as bytes"),
        &one(),
    );
    agree4(
        "var m = map{1u8 : 5u64}\nreturn (m[1u64] as u64) as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u16 : 5u64}\nreturn (has_key(m, 1u128) as u64) as bytes",
        &one(),
    );
    // constant-key literal then REMOVE, re-INSERT, MERGE — every
    // consumer of the pool-staged key bytes
    agree4(
        "var m = map{1u64 : 2u64, 2u64 : 3u64}\nremove(m, 2u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64, 3u64 : 4u64}\nvar b = map{5u64 : 6u64}\nmerge(a, b)\nvar ks = keys(a)\nreturn ((ks[2] as u64) * 10u64 + length(a)) as bytes",
        &one(),
    );
    // non-uint VALUE species through the direct store: bytes and u128
    agree4(
        "var m = map{2u64 : buf_left(2, data), 1u64 : buf_right(2, data)}\nreturn m[2u64]",
        &[vec![7, 9, 3, 4]],
    );
    agree4(
        &format!("var m = map{{1u64 : {M_BIG128}, 2u64 : 1u128}}\nreturn m[1u64] as bytes"),
        &one(),
    );
    agree4(
        &format!("var m = map{{1u64 : {M_BIG128}, 2u64 : 1u128}}\nvar vs = values(m)\nreturn vs[0] as bytes"),
        &one(),
    );
}

#[test]
fn m23_const_key_literal_mixed_dynamic() {
    // one dynamic key pair: the pairwise path runs (pair 0's constant
    // key still takes the B1 pool staging)
    agree4(
        "var k = byte(data, 0) as u64\nvar m = map{100u64 : 1u64, (k) : 2u64}\nreturn ((length(m) as u64) * 1000u64 + (m[100u64] as u64) * 10u64 + (m[k] as u64)) as bytes",
        &[vec![7, 1, 1, 1], vec![100, 1, 1, 1]],
    );
    // dynamic key FIRST, constant second — same agreement
    agree4(
        "var k = byte(data, 0) as u64\nvar m = map{(k) : 2u64, 100u64 : 1u64}\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 10u64 + length(m)) as bytes",
        &[vec![7, 1, 1, 1], vec![100, 1, 1, 1]],
    );
    // the dynamic pair colliding with the constant pair overwrites
    // (insert semantics on the pairwise path)
    agree4(
        "var k = byte(data, 0) as u64\nvar m = map{7u64 : 1u64, (k) : 2u64}\nreturn ((m[7u64] as u64) * 10u64 + length(m)) as bytes",
        &[vec![7, 1, 1, 1], vec![8, 1, 1, 1]],
    );
}

#[test]
fn m24_const_duplicate_key_runtime_84() {
    // plain duplicate: every run traps 84 on all four engines
    agree4(
        "var m = map{1u64 : 2u64, 1u64 : 3u64}\nreturn length(m) as bytes",
        &one(),
    );
    // duplicate in the MIDDLE: the pairs after it never evaluate (the
    // interpreter's pairwise loop order)
    agree4(
        "var m = map{1u64 : 2u64, 1u64 : 3u64, 5u64 : 4u64}\nreturn length(m) as bytes",
        &one(),
    );
    // cross-width collision inside one literal: canonical bytes equal
    agree4(
        "var m = map{1u64 : 2u64, 1u8 : 3u64}\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var m = map{1u16 : 2u64, 1u128 : 3u64}\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var m = map{0u8 : 1u64, 0u64 : 2u64}\nreturn 0u64 as bytes",
        &one(),
    );
    // zero-extension equality: 0x0100 as u16 == 256 as u64
    agree4(
        "var m = map{256u64 : 1u64, 256u16 : 2u64}\nreturn 0u64 as bytes",
        &one(),
    );
}

#[test]
fn m25_const_duplicate_values_evaluate_before_the_84() {
    // observable evaluation order: a pair BEFORE the duplicate carries
    // an input-dependent fault face (u64 division by zero -> 90 — a face
    // both engines agree on; the byte OOB face is the pinned D2 class
    // and cannot serve). The value must evaluate BEFORE the 84: a zero
    // first byte yields 90 on every engine, a nonzero one yields 84.
    agree4(
        "var m = map{1u64 : (100u64 / (byte(data, 0) as u64)), 2u64 : 3u64, 2u64 : 4u64}\nreturn length(m) as bytes",
        &[vec![0, 2, 3], vec![7, 2, 3]],
    );
    // the duplicate pair's OWN value also evaluates first: zero first
    // byte -> 90, nonzero -> 84
    agree4(
        "var m = map{1u64 : 2u64, 2u64 : (100u64 / (byte(data, 0) as u64)), 2u64 : 4u64}\nreturn length(m) as bytes",
        &[vec![0, 2, 3], vec![7, 2, 3]],
    );
}

#[test]
fn m26_const_key_boundaries_128_and_256_pairs() {
    // 127 pairs (n = 254): the largest literal the bytecode
    // interpreter's own operand stack can hold, one under the cap —
    // full agreement end to end
    let mut body = String::from("var m = map{");
    for k in 0..127u64 {
        if k > 0 {
            body.push_str(", ");
        }
        body.push_str(&format!("{k}u64 : {k}u64"));
    }
    body.push_str("}\nvar ks = keys(m)\nreturn ((length(m) as u64) * 1000u64 + (ks[0] as u64) * 10u64 + (ks[126] as u64)) as bytes");
    agree4(&body, &one());

    // Beyond 127 pairs the interpreter's BYTECODE operand stack faults
    // OutOfStack(32) before PACKMAP ever runs (a host resource face,
    // like gas — no semantic reference behavior exists there), and the
    // pre-B3 rv64 pairwise emission could not even pack these sizes
    // (code-size overflow at compile). The rv64-only assertions below
    // pin the direct-store side: 128 pairs exactly fill the capacity
    // (no 40), and the static take_items face covers everything past
    // it (129 pairs = n 258 > 2*cap, and the 256-pair body).
    let lit = |pairs: u64| {
        let mut b = String::from("var m = map{");
        for k in 0..pairs {
            if k > 0 {
                b.push_str(", ");
            }
            b.push_str(&format!("{k}u64 : {k}u64"));
        }
        b.push_str("}\nreturn length(m) as bytes");
        b
    };
    let rv = Rv64::try_build(&lit(128)).unwrap();
    for (mode, got) in rv.run(&[1, 2, 3, 4]) {
        assert_eq!(got, Ok(128u32.to_be_bytes().to_vec()), "128 pairs, {mode:?}");
    }
    for pairs in [129u64, 256] {
        let rv = Rv64::try_build(&lit(pairs)).unwrap();
        for (mode, got) in rv.run(&[1, 2, 3, 4]) {
            assert_eq!(got, Err(40), "{pairs} pairs must 40, {mode:?}");
        }
    }
}

#[test]
fn m27_u128_cbytes_direct_offset() {
    // B10: the u128 scalar->bytes cast (emit_cbytes128) round-trips its
    // 16 big-endian bytes through output and back into a u128 fold
    agree4(
        &format!("var x = {M_BIG128}\nreturn x as bytes"),
        &one(),
    );
    agree4(
        "var x = 1u128\nreturn x as bytes",
        &one(),
    );
    agree4(
        "var x = 0u128\nreturn x as bytes",
        &one(),
    );
    agree4(
        "var x = 18446744073709551616u128\nreturn x as bytes",
        &one(),
    );
    // both halves exercise every byte lane: 0x0102...0f10 (decimal
    // form — the tokenizer takes hex only as a bytes literal)
    agree4(
        "var x = 1339673755198158349044581307228491536u128\nreturn x as bytes",
        &one(),
    );
}

/* ================================================================== */
/* Section 17: B4/B6/B7 — find scan positions, direct single-key ops,  */
/* the two-pointer MERGE sweep, tag-free descriptors.                  */
/* ================================================================== */

/// A helper body builder: fill map `name` with `n` entries keyed
/// `base + i` (dynamic keys through the loop counter). `iv` names the
/// loop counter (the frontend rejects re-binding a variable name).
fn fill_loop(name: &str, n: u64, base: u64, iv: &str) -> String {
    format!(
        "var {name} = new_map()\nvar {iv} = 0u64\nwhile {iv} < {n}u64 {{ insert({name}, {base}u64 + {iv}, {iv})\n{iv} += 1u64 }}\n"
    )
}

#[test]
fn m28_find_scan_positions_hit_and_miss() {
    // a 7-entry dynamic map; the same lookup site hits the FIRST, a
    // MIDDLE, and the LAST entry (and misses past the end) depending on
    // the input byte — the find scan walks the byte-lex ring over every
    // position in every mode
    let body = "var m = new_map()\nvar i = 0u64\nwhile i < 7u64 { insert(m, i * 11u64, i * 3u64)\ni += 1u64 }\nvar k = (byte(data, 0) as u64) * 11u64\nreturn (m[k] as u64) as bytes";
    let inputs: Vec<Vec<u8>> = (0u8..8).map(|b| vec![b, 9, 9, 9]).collect();
    agree4(body, &inputs);
    // the same positions through has_key (no 85 face — 0/1 both ways)
    let body = "var m = new_map()\nvar i = 0u64\nwhile i < 7u64 { insert(m, i * 11u64, i * 3u64)\ni += 1u64 }\nvar k = (byte(data, 0) as u64) * 11u64\nreturn (has_key(m, k) as u64) as bytes";
    agree4(body, &inputs);
    // and through remove (miss → 85)
    let body = "var m = new_map()\nvar i = 0u64\nwhile i < 7u64 { insert(m, i * 11u64, i * 3u64)\ni += 1u64 }\nvar k = (byte(data, 0) as u64) * 11u64\nremove(m, k)\nreturn (length(m) as u64) as bytes";
    agree4(body, &inputs);
}

#[test]
fn m29_single_key_ops_hit_miss_85() {
    // item_get / remove / item_set hit and miss (85) with dynamic keys —
    // the B4(c) direct-mode find (live pool key pair, no staging round
    // trip) agrees face-by-face with the interpreter
    let hit = vec![3u8, 9, 9, 9];
    let miss = vec![4u8, 9, 9, 9];
    let m = "var m = new_map()\ninsert(m, 1u64, 10u64)\ninsert(m, 3u64, 30u64)\ninsert(m, 5u64, 50u64)\nvar k = (byte(data, 0) as u64) + (byte(data, 1) as u64)\n";
    agree4(&format!("{m}var r = m[k]\nreturn (r as u64) as bytes"), &[hit.clone(), miss.clone()]);
    agree4(&format!("{m}remove(m, k)\nreturn (length(m) as u64) as bytes"), &[hit.clone(), miss.clone()]);
    agree4(
        &format!("{m}item_set(m, k, 99u64)\nreturn ((m[k] as u64) * 10u64 + length(m)) as bytes"),
        &[hit.clone(), miss.clone()],
    );
    // overwrite in place: length unchanged, value replaced
    agree4(
        &format!("{m}insert(m, k, 77u64)\nreturn ((m[k] as u64) * 10u64 + length(m)) as bytes"),
        &[hit.clone(), miss.clone()],
    );
}

#[test]
fn m30_find_prefix_collisions_byte_lex() {
    // the canonical bytes 1=[01] 256=[01 00] 65536=[01 00 00] are byte-lex
    // prefixes of each other: a lookup target shorter than the entry ends
    // the compare on the entry-longer face, a longer target on the
    // target-longer face, and the equal-length face is the find hit —
    // every arm of the folded min() bound is exercised dynamically
    let body = "var m = new_map()\ninsert(m, 65536u64, 3u64)\ninsert(m, 256u64, 2u64)\ninsert(m, 1u64, 1u64)\nvar s = byte(data, 0) as u64\nvar k = 1u64 * s * s\nreturn (m[k] as u64) as bytes";
    // s=1 → key 1 (hit first entry); s=2 → key 4 (miss → 85); the
    // multiplication form keeps the key dynamic for the normalization
    agree4(body, &[vec![1, 9, 9, 9], vec![2, 9, 9, 9], vec![3, 9, 9, 9]]);
    // the full prefix chain through has_key (0/1, no fault)
    let body = "var m = new_map()\ninsert(m, 65536u64, 3u64)\ninsert(m, 256u64, 2u64)\ninsert(m, 1u64, 1u64)\nvar s = byte(data, 0) as u64\nvar k = 1u64 * s * s\nreturn (has_key(m, k) as u64) as bytes";
    agree4(body, &[vec![1, 9, 9, 9], vec![2, 9, 9, 9]]);
    // dynamic BYTES keys with prefix relations from the input itself:
    // [7] is a prefix of [7 9]; lookups of both plus a third distinct key
    let body = "var m = new_map()\ninsert(m, buf_left(2, data), 1u64)\ninsert(m, buf_left(1, data), 2u64)\ninsert(m, buf_right(2, data), 3u64)\nvar r = (m[buf_left(1, data)] as u64) * 100u64 + (m[buf_left(2, data)] as u64)\nreturn r as bytes";
    agree4(body, &[vec![7, 9, 3, 4], vec![5, 6, 7, 8]]);
    // a bytes key that equals no entry (different middle byte) → 85
    let body = "var m = new_map()\ninsert(m, buf_left(2, data), 1u64)\nvar r = m[buf_right(2, data)]\nreturn (r as u64) as bytes";
    agree4(body, &[vec![7, 9, 3, 4]]);
}

#[test]
fn m31_dynamic_key_forms_single_ops() {
    // every uint width through the runtime normalization + the B4(c)
    // direct find: narrow widths zero-extend to the same canonical slot
    let hit = vec![3u8, 9, 9, 9];
    let m = "var m = new_map()\ninsert(m, 3u64, 30u64)\n";
    agree4(
        &format!("{m}var k = byte(data, 0) as u8\ninsert(m, k, 1u64)\nreturn ((m[3u64] as u64) * 10u64 + length(m)) as bytes"),
        &[hit.clone(), vec![7u8, 9, 9, 9]],
    );
    agree4(
        &format!("{m}var k = (byte(data, 0) as u16) + 3u16\nreturn (m[k] as u64) as bytes"),
        &[hit.clone()],
    );
    agree4(
        &format!("{m}var k = (byte(data, 0) as u32) + 3u32\nreturn (m[k] as u64) as bytes"),
        &[hit.clone()],
    );
    // a dynamic u128 key whose canonical span is 3 bytes (0x010301):
    agree4(
        &format!("{m}var k = (byte(data, 0) as u128) * 65536u128 + 257u128\nreturn ((m[k] as u64) * 10u64 + (has_key(m, 65537u64) as u64)) as bytes"),
        &[vec![1u8, 9, 9, 9], vec![2u8, 9, 9, 9]],
    );
    // cross-width lookup of a dynamic narrow key: u16(3) hits the u64(3)
    agree4(
        &format!("{m}var k = 3u16\nremove(m, k)\nreturn (length(m) as u64) as bytes"),
        &[hit.clone()],
    );
    // empty dynamic bytes key → 73 through remove (B4(c) direct face)
    agree4(
        "var m = new_map()\ninsert(m, buf_left(2, data), 5u64)\nremove(m, buf_left(0, data))\nreturn (length(m) as u64) as bytes",
        &[vec![7, 9, 3, 4]],
    );
}

#[test]
fn m32_merge_two_pointer_sweep_faces() {
    // duplicate as b's FIRST entry and as b's LAST entry — the sweep
    // detects the equal pair wherever it sits (the interpreter fires at
    // the first duplicate in b's sorted order; the fault code is the
    // same 84 either way)
    agree4(
        "var a = map{1u64 : 2u64, 3u64 : 4u64}\nvar b = map{1u64 : 9u64, 5u64 : 6u64}\nmerge(a, b)\nreturn 0u64 as bytes",
        &one(),
    );
    agree4(
        "var a = map{1u64 : 2u64, 3u64 : 4u64}\nvar b = map{5u64 : 9u64, 3u64 : 6u64}\nmerge(a, b)\nreturn 0u64 as bytes",
        &one(),
    );
    // a cross-width duplicate (u8(3) == u64(3)) through the sweep
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = map{3u8 : 9u64, 7u64 : 6u64}\nmerge(a, b)\nreturn (a[3u64] as u64) as bytes",
        &one(),
    );
    // empty either side sweeps to done with no fault
    agree4(
        "var a = new_map()\nvar b = map{1u64 : 2u64}\nmerge(a, b)\nreturn (length(a) as u64) as bytes",
        &one(),
    );
    // two dynamic 40-entry maps (disjoint ranges) merge to 80 sorted
    let mut body = fill_loop("a", 40, 0, "i");
    body.push_str(&fill_loop("b", 40, 100, "j"));
    body.push_str("merge(a, b)\nvar ks = keys(a)\nreturn ((length(a) as u64) * 1000u64 + (ks[39] as u64) * 10u64 + (ks[79] as u64)) as bytes");
    agree4(&body, &one());
    // multi-round: three merges into the same receiver, loop-driven
    let mut body = fill_loop("a", 10, 0, "i");
    body.push_str(&fill_loop("b", 10, 50, "j"));
    body.push_str(&fill_loop("c", 10, 100, "k"));
    body.push_str("merge(a, b)\nmerge(a, c)\nmerge(a, b)\nreturn (length(a) as u64) as bytes");
    // the third merge is a duplicate re-merge → 84
    agree4(&body, &one());
    // overlapping dynamic ranges → 84 mid-sweep
    let mut body = fill_loop("a", 20, 0, "i");
    body.push_str(&fill_loop("b", 20, 10, "j"));
    body.push_str("merge(a, b)\nreturn 0u64 as bytes");
    agree4(&body, &one());
}

#[test]
fn m33_merge_84_before_40_and_cap_face() {
    // over-cap WITHOUT duplicates → 40 (the sweep finds nothing, the cap
    // check fires): a = 100 entries, b = 40 → 140 > 128
    let mut body = fill_loop("a", 100, 0, "i");
    body.push_str(&fill_loop("b", 40, 1000, "j"));
    body.push_str("merge(a, b)\nreturn 0u64 as bytes");
    agree4(&body, &one());
    // over-cap WITH a duplicate → 84 must fire BEFORE the 40 (the
    // interpreter checks every source key before the cap; the sweep
    // completes before the cap check the same way): a = 128 entries, b
    // carries one duplicate
    let mut body = fill_loop("a", 128, 0, "i");
    body.push_str("var b = map{5u64 : 9u64}\nmerge(a, b)\nreturn 0u64 as bytes");
    agree4(&body, &one());
    // exactly at cap: 128 + 0 new keys (b all duplicates... no — b empty
    // is trivial); 127 + 1 new = 128, admitted
    let mut body = fill_loop("a", 127, 0, "i");
    body.push_str("var b = map{999u64 : 1u64}\nmerge(a, b)\nreturn (length(a) as u64) as bytes");
    agree4(&body, &one());
}

#[test]
fn m34_large_map_multi_round_ring() {
    // a large map driven through many insert/remove rounds in loops: the
    // find ring, the sorted insert shifting, and the remove shifting all
    // replay across the whole entry array in every mode
    let mut body = String::from("var m = new_map()\nvar r = 0u64\nvar acc = 0u64\nwhile r < 6u64 {\nvar i = 0u64\nwhile i < 30u64 { insert(m, i, r * 100u64 + i)\ni += 1u64 }\nvar j = 0u64\nwhile j < 15u64 { remove(m, j * 2u64)\nj += 1u64 }\nacc += (m[29u64] as u64) + length(m)\nr += 1u64 }\nreturn acc as bytes");
    agree4(&body, &one());
    // the merged 80-entry map of m32 read back through a ring loop
    let mut body = fill_loop("a", 40, 0, "i");
    body.push_str(&fill_loop("b", 40, 100, "j"));
    body.push_str("merge(a, b)\nvar s = 0u64\nvar ks = keys(a)\nvar w = 0u64\nwhile w < 80u64 { s = s + (ks[w] as u64)\nw += 1u64 }\nreturn ((s as u64) * 10u64 + length(a)) as bytes");
    agree4(&body, &one());
}

#[test]
fn m35_tag_free_descriptor_alias_merge() {
    // B6: bare-pointer descriptors — the alias copy passes the whole slot
    // word, the self-merge face compares two bare pointers, and every map
    // op dereferences without a mask; alias chains stay one object
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nvar c = b\ninsert(c, 3u64, 4u64)\nreturn ((a[3u64] as u64) * 10u64 + length(b)) as bytes",
        &one(),
    );
    // alias merge → 80 on the bare pointers
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nmerge(a, b)\nreturn 0u64 as bytes",
        &one(),
    );
    // length/itemget through a chained alias after a merge
    agree4(
        "var a = map{1u64 : 2u64}\nvar b = a\nvar c = map{2u64 : 3u64}\nmerge(b, c)\nreturn ((length(a) as u64) * 10u64 + (b[2u64] as u64)) as bytes",
        &one(),
    );
}
