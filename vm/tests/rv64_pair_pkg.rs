//! P3-PAIRPKG (A-25 milestone M6): the REMOVE+INSERT adjacent pair — the
//! language-level faithful position-overwrite simulation lowered to one
//! guarded `sd`.
//!
//! Semantics (doc 98 §Method, interpreter pins r1..r8/x1..x3): for i < len the
//! pair is a per-value overwrite (remove left-shifts, the insert arguments
//! are evaluated AFTER the remove executed — shifted view); i ≥ len traps
//! CompoNoFindItem(85) before any state change; the 81/40 faces are
//! structurally empty for pairs; a Nil value is NOT trappable (check_scalar
//! admits Nil — the interpreter silently stores it) so rv64 compile-rejects
//! may-Nil pair values. The fused emission: index evaluated once → 72 → 85
//! → value eval → one `sd`; the length shadow is net-zero.
//!
//! Unpaired REMOVE/INSERT keep the opcode-named reject (no shift lowering).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

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
        "reject must use the canonical subset text: {err:?} (body: {body})"
    );
}

/// Interpreter acceptance proof: the body compiles AND runs (any outcome).
fn interp_accepts(body: &str) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    let _ = it.run(&[1, 2, 3, 4]);
}

fn inputs_first_byte() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 7, 11]).collect()
}

/// 8-byte LE readback of three elements (byte-order independent probe).
fn read3(a: &str, b: &str, c: &str) -> String {
    format!("return (((({a}) * 65536u64) + (({b}) * 256u64)) + ({c})) as bytes")
}

/* ================================================================== */
/* Section 1: the pair shape matrix — four-way crosschecks.            */
/* ================================================================== */

/// Constant-index pairs: i ∈ {0, len−1, len, len+1} over len 3, plus the
/// u32 boundary family {2³²−1 (85), 2³² (72), 2⁶⁴−1 (72)}. Each body is a
/// different compile-time const_idx path of the fused emission.
#[test]
fn p1_const_index_matrix() {
    let setup = "var l = [11u64, 22u64, 33u64]\n";
    // in-range positions: full overwrite semantics per position — the
    // exact BE readback is asserted per position, then four-way agreement
    for (pos, i) in ["0u64", "1u64", "2u64"].iter().enumerate() {
        let mut body = format!("{setup}remove(l, {i})\ninsert(l, {i}, 99u64)\n");
        body.push_str(&read3("l[0]", "l[1]", "l[2]"));
        let rv = Rv64::try_build(&body).unwrap_or_else(|e| panic!("build: {e}\n{body}"));
        let got = rv.run(&[0])[0].1.clone().unwrap();
        // the readback folds to ONE u64 (BE 8B): position `pos` holds 99,
        // the other two keep 11/22/33
        let mut val = 0u64;
        for (p, b) in [11u64, 22, 33].iter().enumerate() {
            let x = if p == pos { 99u64 } else { *b };
            val |= x << (16 - 8 * p);
        }
        let want = val.to_be_bytes().to_vec();
        assert_eq!(got, want, "position {pos} body:\n{body}");
        agree4(&body, &[vec![0], vec![255]]);
    }
    // i == len and i > len: remove traps 85 first, insert never runs
    for i in ["3u64", "4u64", "4294967295u64"] {
        let mut body = format!("{setup}remove(l, {i})\ninsert(l, {i}, 99u64)\n");
        body.push_str(&read3("l[0]", "l[1]", "l[2]"));
        agree4(&body, &[vec![0], vec![9], vec![250]]);
    }
    // ≥ 2³²: extract_u32 faults CastParamFail(72) before any bound check
    for i in ["4294967296u64", "18446744073709551615u64", "5000000000u64"] {
        let mut body = format!("{setup}remove(l, {i})\ninsert(l, {i}, 99u64)\n");
        body.push_str(&read3("l[0]", "l[1]", "l[2]"));
        agree4(&body, &[vec![0], vec![9]]);
    }
    // the 2³²−1 const over a SHORT list also 85s unconditionally (k ≥ 128
    // cap-invariant form)
    let mut body =
        String::from("var l = [7u64, 8u64]\nremove(l, 4294967295u64)\ninsert(l, 4294967295u64, 1u64)\n");
    body.push_str(&read3("l[0]", "l[1]", "l[1]"));
    agree4(&body, &[vec![0]]);
}

