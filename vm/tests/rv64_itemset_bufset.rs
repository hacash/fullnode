//! B2 task 5: `ITEMSET` (0x6C) / `BUFSET` (0x52) / `ITEMCAT` (0x6D) — the
//! rv64 lowerings of the in-place element store, the bytes patch and the
//! list→bytes splice, four-way agreed against the interpreter.
//!
//! Faces and their ORDER mirror the interpreter exactly:
//! * ITEMSET (execute.rs → compo.rs item_set): receiver 83 (structurally
//!   absent on a local list slot; a non-slot receiver compile-rejects),
//!   value check_scalar 76 (a definite uint cannot fail; a may-Nil value
//!   compile-rejects — the BUG-029 family), extract_u32 72 (Nil / ≥ 2³²),
//!   idx ≥ len (incl. ==) → 85; no shift, no length change. The compound
//!   `l[i] op= v` lowers natively (NOT as a raw DUPN/ROLL splice) with the
//!   fragment's RHS-first evaluation order and the arithmetic's own faces
//!   (71 Nil / 90 overflow) BETWEEN the itemget guards and the store.
//! * BUFSET (operand.rs bufset): src extract 91 → ost extract_u16 72 →
//!   receiver 91 → boundary 42 (`ost + src.len` wraps or exceeds the
//!   length; == length is legal); the length never changes. rv64 bytes
//!   slots hold (ptr,len) descriptors that may alias anything (a cut of
//!   the input, another slot, the literal pool), so the store FIRST copies
//!   the slot's own bytes into a fresh arena region (compiler.md premise
//!   24 — iron-law level), writes inside the copy and rebinds the slot;
//!   every other holder keeps the old bytes.
//! * ITEMCAT (canbe.rs concat_call_chunks): non-list/map receivers are
//!   structurally absent on a local list slot; every element contributes
//!   its `extract_bytes` form (the width's big-endian bytes — NOT a block
//!   copy: the element block is little-endian); OutOfValueSize 39 when the
//!   total exceeds the kernel's output budget; the element 91 faces are
//!   structurally absent (definite uint elements only).
//!
//! Reject rows pin the kernel-subset boundaries: `l[i][j] = v` (the inner
//! receiver is not a local list slot), `b[i] = v` on bytes (interpreter 83
//! at runtime), map receivers, Input receivers for BUFSET.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

/// The default harness budget is output_max 16; the interpreter channel's
/// value budget is the SpaceCap default (1280). Bodies whose ITEMCAT total
/// (len × 8) exceeds 16 build rv64 with the raised budget so the four-way
/// agreement compares SEMANTICS, not budget sizes.
fn agree4_big(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64::try_build_out(body, 1280, 0x7000)
        .unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
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

/// Interpreter result mapped to the rv64 observable shape.
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
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

/// rv64 must reject, naming `frag` and the canonical subset text.
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

/// Interpreter acceptance proof: compiles AND runs (any outcome).
fn interp_accepts(body: &str, run: bool) {
    let it = Interp::try_build(body)
        .unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    if run {
        let _ = it.run(&[1, 2, 3, 4]);
    }
}

fn inputs_first_byte() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 7, 11]).collect()
}

/* ================================================================== */
/* Section 1: ITEMSET — plain assignment `l[i] = v` / item_set().      */
/* ================================================================== */

