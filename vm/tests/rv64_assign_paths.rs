//! T8 + T15 of the pure-compute extension list: the two assignment paths.
//!
//! * T8 — the high-slot local compound assignment `x op= v` (slot ≥ 64, no
//!   `XOP` encoding): the parser emits the bare fragment
//!   `IRLIST[value, GET idx; SWAP; op; PUT idx]` and the rv64 backend now
//!   consumes it natively with the exact `XOP` semantics (RHS-first, then
//!   read/modify/write of the old slot value at the slot/rhs common width,
//!   CastFail 71 on a non-uint old value, Arithmetic 90 on overflow and on
//!   division by zero). High slots come from the `var name $idx` alias
//!   declaration; the alias token only binds the slot index — expressions
//!   spell the plain name.
//! * T15 — input-parameter reassignment `data = <bytes>` and the
//!   `buf_set(data, ..)` receiver: plan() transitions slot 0 to a bytes
//!   local whose entry value is a COPY of the ABI input (the interpreter
//!   only ever mutates freshly extracted copies — reads before the first
//!   reassignment still see the input, `buf_set` never changes the slot,
//!   and the caller's buffer is untouched). XOP0 (`data += v`) and an
//!   int-typed `data = v` stay compile-rejects: the interpreter faults
//!   CastFail 71 arithmetically adding the bytes old value (XOP0) or would
//!   need dynamically-typed later reads (int rebind), both outside the
//!   one-type-per-slot model.
//!
//! Every positive case is a four-way differential: interpreter result and
//! failure code == all three rv64 modes (Im / Imc / ImcRemap).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

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

/// rv64 must reject with `frag` in the message.
fn rv64_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must contain {frag:?}: {err:?} (body: {body})"
    );
}

fn inputs_first_bytes() -> Vec<Vec<u8>> {
    (0u16..=255)
        .step_by(3)
        .map(|b| vec![b as u8, 7, 11, 13])
        .collect()
}

/* ================================================================== */
/* Section 1: T8 — high-slot local compound assignment (slot ≥ 64).   */
/* ================================================================== */

#[test]
fn s8_all_four_ops_u64_slot() {
    for (op, init) in [("+=", 10u64), ("-=", 200u64), ("*=", 7000u64), ("/=", 4_000_000_000u64)] {
        let body = format!(
            "var x $64 = {init}u64\nx {op} ((byte(data, 0) as u64) + 1u64)\nreturn x as bytes"
        );
        agree4(&body, &inputs_first_bytes());
    }
}

#[test]
fn s8_rhs_first_and_self_reference() {
    // the rhs evaluates BEFORE the old value is read (XOP order): a rhs
    // reading the same slot must observe the OLD value in the op
    let body = "var x $70 = (byte(data, 0) as u64)\n\
                x += (x * 10u64)\n\
                return x as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_constant_rhs_and_chained() {
    let body = "var x $65 = (byte(data, 0) as u64)\n\
                x += 1u64\n\
                x *= 3u64\n\
                x -= 2u64\n\
                x /= 2u64\n\
                return x as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_mixed_with_low_slot_xop() {
    // the high-slot fragment and the low-slot XOP encoding side by side
    let body = "var i = (byte(data, 0) as u64)\n\
                var j $64 = (byte(data, 1) as u64)\n\
                i += 1u64\n\
                j += 1u64\n\
                i *= j\n\
                j *= 2u64\n\
                return ((i + (j * 100u64)) as u64) as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_loop_carried_high_slot() {
    // the fragment inside a while loop: the backedge re-reads the write
    let body = "var i $64 = ((byte(data, 0) as u64) % 7u64)\n\
                var n = 0u64\n\
                while (n < 5u64) {\n\
                i += n\n\
                n += 1u64\n\
                }\n\
                return ((i * 100u64 + n) as u64) as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_u128_slot_compound() {
    // a 16-byte high slot: the (lo, hi) staged pair path
    let body = "var x $64 = ((byte(data, 0) as u128) * 18446744073709551616u128)\n\
                x /= 3u128\n\
                x -= 1u128\n\
                return x as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_fault_faces_agree() {
    // division by zero → Arithmetic 90 on both engines
    let body = "var x $64 = (byte(data, 0) as u64)\nx /= 0u64\nreturn x as bytes";
    agree4(body, &inputs_first_bytes());
    // u64 overflow via += → Arithmetic 90 on both engines
    let body = "var x $64 = 18446744073709551615u64\nx += 1u64\nreturn x as bytes";
    agree4(body, &inputs_first_bytes());
    // sub underflow → 90
    let body = "var x $64 = ((byte(data, 0) as u64) % 3u64)\nx -= 10u64\nreturn x as bytes";
    agree4(body, &inputs_first_bytes());
}

#[test]
fn s8_bytes_high_slot_still_rejects() {
    // a bytes old value: the interpreter faults CastFail(71) at runtime;
    // the kernel subset refuses up front with the slot-conflict text (the
    // XOP-on-list / XOP-on-bytes precedent)
    let body = "var b $64 = \"ab\"\nb += 1u64\nreturn 0u64 as bytes";
    let it = Interp::try_build(body).unwrap();
    let f = it.run(&[1, 2, 3, 4]).unwrap_err();
    assert_eq!(f.code, 71, "interp arith on bytes old value");
    rv64_must_reject(body, "mixes bytes with int");
}