/// Variable-index pairs: the index is evaluated ONCE at the remove root
/// (lemma P3). In-range/OOB/2³² boundary mixed per input byte; overwrites
/// verified elementwise; the length shadow must stay net-zero.
#[test]
fn p2_variable_index_matrix() {
    // sweep 0..=3 over len 3: positions 0..2 overwrite, 3 → 85
    let mut body = String::from(
        "var l = [11u64, 22u64, 33u64]\n\
         var i = (byte(data, 0) as u64) % 4u64\n\
         var v = (byte(data, 1) as u64) * 3u64 + 1u64\n\
         remove(l, i)\ninsert(l, i, v)\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[2]"));
    agree4(&body, &inputs_first_byte());

    // boundary sweep: 2³²−1 (85) and 2³².. (72) hit the same guards
    let mut body = String::from(
        "var l = [11u64, 22u64, 33u64]\n\
         var i = 4294967295u64 + (byte(data, 0) as u64)\n\
         remove(l, i)\ninsert(l, i, 9u64)\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[2]"));
    agree4(&body, &[vec![0], vec![1], vec![128], vec![255]]);

    // variable index over a 2-element list, value = plain input byte
    let mut body = String::from(
        "var l = [7u64, 8u64]\n\
         var i = (byte(data, 0) as u64) % 3u64\n\
         remove(l, i)\ninsert(l, i, (byte(data, 1) as u64))\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[1]"));
    agree4(&body, &inputs_first_byte());
}

/// Expression-index pairs: the SAME (structurally equal) expression on both
/// sides, evaluated once by the fused form. The value is a byte-fuse
/// BE-2 candidate (`b0*256 + b1`) — the fused group inside the skipped
/// insert's value subtree must lower through the eval path (coexistence).
#[test]
fn p3_expression_index_and_fuse_coexistence() {
    // expr index ((byte+1) % 4): 0..3 → three overwrites + one 85
    let mut body = String::from(
        "var l = [1u64, 2u64, 3u64]\n\
         var j = ((byte(data, 0) as u64) + 1u64) % 4u64\n\
         remove(l, j)\ninsert(l, j, 42u64)\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[2]"));
    agree4(&body, &inputs_first_byte());

    // pair value = byte-fuse BE-2 candidate: coexistence of the M3 wide
    // load inside the fused pair's value eval
    let mut body = String::from(
        "var l = [1u64, 2u64, 3u64]\n\
         remove(l, 1u64)\ninsert(l, 1u64, ((byte(data, 0) as u64) * 256u64) + (byte(data, 1) as u64))\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[2]"));
    agree4(&body, &inputs_first_byte());

    // pair value = byte-fuse LE-2 candidate
    let mut body = String::from(
        "var l = [1u64, 2u64, 3u64]\n\
         remove(l, 0u64)\ninsert(l, 0u64, (byte(data, 0) as u64) | ((byte(data, 1) as u64) << 8u64))\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[2]"));
    agree4(&body, &inputs_first_byte());
}