#[test]
fn s1_itemset_var_index_inplace() {
    // in-place store: neighbours never move, length unchanged; the store
    // lands in a GROWABLE block (rodata upgraded by the itemset mutation)
    let body = "var l = [10u64, 20u64, 30u64]\n\
                var i = (byte(data, 0) as u64) % 3u64\n\
                l[i] = (byte(data, 1) as u64) * 7u64\n\
                var r = l[0] + l[1] + l[2] + length(l)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn s1b_itemset_call_form_and_literal_index() {
    // `item_set(l, i, v)` is the same ITEMSET; a constant index takes the
    // same live-length guard (the Growable class proves nothing statically)
    let body = "var l = [1u64, 2u64, 3u64]\n\
                item_set(l, 1u64, (byte(data, 0) as u64))\n\
                var r = l[0] + l[1] + l[2]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn s2_itemset_oob_85_incl_equal() {
    // idx ≥ len (INCLUDING == len) → CompoNoFindItem(85); itemset never
    // grows the list
    let body = "var l = [7u64, 8u64]\n\
                var i = (byte(data, 0) as u64) % 4u64\n\
                l[i] = 5u64\n\
                var r = l[0] + l[1]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
    // constant idx == len on the same shape
    let body2 = "var l = [7u64, 8u64]\n\
                 l[2] = 5u64\n\
                 return 0u64 as bytes";
    agree4(body2, &[vec![0u8; 4]]);
    // append-then-set: the store into a freshly grown slot
    let body3 = "var l = []\n\
                 append(l, 1u64)\n\
                 append(l, 2u64)\n\
                 var i = (byte(data, 0) as u64) % 3u64\n\
                 l[i] = 9u64\n\
                 var r = l[0] + l[1]\n\
                 return (r as u64) as bytes";
    agree4(body3, &inputs_first_byte());
}

#[test]
fn s3_itemset_u32_boundary_72_before_85() {
    // extract_u32 runs FIRST: idx ≥ 2³² faults 72 even though it is also
    // out of bounds; 2³²−1 over a 2-element list is 85
    let body = "var l = [7u64, 8u64]\n\
                var i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\n\
                l[i] = 5u64\n\
                return 0u64 as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn s4_itemset_after_clear_and_reuse() {
    // the write lands through the same shadow the loop re-uses
    let body = "var l = [1u64, 2u64]\n\
                clear(l)\n\
                append(l, 5u64)\n\
                append(l, 6u64)\n\
                l[0] = (byte(data, 0) as u64)\n\
                var r = l[0] + l[1]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn s5_itemset_nil_value_compile_reject() {
    // BUG-029 family: the interpreter stores the Nil element silently and
    // only a later read faults; the u64 element block cannot represent it
    let nil_decl = "var q = 0u64\nif q == 1u64 { var n = 5u64 }\n";
    let body = format!(
        "{nil_decl}var l = [1u64, 2u64]\nl[0] = n\nreturn 0u64 as bytes"
    );
    interp_accepts(&body, true);
    rv64_must_reject(&body, "ITEMSET");

    // the compound form `l[0] += n` KEEPS the may-Nil rhs: the Nil is the
    // ARITHMETIC operand there (CastFail 71 at the op, AFTER the itemget
    // guards) — a runtime 71 guard, four-way agreed
    let body2 = format!(
        "{nil_decl}var l = [1u64, 2u64]\nl[0] += n\nreturn 0u64 as bytes"
    );
    let it2 = Interp::try_build(&body2).unwrap();
    let f = it2.run(&[0u8; 4]).unwrap_err();
    assert_eq!(f.code, 71, "the compound faults CastFail(71) on the Nil rhs");
    let rv2 = Rv64::try_build(&body2).unwrap();
    for (mode, got) in rv2.run(&[0u8; 4]) {
        assert_eq!(got, Err(71), "mode {mode:?}");
    }
}

#[test]
fn s6_itemset_bytes_value_reject() {
    // a bytes-valued store would make the element heterogeneous — the
    // interpreter admits it (elements keep their own width), the kernel
    // subset refuses
    let body = "var l = [1u64, 2u64]\nl[0] = \"ab\" as bytes\nreturn 0u64 as bytes";
    interp_accepts(&body, false);
    rv64_must_reject(&body, "ITEMSET");
}

#[test]
fn s7_itemset_nested_and_bytes_receiver_reject() {
    // `l[i][j] = v`: the inner receiver is an ITEMGET (a scalar element),
    // not a local list slot — the interpreter faults 83 at runtime
    let body = "var l = [1u64, 2u64]\nl[0][1] = 5u64\nreturn 0u64 as bytes";
    interp_accepts(&body, true);
    rv64_must_reject(&body, "ITEMSET");

    // `b[i] = v` on a bytes receiver is ALSO an ITEMSET — interpreter 83
    let body2 = "var b = \"abcdefgh\"\nb[0] = 5u64\nreturn 0u64 as bytes";
    let it = Interp::try_build(body2)
        .unwrap_or_else(|e| panic!("interp must accept: {e}"));
    let f = it.run(&[0u8; 4]).unwrap_err();
    assert_eq!(f.code, 83, "bytes receiver is CompoOpNotMatch(83)");
    rv64_must_reject(body2, "ITEMSET");

    // `data[i] = v` on the input — same 83 face
    let body3 = "data[0] = 5u64\nreturn 0u64 as bytes";
    rv64_must_reject(body3, "ITEMSET");
}