#[test]
fn s8_uninitialized_high_slot_old_value_is_zero() {
    // first touch is the compound's READ: the frame zero makes the old
    // value U64(0) exactly like the interpreter's zero-initialized local
    let body = "var x $64\nx += (byte(data, 0) as u64)\nreturn x as bytes";
    agree4(body, &inputs_first_bytes());
}

/* ================================================================== */
/* Section 2: T15 — input parameter reassignment + buf_set(data).     */
/* ================================================================== */

#[test]
fn s15_data_reassign_bytes_reads_new_value() {
    // the flipped rv64_gen_corpus P2 pin: interp accepts and runs, and the
    // rv64 lowering now agrees (was: compile-reject)
    let body = "data = buf_right(1, data)\nreturn (byte(data, 0) as u64) as bytes";
    agree4(body, &[vec![7, 9, 1, 2], vec![0, 0, 0, 0], vec![255]]);
}

#[test]
fn s15_data_read_before_and_after_reassign() {
    // reads before the store see the input; reads after see the new value
    let body = "var a = (byte(data, 0) as u64)\n\
                data = 0x0506\n\
                var b = (byte(data, 0) as u64)\n\
                return (((a * 100u64) + b) as u64) as bytes";
    agree4(body, &[vec![7, 9, 1, 2], vec![200, 0, 0, 0]]);
}

#[test]
fn s15_data_reassign_from_expression_using_data() {
    // (no empty input: buf_left(1) of len 0 is the pinned PinD2BoundsClass
    // OOB divergence — interp 42 vs rv64 71 — unrelated to this feature)
    let body = "data = data ++ buf_left(1, data)\n\
                return (size(data) as u64) as bytes";
    agree4(body, &[vec![1, 2], vec![7, 9, 1, 2]]);
}

#[test]
fn s15_data_reassign_conditional_store() {
    // the store sits under an if: reads still see the input on the false
    // path (the entry copy initializes the slot either way). (No empty
    // input here: `byte(data,0)` on len 0 is the pinned PinD2BoundsClass
    // divergence — interp 42 vs rv64 71 — unrelated to this feature.)
    let body = "if ((byte(data, 0) as u64) == 7u64) {\n\
                data = 0x0102\n\
                }\n\
                return (byte(data, 0) as u64) as bytes";
    agree4(body, &[vec![7, 1], vec![8, 1], vec![7, 9]]);
}

#[test]
fn s15_data_reassign_twice_and_return_it() {
    // (inputs are ≥ 2 bytes: buf_left(2) of a 1-byte input is the pinned
    // PinD2BoundsClass OOB divergence, unrelated to this feature)
    let body = "data = buf_left(2, data)\n\
                data = data ++ 0xff\n\
                return data";
    agree4(body, &[vec![1, 2, 3, 4], vec![9, 9]]);
}

#[test]
fn s15_bufset_data_receiver_slot_unchanged() {
    // buf_set patches a freshly extracted copy on the interpreter: `data`
    // itself reads back UNPATCHED; the fault faces agree
    let body = "buf_set(data, 0u64, 85u8)\n\
                return (byte(data, 0) as u64) as bytes";
    agree4(body, &[vec![1, 2, 3, 4], vec![200, 0], vec![]]);
}

#[test]
fn s15_bufset_data_boundary_fault() {
    // ost + src.len > len → StackError 42 on both engines
    let body = "buf_set(data, 100u64, \"QQ\")\n\
                return (byte(data, 0) as u64) as bytes";
    agree4(body, &[vec![1, 2, 3, 4]]);
    // == len is legal (write ending at the end): no fault
    let body = "buf_set(data, 2u64, \"QQ\")\n\
                return 0u64 as bytes";
    agree4(body, &[vec![1, 2, 3, 4]]);
}

#[test]
fn s15_bufset_data_then_reassign() {
    let body = "buf_set(data, 0u64, 85u8)\n\
                data = buf_left(1, data)\n\
                return (((byte(data, 0) as u64) * 10u64) as u64) as bytes";
    agree4(body, &[vec![3, 2, 1], vec![70, 1]]);
}

#[test]
fn s15_xop0_still_rejects() {
    // `data += v` while slot 0 holds the input bytes: the interpreter
    // faults CastFail(71) at runtime (arithmetic_args2 to_uint on Bytes);
    // the kernel subset refuses up front with the pinned message
    let body = "data += 1u64\nreturn 0u64 as bytes";
    let it = Interp::try_build(body).unwrap();
    let f = it.run(&[1, 2, 3, 4]).unwrap_err();
    assert_eq!(f.code, 71, "interp XOP0 on the bytes input");
    rv64_must_reject(body, "input parameter cannot be assigned");
}

#[test]
fn s15_int_reassign_still_rejects() {
    // `data = <int>`: the interpreter rebinds slot 0 to a uint, but later
    // `data` reads would be dynamically typed (bytes before, int after) —
    // outside the one-type-per-slot model, so the kernel refuses (B-family)
    let body = "data = 5u64\nreturn (data as u64) as bytes";
    assert!(Interp::try_build(body).is_ok(), "interp accepts int rebind");
    rv64_must_reject(body, "input parameter cannot be assigned");
}