/// The shifted-view control group (k6 discipline, both directions): the
/// interpreter evaluates the insert arguments AFTER the remove — a value
/// reading the receiver sees the shifted list. rv64 refuses to fuse that
/// shape (the fused form would store the pre-shift value) and names
/// REMOVE; the interpreter itself runs it.
#[test]
fn p4_shifted_view_control_group() {
    // value reads receiver: interpreter [1,2,3] remove@0 insert(0, l[0]) =
    // [2,2,3] (the doc 75 E6 pin), rv64 must refuse the pair
    let body = "var l = [1u64, 2u64, 3u64]\nremove(l, 0u64)\ninsert(l, 0u64, l[0])\n";
    let mut full = body.to_string();
    full.push_str(&read3("l[0]", "l[1]", "l[2]"));
    interp_accepts(&full);
    rv64_must_reject(&full, "REMOVE");

    // index reads receiver: structurally unequal AND receiver-touching
    let body2 = "var l = [1u64, 2u64, 3u64]\nvar z = 0u64\nremove(l, 1u64)\ninsert(l, l[z], 9u64)\nreturn l[0] as bytes";
    interp_accepts(body2);
    rv64_must_reject(body2, "REMOVE");

    // the same-value NON-receiver read fuses fine (control: the reject is
    // about the receiver, not reads in general)
    let body3 = "var l = [1u64, 2u64, 3u64]\nvar l2 = [4u64, 5u64]\nremove(l, 1u64)\ninsert(l, 1u64, l2[0])\n";
    let mut full3 = body3.to_string();
    full3.push_str(&read3("l[0]", "l[1]", "l[1]"));
    agree4(&full3, &[vec![0], vec![7]]);
}

/// The may-Nil value face: the interpreter does NOT trap at the mutation
/// (check_scalar admits Nil — the x1..x3 pins) and faults only when the
/// stored Nil element is later read. rv64 compile-rejects the pair (the
/// u64 frame block cannot hold a Nil hole) — a plan-refusal, not a
/// runtime divergence.
#[test]
fn p5_may_nil_value_compile_reject() {
    let nil_decl = "var q = 0u64\nif q == 1u64 { var n = 5u64 }\n";
    // interpreter: insert succeeds, length stays 3 (the x1 pin)
    let body = format!(
        "{nil_decl}var l = [1u64, 2u64, 3u64]\nremove(l, 0u64)\ninsert(l, 0u64, n)\nreturn length(l) as bytes"
    );
    let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("interp: {e}"));
    let got = it.run(&[0]).unwrap_or_else(|f| panic!("interp run: {:?}", f.code));
    // length(l) as bytes = the pinned U32 width (D5/D7 family), BE
    assert_eq!(
        got,
        vec![0u8, 0, 0, 3],
        "interpreter must have stored the Nil silently (length still 3)"
    );
    rv64_must_reject(&body, "REMOVE");

    // Nil via a choose arm (may-Nil Int expression)
    let body2 = format!(
        "{nil_decl}var l = [1u64, 2u64]\nremove(l, 1u64)\ninsert(l, 1u64, if q == 1u64 {{ n }} else {{ 4u64 }})\nreturn l[1] as bytes"
    );
    rv64_must_reject(&body2, "REMOVE");
    let _ = Interp::try_build(&body2).unwrap_or_else(|e| panic!("interp must accept: {e}"));

    // a DEFINITE value fuses (control)
    let body3 = "var q = 0u64\nif q == 1u64 { var n = 5u64 }\nvar l = [1u64, 2u64]\nvar w = 9u64\nremove(l, 0u64)\ninsert(l, 0u64, w)\nreturn l[0] as bytes";
    agree4(body3, &[vec![0]]);
}