#[test]
fn s8_itemset_in_conditional_and_loop() {
    // the mutation reachability upgrades the storage class even when the
    // store sits under control flow; loop stores hit the same shadow
    let body = "var l = [1u64, 2u64, 3u64]\n\
                var i = 0u64\n\
                while i < 3u64 {\n\
                l[i] = l[i] + (byte(data, 0) as u64)\n\
                i += 1\n\
                }\n\
                var r = l[0] + l[1] + l[2]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());

    let body2 = "var l = [1u64, 2u64]\n\
                 if (byte(data, 0) as u64) % 2u64 == 0u64 {\n\
                 l[0] = 9u64\n\
                 }\n\
                 var r = l[0] + l[1]\n\
                 return (r as u64) as bytes";
    agree4(body2, &inputs_first_byte());
}

/* ================================================================== */
/* Section 2: the compound `l[i] op= v` — native lowering.             */
/* ================================================================== */

#[test]
fn s10_compound_assign_agree4() {
    // RHS-first, element read + op + write back; neighbours untouched
    for op in ["+=", "-=", "*="] {
        let body = format!(
            "var l = [10u64, 20u64, 30u64]\n\
             var i = (byte(data, 0) as u64) % 3u64\n\
             l[i] {op} (byte(data, 1) as u64) + 1u64\n\
             var r = l[0] + l[1] + l[2]\n\
             return (r as u64) as bytes"
        );
        agree4(&body, &inputs_first_byte());
    }
}

#[test]
fn s11_compound_const_index_and_overflow_90() {
    // constant index; and the arithmetic's own face: a u64 overflow traps
    // 90 AFTER the itemget guards, on both engines
    let body = "var l = [18446744073709551615u64, 2u64]\n\
                l[0] += (byte(data, 0) as u64) + 1u64\n\
                var r = l[0] + l[1]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());
}

#[test]
fn s12_compound_fault_order_itemget_before_arith() {
    // the fragment's ITEMGET faults (72/85) fire BEFORE the arithmetic —
    // an out-of-bounds index with an overflowing rhs is 85, not 90
    let body = "var l = [7u64, 8u64]\n\
                var i = (byte(data, 0) as u64) % 4u64\n\
                l[i] += 18446744073709551615u64\n\
                return 0u64 as bytes";
    agree4(body, &inputs_first_byte());
    // and the u32 boundary face (72) likewise precedes the op
    let body2 = "var l = [7u64, 8u64]\n\
                 var i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\n\
                 l[i] += 1u64\n\
                 return 0u64 as bytes";
    agree4(body2, &inputs_first_byte());
}

#[test]
fn s13_compound_division_and_conditional() {
    let body = "var l = [100u64, 21u64]\n\
                var i = (byte(data, 0) as u64) % 2u64\n\
                l[i] /= 4u64\n\
                var r = l[0] * 10u64 + l[1]\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_first_byte());

    // under an if: the store's reachability still upgrades the class
    let body2 = "var l = [5u64]\n\
                 if (byte(data, 0) as u64) % 2u64 == 0u64 {\n\
                 l[0] *= 3u64\n\
                 }\n\
                 var r = l[0]\n\
                 return (r as u64) as bytes";
    agree4(body2, &inputs_first_byte());
}

/* ================================================================== */
/* Section 3: BUFSET — the bytes patch.                                */
/* ================================================================== */

/// 8-byte inputs for the buf/patch boundary sweeps.
fn inputs_b0_variants() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 1, 2, 3, 4, 5, 6, 7]).collect()
}

