//! T16 — the fitsh→rv64 MAP family differential tests.
//!
//! Every test drives the DEFAULT lowering channel (`c3lib::Rv64`, all three
//! modes {Im, Imc, ImcRemap} round-tripped through the consensus container
//! decoder) against the consensus interpreter: identical Ok bytes or the
//! identical fault code. The arena map model lives in `vm/src/ir/rv64.rs`
//! ("T16: arena map descriptor emission"): a map slot's cell holds a tagged
//! arena descriptor (`ptr | 1`), entries are kept sorted by the canonical
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