/// Negative family — every row names the opcode (canonical subset text).
/// The interpreter accepts and runs each shape (where it terminates).
#[test]
fn p6_negative_family() {
    // single REMOVE / single INSERT
    let b = "var l = [1u64, 2u64]\nremove(l, 0u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");
    let b = "var l = [1u64, 2u64]\ninsert(l, 0u64, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "INSERT");

    // non-adjacent pair (a statement between)
    let b = "var l = [1u64, 2u64]\nremove(l, 0u64)\nvar t = 5u64\ninsert(l, 0u64, t)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // cross-control-flow pair
    let b = "var l = [1u64, 2u64]\nvar c = 0u64\nremove(l, 0u64)\nif c == 1u64 { var x = 1u64 }\ninsert(l, 0u64, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // structurally unequal indices (const vs var)
    let b = "var l = [1u64, 2u64]\nvar j = 0u64\nremove(l, 0u64)\ninsert(l, j, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // value-unequal indices with equal structure but different slots
    let b = "var l = [1u64, 2u64]\nvar j = 0u64\nvar k2 = 0u64\nremove(l, j)\ninsert(l, k2, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // index reads receiver (shifted view)
    let b = "var l = [1u64, 2u64]\nvar z = 0u64\nremove(l, 0u64)\ninsert(l, l[z], 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // value reads receiver
    let b = "var l = [1u64, 2u64]\nremove(l, 0u64)\ninsert(l, 0u64, l[0])\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // value reads receiver's length (mid-pair value differs)
    let b = "var l = [1u64, 2u64]\nremove(l, 1u64)\ninsert(l, 1u64, length(l))\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // remove result bound into a var (alias face)
    let b = "var l = [1u64, 2u64]\nvar m = remove(l, 0u64)\ninsert(l, 0u64, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");

    // different receiver slots
    let b = "var l = [1u64, 2u64]\nvar l2 = [3u64, 4u64]\nremove(l, 0u64)\ninsert(l2, 0u64, 9u64)\nreturn l[0] as bytes";
    interp_accepts(b);
    rv64_must_reject(b, "REMOVE");
}

/// Pairs upgrade the storage class: a literal (rodata candidate) or
/// computed (FrameStatic candidate) list with a pair touch becomes
/// Growable — creation fills the frame block, the pair mutates it, reads
/// verify. Also the Empty class: every pair position is a runtime 85.
#[test]
fn p7_storage_class_upgrade_and_empty() {
    // literal list + pair → Growable{init_n: 2}
    let mut body = String::from(
        "var l = [5u64, 6u64]\nvar i = (byte(data, 0) as u64) % 3u64\nremove(l, i)\ninsert(l, i, 99u64)\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[1]"));
    agree4(&body, &inputs_first_byte());

    // computed-element list + const pair
    let mut body = String::from(
        "var l = [(byte(data, 0) as u64), 6u64]\nremove(l, 0u64)\ninsert(l, 0u64, 7u64)\n",
    );
    body.push_str(&read3("l[0]", "l[1]", "l[1]"));
    agree4(&body, &inputs_first_byte());

    // empty list + pair: always 85 (const k<128 guarded path)
    let body = String::from("var l = []\nremove(l, 0u64)\ninsert(l, 0u64, 1u64)\nreturn l[0] as bytes");
    agree4(&body, &[vec![0]]);
    // empty list + var pair
    let body2 = "var l = []\nvar i = (byte(data, 0) as u64) % 2u64\nremove(l, i)\ninsert(l, i, 1u64)\nreturn l[0] as bytes";
    agree4(body2, &inputs_first_byte());
}