#[test]
fn s20_bufset_scalar_widths_and_slot_untouched() {
    // the PATCHED bytes are the RESULT value; the local slot itself reads
    // back UNPATCHED (the interpreter mutates the deep-copied stack value
    // only — probed fact, pinned four-way here)
    let body = "var b = \"aaaaaaaa\"\n\
                var o = (byte(data, 0) as u64) % 8u64\n\
                var x = buf_set(b, o, 17u8)\n\
                var r = (byte(x, o) as u64) * 1000000u64 + (byte(b, o) as u64)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_b0_variants());

    // u64 source writes its 8 big-endian bytes; length unchanged
    let body2 = "var b = \"aaaaaaaa\"\n\
                 var x = buf_set(b, 0u64, 2464388554683811993u64)\n\
                 var r = size(x) * 1000000u64 + (byte(x, 7) as u64)\n\
                 return (r as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);

    // bool source: 1 canonical byte (0/1)
    let body3 = "var b = \"aaaaaaaa\"\n\
                 var x = buf_set(b, 3u64, (byte(data, 0) as u64) % 2u64 == 0u64)\n\
                 var r = (byte(x, 3) as u64) * 10u64 + size(x)\n\
                 return (r as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn s21_bufset_bytes_source_and_boundary_42() {
    // patch with a bytes source; == length is legal, one past is 42
    let body = "var b = \"abcdefgh\"\n\
                var o = (byte(data, 0) as u64) % 7u64\n\
                var x = buf_set(b, o, \"XY\")\n\
                var r = size(x) * 1000000u64 + (byte(x, 0) as u64) * 256u64 + (byte(x, 6) as u64)\n\
                return (r as u64) as bytes";
    agree4(body, &inputs_b0_variants());

    // ost + 2 == 8 fills exactly to the end — legal
    let body2 = "var b = \"abcdefgh\"\n\
                 var x = buf_set(b, 6u64, \"XY\")\n\
                 return x";
    agree4(body2, &[vec![0u8; 4]]);

    // ost + 3 > 8 → StackError(42)
    let body3 = "var b = \"abcdefgh\"\n\
                 buf_set(b, 7u64, \"XYZ\")\n\
                 return 0u64 as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn s22_bufset_fault_order_91_72_91_42() {
    // src extract 91 fires BEFORE a bad start (72), the receiver face is
    // structurally absent on an assigned local slot, then the boundary 42
    let body = "var b = \"abcdefgh\"\n\
                var o = 9u64\n\
                buf_set(b, o, 5u8)\n\
                return 0u64 as bytes";
    agree4(body, &[vec![0u8; 4]]); // 42 (1-byte write at ost 9 > 8)

    // start ≥ 2¹⁶ → extract_u16 72
    let body2 = "var b = \"abcdefgh\"\n\
                 buf_set(b, 65536u64, \"X\")\n\
                 return 0u64 as bytes";
    agree4(body2, &[vec![0u8; 4]]);

    // Nil SOURCE → 91 first (before the bad start would matter)
    let nil_decl = "var q = 0u64\nif q == 1u64 { var n = 5u64 }\n";
    let body3 = format!("{nil_decl}var b = \"abcdefgh\"\nbuf_set(b, 9u64, n)\nreturn 0u64 as bytes");
    let it = Interp::try_build(&body3).unwrap();
    let f = it.run(&[0u8; 4]).unwrap_err();
    assert_eq!(f.code, 91, "Nil source is BytesHandle(91) even with a bad start");
    let rv = Rv64::try_build(&body3).unwrap();
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(got, Err(91), "mode {mode:?}");
    }
}