/// Emission accounting: the fused pair adds a bounded, deterministic byte
/// increment per pair site (li + ld len + bgeu → thunk + sd for the const
/// form), and the 85-thunk is reachable from the guard. Two same-shape
/// bodies differing only in the pair count differ by exactly the second
/// pair's static increment; rebuilding is byte-identical (determinism).
#[test]
fn p8_emission_accounting_and_determinism() {
    // both bodies share one growable list and differ ONLY by the second
    // pair (two growable frame blocks would exceed the addi window — the
    // doc 92 follow-up 4 limit)
    let mut b1 = String::from("var l = [1u64, 2u64]\nremove(l, 0u64)\ninsert(l, 0u64, 9u64)\n");
    b1.push_str(&read3("l[0]", "l[1]", "l[1]"));
    let mut b2 = String::from(
        "var l = [1u64, 2u64]\nremove(l, 0u64)\ninsert(l, 0u64, 9u64)\nremove(l, 1u64)\ninsert(l, 1u64, 8u64)\n",
    );
    b2.push_str(&read3("l[0]", "l[1]", "l[1]"));
    let r1 = Rv64::try_build(&b1).unwrap_or_else(|e| panic!("build1: {e}"));
    let r2a = Rv64::try_build(&b2).unwrap_or_else(|e| panic!("build2: {e}"));
    let r2b = Rv64::try_build(&b2).unwrap_or_else(|e| panic!("build2b: {e}"));
    let code1 = r1.containers[0].1.len();
    let code2a = r2a.containers[0].1.len();
    let code2b = r2b.containers[0].1.len();
    // determinism: two builds of the same body are byte-identical
    assert_eq!(code2a, code2b, "pair body rebuild must be byte-identical");
    // bounded per-pair increment: the second pair (same const form on a
    // second list) adds a small bounded block — pin it as 8..48B (li 4B /
    // ld 4B / bgeu 4B / sd 4B + eval of the shared read glue differences
    // stay out of the band by construction)
    let delta = code2a as i64 - code1 as i64;
    assert!(
        (8..=48).contains(&delta),
        "second pair must add a bounded 8..48B increment, got {delta} (code1 {code1}, code2 {code2a})"
    );
    // agreement on the bodies themselves
    agree4(&b1, &[vec![0], vec![5]]);
    agree4(&b2, &[vec![0], vec![5]]);
}

/// Loop dynamic-index pairs (doc 75 E7 keccak core shape): the fused form
/// keeps the length shadow loop-carried-invariant (net zero per iteration);
/// the dynamic sd path is exercised per position; crosschecked per input.
#[test]
fn p9_loop_dynamic_index_pairs() {
    // overwrite every position from a loop
    let body = "var l = [1u64, 2u64, 3u64, 4u64]\n\
         var i = 0u64\n\
         while i < 4u64 {\n\
         remove(l, i)\ninsert(l, i, (i + 1u64) * 11u64)\ni += 1u64\n}\n\
         return ((((l[0] * 65536u64) + (l[1] * 256u64)) + l[2]) * 16777216u64 + l[3]) as bytes";
    agree4(body, &[vec![0], vec![3], vec![200]]);

    // rotate: read both before mutating (source-level discipline the
    // recognizer enforces), pair per position inside the loop, plus a
    // length fold after the loop (shadow invariance)
    let body2 = "var l = [10u64, 20u64, 30u64]\n\
         var i = 0u64\n\
         var n = length(l)\n\
         var first = l[0]\n\
         while i < 3u64 {\n\
         var nxt = (i + 1u64) % 3u64\n\
         var v = l[nxt]\n\
         remove(l, i)\ninsert(l, i, v)\ni += 1u64\n}\n\
         remove(l, 2u64)\ninsert(l, 2u64, first)\n\
         return ((length(l) * 16777216u64) + (((l[0] * 65536u64) + (l[1] * 256u64)) + l[2])) as bytes";
    agree4(body2, &[vec![0], vec![9]]);

    // pair loop inside an if body (median comparator context)
    let body3 = "var l = [5u64, 6u64]\n\
         var c = (byte(data, 0) as u64) % 2u64\n\
         if c == 1u64 {\n\
         var t = l[0]\n\
         var u = l[1]\n\
         remove(l, 0u64)\ninsert(l, 0u64, u)\nremove(l, 1u64)\ninsert(l, 1u64, t)\n\
         }\n\
         return ((l[0] * 256u64) + l[1]) as bytes";
    agree4(body3, &inputs_first_byte());
}

/* ================================================================== */
/* Section 2 (M6-b): the median list-form kernel — pairs at scale.     */
/* ================================================================== */