#[test]
fn s23_bufset_alias_isolation_and_cut_receiver() {
    // PREMISE 24 (iron law): rv64's cut is a SHARED descriptor — an
    // in-place patch would corrupt every holder. The patch lands in a
    // fresh copy (the result); b AND c read back unpatched.
    // c = cut("abcdefgh",1,4) = "bcde"; x = "ZZde"; b/c untouched.
    let body = "var b = \"abcdefgh\"\n\
                var c = buf_cut(b, 1u64, 4u64)\n\
                var x = buf_set(c, 0u64, \"ZZ\")\n\
                var r = (byte(b, 1) as u64) * 1000000u64 + (byte(c, 0) as u64) * 10000u64 \
                + size(x) * 100u64 + (byte(x, 0) as u64)\n\
                return (r as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);

    // a plain alias (`var b2 = b`) is equally isolated
    let body2 = "var b = \"abcdefgh\"\n\
                 var b2 = b\n\
                 var x = buf_set(b, 0u64, \"QQ\")\n\
                 var r = (byte(b, 0) as u64) * 10000u64 + (byte(b2, 0) as u64) * 100u64 \
                 + (byte(x, 0) as u64)\n\
                 return (r as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);

    // and the literal-pool hazard: two slots from IDENTICAL literals share
    // one pool entry — the store into one must not corrupt the other
    let body3 = "var b = \"abcdefgh\"\n\
                 var s2 = \"abcdefgh\"\n\
                 var x = buf_set(b, 0u64, \"WW\")\n\
                 var r = (byte(b, 0) as u64) * 10000u64 + (byte(s2, 0) as u64) * 100u64 \
                 + (byte(x, 0) as u64)\n\
                 return (r as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn s24_bufset_result_value_and_input_receiver_reject() {
    // buf_set yields the receiver itself: `var x = buf_set(...)` reads back
    // the patched buffer
    let body = "var b = \"abcdefgh\"\n\
                var x = buf_set(b, 2u64, 85u8)\n\
                var r = (byte(x, 2) as u64) * 10u64 + (byte(b, 2) as u64)\n\
                return (r as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);
    // the result IS the patched buffer: x[2] = 85 while b[2] = 'c' = 99

    // T15: an Input receiver mutates a fresh extract copy on the
    // interpreter (slot 0 reads back UNPATCHED) — the kernel now models
    // slot 0 as an entry-copied bytes local, so the shape LOWERS and all
    // four engines agree (was: compile-reject naming BUFSET)
    let body2 = "buf_set(data, 0u64, 85u8)\nreturn (byte(data, 0) as u64) as bytes";
    agree4(body2, &[vec![1, 2, 3, 4], vec![200, 0]]);

    // a cut EXPRESSION receiver (not a slot) — the interpreter patches a
    // temporary; rv64 refuses (only local bytes slots are stores)
    let body3 = "var b = \"abcdefgh\"\n\
                 buf_set(buf_cut(b, 1u64, 4u64), 0u64, \"Z\")\n\
                 var r = b\n\
                 return r";
    let it = Interp::try_build(body3).unwrap();
    let got = it.run(&[0u8; 4]).unwrap_or_else(|f| panic!("interp: {:?}", f.code));
    assert_eq!(got, b"abcdefgh".to_vec(), "the interpreter patched a temporary");
    rv64_must_reject(body3, "BUFSET");
}

#[test]
fn s25_bufset_nil_receiver_tracked_slot() {
    // an unassigned bytes slot reads back Nil → receiver extract 91
    let nil_decl = "var q = 0u64\nif q == 1u64 { var t = \"ab\" }\n";
    let body = format!("{nil_decl}buf_set(t, 0u64, 5u8)\nreturn 0u64 as bytes");
    let it = Interp::try_build(&body).unwrap();
    let f = it.run(&[0u8; 4]).unwrap_err();
    assert_eq!(f.code, 91, "Nil receiver is BytesHandle(91)");
    let rv = Rv64::try_build(&body).unwrap();
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(got, Err(91), "mode {mode:?}");
    }
}

/* ================================================================== */
/* Section 4: ITEMCAT — the list → bytes splice.                       */
/* ================================================================== */

#[test]
fn s30_itemcat_u64_elements_big_endian() {
    // every element contributes its extract_bytes form: 8 big-endian bytes
    // — NOT the little-endian element block copy (24B > the default 16B
    // harness budget → the raised-budget builder)
    let body = "var l = [1u64, 2u64, 3u64]\n\
                var r = item_concat(l)\n\
                return r";
    let it = Interp::try_build(body).unwrap();
    let want = it.run(&[0u8; 4]).unwrap();
    let mut expect = Vec::new();
    for v in [1u64, 2, 3] {
        expect.extend_from_slice(&v.to_be_bytes());
    }
    assert_eq!(want, expect, "interpreter splices BE per element");
    agree4_big(body, &[vec![0u8; 4]]);
}

#[test]
fn s31_itemcat_empty_and_growable() {
    // empty list → empty bytes; appended elements splice in order
    let body = "var e = []\n\
                var r = size(item_concat(e))\n\
                return (r as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);

    let body2 = "var w = []\n\
                 var i = 0u64\n\
                 while i < 4u64 {\n\
                 append(w, (i + 1u64) * (byte(data, 0) as u64 + 1u64))\n\
                 i += 1\n\
                 }\n\
                 var r = item_concat(w)\n\
                 return r ++ \"\"";
    agree4_big(body2, &inputs_first_byte());
}

#[test]
fn s32_itemcat_39_bound() {
    // OutOfValueSize 39: the total (len × 8) against the kernel's OUTPUT
    // budget — on rv64 that is `a3` (the harness default 16), on the
    // interpreter `cap.value_size` (1280). The budgets differ (iron law:
    // resource limits are not opcode semantics) — both engines enforce the
    // SAME rule total > budget → 39, == budget legal.
    //
    // Under the default 16B harness budget: 2 elements (16B) == budget →
    // legal on both engines; 3 elements (24B) → 39 on rv64 while the
    // interpreter (1280) succeeds — the pinned cap difference.
    let body2 = "var l = [1u64, 2u64]\nvar r = item_concat(l)\nreturn r";
    let it2 = Interp::try_build(body2).unwrap();
    let want = it2.run(&[0u8; 4]).unwrap();
    assert_eq!(want.len(), 16, "== the budget is legal");
    let rv2 = Rv64::try_build(body2).unwrap();
    for (mode, got) in rv2.run(&[0u8; 4]) {
        assert_eq!(got.unwrap(), want, "mode {mode:?}");
    }

    let body = "var l = [1u64, 2u64, 3u64]\nvar r = item_concat(l)\nreturn r";
    let it = Interp::try_build(body).unwrap();
    let got_it = it.run(&[0u8; 4]).unwrap();
    assert_eq!(got_it.len(), 24, "the interpreter's 1280 budget admits 24B");
    let rv = Rv64::try_build(body).unwrap();
    for (mode, got) in rv.run(&[0u8; 4]) {
        assert_eq!(got.unwrap_err(), 39, "mode {mode:?}: 24B > 16 → 39");
    }

    // under the raised 1280 budget the same body is four-way green
    agree4_big(body, &[vec![0u8; 4]]);
}

#[test]
fn s33_itemcat_in_expression() {
    // bytes-valued result participates in the usual byte forms
    let body = "var l = [72623859790382856u64, 1234605616436508552u64]\n\
                var r = item_concat(l)\n\
                var a = byte(r, 7)\n\
                var b = size(r)\n\
                var c = item_concat(l)\n\
                return ((a as u64) * 1000000u64 + (b as u64) * 1000u64 + (size(c) as u64)) as bytes";
    agree4(body, &[vec![0u8; 4]]);
}

#[test]
fn s34_itemcat_map_rejects_at_creation() {
    // T16 lowered map creation; item_concat still admits LIST receivers
    // only, so the reject now names ITEMCAT (the interpreter's runtime
    // CompoOpInvalid 80 face stays a compile reject).
    let body = "var m = map{1u64 : 2u64}\nvar r = item_concat(m)\nreturn r";
    interp_accepts(body, true);
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("map must reject"))
        .to_string();
    assert!(err.contains("ITEMCAT"), "{err}");
}

/* ================================================================== */
/* Section 5: determinism + cross-mode.                                */
/* ================================================================== */

#[test]
fn s40_determinism_and_mixed_body() {
    // ITEMSET + compound + BUFSET + ITEMCAT in one body (the ITEMCAT total
    // stays at the 16B default budget; the final size read keeps the
    // return inside the channel)
    let body = "var l = [1u64, 2u64]\n\
                var b = \"abcdefgh\"\n\
                var i = (byte(data, 0) as u64) % 2u64\n\
                l[i] = l[i] * (byte(data, 1) as u64 + 1u64)\n\
                var x = buf_set(b, 1u64, 17u8)\n\
                var r = item_concat(l)\n\
                return (size(r) * 1000000u64 + (byte(x, 1) as u64) + l[0] + l[1]) as bytes";
    let a = Rv64::try_build(body).unwrap();
    let b = Rv64::try_build(body).unwrap();
    assert_eq!(a.containers, b.containers, "rebuild must be byte-identical");
    agree4(body, &inputs_first_byte());
}