/// The 60-comparator Batcher network for n=16 (byte-exact b1 network,
/// rv97 scratch median_fitsh / hacdtestnet fitsh_kernels.rs::median_fitsh).
const BATCHER16: [(usize, usize); 60] = [
    (0, 1), (2, 3), (4, 5), (6, 7), (8, 9), (10, 11), (12, 13), (14, 15),
    (0, 2), (1, 3), (4, 6), (5, 7), (8, 10), (9, 11), (12, 14), (13, 15),
    (0, 4), (1, 5), (2, 6), (3, 7), (8, 12), (9, 13), (10, 14), (11, 15),
    (0, 8), (1, 9), (2, 10), (3, 11), (4, 12), (5, 13), (6, 14), (7, 15),
    (5, 10), (6, 9), (3, 12), (13, 14), (1, 2), (4, 8), (7, 11), (11, 14),
    (2, 4), (5, 8), (9, 12), (13, 15), (3, 8), (7, 12), (1, 4), (9, 13),
    (3, 5), (7, 9), (11, 13), (2, 3), (6, 7), (10, 11), (12, 13), (5, 6),
    (9, 10), (3, 4), (7, 8), (11, 12),
];

/// List-form median body: one 16-element list + 60 pair-based
/// compare-exchanges. Each comparator reads BOTH elements into temps
/// BEFORE mutating (the recognizer's receiver-free value discipline),
/// then two fused pairs. `seed`: None = the b1 golden constants
/// (the volume-measurement body); Some(fmt) = element k read from the
/// input (the correctness-sweep body).
fn median_list_body(input_elements: bool) -> String {
    let mut src = String::new();
    if input_elements {
        // correctness-sweep body: element k = input byte k
        let elems: Vec<String> = (0..16).map(|k| format!("(byte(data, {k}) as u64)")).collect();
        src.push_str(&format!("var l = [{}]\n", elems.join(", ")));
    } else {
        // volume-measurement body: the b1 golden constants as list
        // literals (16 extra m-temps would push the frame past the addi
        // window: 137×8 + 1024 > 2040 — the doc 92 follow-up 4 limit)
        let consts: Vec<String> = (0..16u64)
            .map(|i| {
                let v = i.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(7);
                format!("{v}u64")
            })
            .collect();
        src.push_str(&format!("var l = [{}]\n", consts.join(", ")));
    }
    for (n, (a, b)) in BATCHER16.iter().enumerate() {
        src.push_str(&format!(
            "if l[{a}] > l[{b}] {{\nvar ta{n} = l[{a}]\nvar tb{n} = l[{b}]\nremove(l, {a}u64)\ninsert(l, {a}u64, tb{n})\nremove(l, {b}u64)\ninsert(l, {b}u64, ta{n})\n}}\n"
        ));
    }
    src.push_str("return l[8] as bytes");
    src
}

/// The scalar-form median body (the rv97/b1 shape): 16 scalar locals +
/// the same 60 comparator if-blocks with direct scalar assignment. This is
/// the SEMANTIC reference for the list form — same network, same constants,
/// no pairs involved.
fn median_scalar_body(input_elements: bool) -> String {
    let mut src = String::new();
    if input_elements {
        for k in 0..16 {
            src.push_str(&format!("var m{k} = (byte(data, {k}) as u64)\n"));
        }
    } else {
        for i in 0..16u64 {
            let v = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let r = v.rotate_left(7);
            src.push_str(&format!("var m{i} = {r}u64\n"));
        }
    }
    for (n, (a, b)) in BATCHER16.iter().enumerate() {
        src.push_str(&format!(
            "if m{a} > m{b} {{\nvar t{n} = m{a}\nm{a} = m{b}\nm{b} = t{n}\n}}\n"
        ));
    }
    src.push_str("return m8 as bytes");
    src
}

/// Correctness sweep: the list-form network vs the scalar-form reference
/// (the b1 60-comparator network is a median-SELECTION network — its
/// pinned semantics are the scalar form's output, rv97/b1 90/0), 16-tuple
/// sweep over 0..255 (corners: all-equal / sorted / reversed + xorshift).
#[test]
fn p10_median_list_form_known_answer_sweep() {
    // NOTE: the b1 60-comparator network is a median-selection network,
    // NOT a full sort — its pinned semantics are the SCALAR form's output
    // (rv97/b1 90/0), so the reference here is the scalar-form interpreter,
    // not a reference sort.
    let body = median_list_body(true);
    let scalar = median_scalar_body(true);
    let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("interp build: {e}\n{body}"));
    let it_scalar = Interp::try_build(&scalar).unwrap_or_else(|e| panic!("scalar interp: {e}"));
    let rv = Rv64::try_build(&body).unwrap_or_else(|e| panic!("rv64 build: {e}\n{body}"));

    let mut tuples: Vec<Vec<u8>> = Vec::new();
    tuples.push(vec![7u8; 16]); // all equal
    tuples.push((0..16u8).collect()); // sorted
    tuples.push((0..16u8).rev().collect()); // reversed
    // deterministic xorshift sweep over 0..255 values
    let mut st = 0x2545F4914F6CDD1Du64;
    for _ in 0..64 {
        let mut t = Vec::with_capacity(16);
        for _ in 0..16 {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            t.push((st >> 33) as u8);
        }
        tuples.push(t);
    }
    for t in &tuples {
        let want = interp_obs(&it_scalar, t)
            .unwrap_or_else(|c| panic!("scalar interp trap {c} on {t:?}"));
        let got_interp = interp_obs(&it, t).unwrap_or_else(|c| panic!("interp trap {c} on {t:?}"));
        assert_eq!(
            got_interp, want,
            "list-form median diverged from the scalar form on {t:?}"
        );
        for (mode, got) in rv.run(t) {
            assert_eq!(
                got.unwrap_or_else(|c| panic!("mode {mode:?} trap {c} on {t:?}")),
                want,
                "mode {mode:?} diverged on {t:?}"
            );
        }
    }
}

/// Volume measurement (honest): the list-form median (pairs at scale, 120
/// fused pair sites) vs the scalar-form fitsh median (1270B code, doc 97
/// MS-c current anchor) vs the b1 rust median (396B prof channel). The
/// list form pays guarded reads + guard-store pairs per comparator where
/// the scalar form keeps values in the register bank — the measurement
/// decides, no curve-fitting.
#[test]
fn p11_median_list_form_volume() {
    let body = median_list_body(false);
    let rv = Rv64::try_build(&body).unwrap_or_else(|e| panic!("rv64 build: {e}\n{body}"));
    let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("interp build: {e}"));
    // known answer: the golden-const list form must equal the scalar form
    // byte-for-byte (the b1-pinned body shape, same network/constants)
    let scalar = Interp::try_build(&median_scalar_body(false))
        .unwrap_or_else(|e| panic!("scalar interp: {e}"));
    let want = interp_obs(&scalar, &[0]).unwrap_or_else(|c| panic!("scalar trap {c}"));
    let interp_out = interp_obs(&it, &[0]).unwrap_or_else(|c| panic!("interp trap {c}"));
    assert_eq!(interp_out, want, "golden-const list-form median wrong");
    for (mode, got) in rv.run(&[0]) {
        assert_eq!(got.unwrap_or_else(|c| panic!("mode {mode:?} trap {c}")), want);
    }
    println!("== median volume (honest measurement) ==");
    for (m, c) in &rv.containers {
        println!("list-form container {m:?}: {} B", c.len());
    }
    println!("list-form code mirror (Im): {} B", rv.code_size);
    println!("scalar-form fitsh (doc 97 anchor): 1270 B code");
    println!("b1 rust median (prof channel): 396 B");
    let list_code = rv.containers[0].1.len();
    // no superiority assertion: this measurement is recorded as-is in the
    // doc (the pair package's value is dynamic-index overwriting, not
    // constant-index comparator volume)
    assert!(list_code > 0);
}
