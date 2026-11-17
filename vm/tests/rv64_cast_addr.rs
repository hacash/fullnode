//! T13 (CTO relaxed casts + the address value model): interpreter-vs-rv64
//! differential pins, engine-vs-engine on the rv64 channel.
//!
//! Ground truth (vm/src/value/cast.rs + convert.rs + item.rs):
//! * `as bool` (CTO Bool) — `extract_bool`: uint ≠ 0, Nil → false,
//!   Bytes/Address → any non-zero byte.
//! * `as u8..u128` on a bytes/address operand — `bytes_to_uint_width` /
//!   relaxed `to_u128`: BE leading-zero-trimmed, the non-zero remainder
//!   must fit the target width, else CastFail(71).
//! * `as bytes` on a bytes/address operand — `scalar_bytes` identity.
//! * `as address` (CTO Address) — `cast_addr`: cast to bytes (Nil → 71),
//!   then exactly 21 bytes AND a decodable version byte, else 71.
//! * address literal = `CTO(Address, PBUF 21B)` — a 21-byte value with the
//!   static Address stain; EQ/NEQ is `value_content_eq` (raw bytes;
//!   Address vs a non-21-byte Bytes faults Arithmetic 90).
//!
//! Every row is an agree4 (interpreter == all three rv64 modes, value or
//! exit code) unless it pins an rv64 compile reject of a deterministic
//! interpreter fault (the legal fail-fast closure of the iron law).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

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

/// The 21-byte-address-output flavor: the default C2 envelope caps the
/// output channel at 16 bytes (an address return legitimately faults the
/// kernel's output budget there); this builds with the wide `bigout`
/// envelope so address-sized outputs are comparable.
fn agree4_out(body: &str, inputs: &[Vec<u8>]) {
    // the bigout envelope: output 64 over a 28 KiB arena (the full
    // 0x9D30 arena + input 4800 + output 64 overflows the 64 KiB cap)
    let rv = Rv64::try_build_out(body, 64, 28 * 1024)
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

// version 1 (contract), payload 0x11 — all bytes non-zero
const A_V1: &str = "RtqvBaudCALQXgZRiRUT8kuYCsjxPFk8P";
// version 5 (scriptmh), payload 0x22
const A_V5: &str = "34oVnh4gNviJGMnNvgquMeLAxvXJuaRVMZ";
// version 0 (privkey), payload 0x33
const A_V0: &str = "15fioDrrk36NmHdRCtGTu2TMj6rPXzG3sn";
// version 1, payload 0x44 — differs from A_V1
const A_V1B: &str = "WZZiQSmNEFiApJydbgwMADGGJj8Sny6bq";
// version 0 with SIX leading zero bytes: 00 00 00 00 00 00 77×15 — the
// BE-trimmed non-zero prefix is 15 bytes (≤ 16): uint-castable as u128
const A_Z15: &str = "111111NsWePLDZ2UwKrRHese4EBiBV9h";

/* ================================================================== */
/* Section 1: `as bool` (CTO Bool) — truthiness                        */
/* ================================================================== */

#[test]
fn t13_as_bool_uint() {
    agree4(
        "var a = byte(data, 0) as u64\nreturn (a as bool) as u64 as bytes",
        &[vec![0], vec![1], vec![255]],
    );
    agree4("return (5u64 as bool) as u64 as bytes", &[vec![]]);
    agree4("return (0u64 as bool) as u64 as bytes", &[vec![]]);
    // u128 pair truthiness: (lo|hi) != 0
    agree4(
        "var a = byte(data, 0) as u128\nreturn (a as bool) as u64 as bytes",
        &[vec![0], vec![9]],
    );
    agree4(
        "return ((1u128 << 100u128) as bool) as u64 as bytes",
        &[vec![]],
    );
    agree4("return (0u128 as bool) as u64 as bytes", &[vec![]]);
}

#[test]
fn t13_as_bool_bytes() {
    agree4(
        "return (data as bool) as u64 as bytes",
        &[vec![], vec![0, 0, 0], vec![0, 0, 1], vec![255]],
    );
    // slot round-trip keeps the bytes face
    agree4(
        "var b = buf_left(2, data)\nreturn (b as bool) as u64 as bytes",
        &[vec![0, 0], vec![0, 7], vec![9, 0]],
    );
}

#[test]
fn t13_as_bool_address() {
    // a version-1 address always has a non-zero byte → true
    agree4(
        &format!("return ({A_V1} as bool) as u64 as bytes"),
        &[vec![]],
    );
    // an address whose 21 bytes are ALL zero (version 0 + zero payload)
    // decodes fine and is falsy — extract_bool has no version special
    // case. (The all-zero ADDRESS LITERAL is not writable in fitsh source
    // — the tokenizer takes the leading-'1' run as a number — so the row
    // reaches the value through the runtime bytes cast.)
    agree4(
        "var a = (data as address)\nreturn (a as bool) as u64 as bytes",
        &[vec![0; 21], {
            let mut v = vec![1u8];
            v.extend([0; 20]);
            v
        }],
    );
    agree4(
        &format!("var a = {A_V0}\nreturn (a as bool) as u64 as bytes"),
        &[vec![]],
    );
}

/* ================================================================== */
/* Section 2: `as u8..u128` on bytes/address (BE trim + width fit)     */
/* ================================================================== */

#[test]
fn t13_as_uint_bytes() {
    agree4(
        "return (data as u8) as bytes",
        &[
            vec![],
            vec![5],
            vec![0, 5],      // leading zero trimmed, fits 1 byte
            vec![1, 5],      // non-zero prefix of 2 bytes → 71
            vec![255],
            vec![0, 0, 0, 0],
        ],
    );
    agree4(
        "return (data as u16) as bytes",
        &[vec![], vec![1, 2], vec![0, 0, 1, 2], vec![1, 2, 3]],
    );
    agree4(
        "return (data as u64) as bytes",
        &[vec![], vec![0, 0, 0, 0, 0, 0, 0, 9], vec![1; 8], vec![1; 9]],
    );
    agree4(
        "return (data as u128) as bytes",
        &[vec![], vec![0; 17], vec![7; 16], vec![7; 17], vec![0; 5]],
    );
}

#[test]
fn t13_as_uint_address() {
    // six leading zero bytes → a 15-byte non-zero prefix: the u128 value
    // of 0x0077..77 (15 payload bytes)
    agree4(
        &format!("return ({A_Z15} as u128) as bytes"),
        &[vec![]],
    );
    // a u64-sized one: 13 zero bytes + 8 payload bytes
    agree4(
        "var a = 11111111111115qCHTcgbQwpuTX8cA\nvar v = (a as address)\nreturn (v as u64) as bytes",
        &[vec![]],
    );
    // the runtime faces the LITERAL rows cannot reach (the shared front
    // end folds literal casts and rejects an overflowing one up front —
    // both engines see the same compile error): version 1 → a 21-byte
    // non-zero prefix faults 71; the 15-byte prefix value overflows u64
    // (71). Slot-stored addresses keep the runtime cast path.
    let v1_21 = {
        let mut v = vec![1u8];
        v.extend([0x11; 20]);
        v
    };
    let z15_21 = {
        let mut v = vec![0u8; 6];
        v.extend([0x77; 15]);
        v
    };
    agree4(
        "var a = (data as address)\nreturn (a as u128) as bytes",
        &[v1_21.clone(), z15_21.clone()],
    );
    agree4(
        "var a = (data as address)\nreturn (a as u64) as bytes",
        &[v1_21, z15_21],
    );
    // the literal forms of the two 71 faces: the front end's
    // check_literal_as_cast rejects them for BOTH engines
    for body in [
        format!("return ({A_V1} as u128) as bytes"),
        format!("return ({A_Z15} as u64) as bytes"),
    ] {
        let e1 = Interp::try_build(&body).err().unwrap_or_else(|| panic!("interp must reject: {body}"));
        let e2 = Rv64::try_build(&body).err().unwrap_or_else(|| panic!("rv64 must reject: {body}"));
        assert!(e1.contains("CastFail"), "interp: {e1}");
        assert!(e2.contains("CastFail"), "rv64: {e2}");
    }
}

/* ================================================================== */
/* Section 3: `as bytes` identity on bytes/address                     */
/* ================================================================== */

#[test]
fn t13_as_bytes_identity() {
    agree4("return data as bytes", &[vec![1, 2, 3], vec![]]);
    agree4(
        "var b = buf_left(2, data)\nreturn b as bytes",
        &[vec![7, 8, 9]],
    );
    // an address's scalar_bytes is its 21 content bytes
    agree4_out(&format!("return {A_V5} as bytes"), &[vec![]]);
    agree4_out(
        &format!("var a = {A_V1}\nreturn a as bytes"),
        &[vec![]],
    );
}

/* ================================================================== */
/* Section 4: `as address` (CTO Address) — 21B + version decode        */
/* ================================================================== */

#[test]
fn t13_as_address_runtime_faces() {
    // observe through `as bytes` (identity) and the fault face
    agree4_out(
        "var a = (data as address)\nreturn a as bytes",
        &[
            vec![1, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11], // ok v1
            vec![5, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22], // ok v5
            vec![2, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33], // version 2 → 71
            vec![1, 0x11],                                   // too short → 71
            vec![1; 22],                                     // too long → 71
            vec![],
        ],
    );
    // size() of the cast address is 21 on both engines
    agree4(
        "return (size(data as address)) as bytes",
        &[vec![1; 21], vec![1; 20]],
    );
}

#[test]
fn t13_as_address_int_operand_rejects() {
    // a uint's scalar_bytes is 1/2/4/8/16 bytes — never 21: the
    // interpreter faults 71 deterministically; the kernel fails fast
    rv64_must_reject(
        "var a = byte(data, 0) as u64\nvar b = (a as address)\nreturn b as bytes",
        "as address",
    );
}

/* ================================================================== */
/* Section 5: the address literal + EQ/NEQ value model                 */
/* ================================================================== */

#[test]
fn t13_address_literal_return() {
    agree4_out(&format!("return {A_V1}"), &[vec![]]);
    agree4_out(&format!("var a = {A_V5}\nreturn a"), &[vec![]]);
}

#[test]
fn t13_address_eq_neq() {
    // literal vs literal
    agree4(
        &format!("var r = ({A_V1} == {A_V1})\nreturn (r as u64) as bytes"),
        &[vec![]],
    );
    agree4(
        &format!("var r = ({A_V1} != {A_V1B})\nreturn (r as u64) as bytes"),
        &[vec![]],
    );
    // literal vs the input bytes (Bytes vs Address): 21-length gate + memcmp
    let raw_v1 = {
        // craft inputs by hand: 21 bytes with version 1 + 0x11 payload
        // equals A_V1
        let mut v = vec![1u8];
        v.extend([0x11; 20]);
        v
    };
    let mut other21 = vec![5u8];
    other21.extend([0x22; 20]);
    agree4(
        &format!("var r = (data == {A_V1})\nreturn (r as u64) as bytes"),
        &[raw_v1.clone(), other21, vec![1; 20], vec![1; 22], vec![]],
    );
    // reversed operand order (Address vs Bytes on the left)
    agree4(
        &format!("var r = ({A_V1} == data)\nreturn (r as u64) as bytes"),
        &[raw_v1, vec![1; 20]],
    );
    // slot-stored address vs address literal: both compare as bytes
    agree4(
        &format!(
            "var a = {A_V1}\nvar r = (a == {A_V1})\nreturn (r as u64) as bytes"
        ),
        &[vec![]],
    );
    agree4(
        &format!(
            "var a = {A_V1}\nvar r = (a != {A_V1B})\nreturn (r as u64) as bytes"
        ),
        &[vec![]],
    );
    // bytes vs bytes NEQ with differing lengths (no fault — just unequal)
    agree4(
        "var b = buf_left(2, data)\nvar r = (data != b)\nreturn (r as u64) as bytes",
        &[vec![1, 2], vec![1, 2, 3], vec![3, 3]],
    );
    // bytes vs bytes equal
    agree4(
        "var r = (data == data)\nreturn (r as u64) as bytes",
        &[vec![1, 2, 3], vec![]],
    );
}

#[test]
fn t13_address_eq_in_branch() {
    // the branch-condition form of EQ (if) rides the materialize+branch path
    agree4(
        &format!(
            "if data == {A_V1} {{ return (1u64) as bytes }}\nreturn (0u64) as bytes"
        ),
        &[
            {
                let mut v = vec![1u8];
                v.extend([0x11; 20]);
                v
            },
            vec![1; 21],
            vec![1; 20],
        ],
    );
    agree4(
        "if data == data { return (1u64) as bytes }\nreturn (0u64) as bytes",
        &[vec![1, 2], vec![]],
    );
}

#[test]
fn t13_address_eq_int_mix_rejects() {
    // int vs bytes/address: the interpreter faults 90 deterministically —
    // compile-time fail-fast
    rv64_must_reject(
        "var r = (data == 1u64)\nreturn (r as u64) as bytes",
        "cannot compare",
    );
    rv64_must_reject(
        &format!("var r = ({A_V1} == 1u64)\nreturn (r as u64) as bytes"),
        "cannot compare",
    );
}

#[test]
fn t13_address_arith_still_rejects() {
    // XOP on an address slot: the interpreter faults 71 (to_uint on an
    // Address) — the kernel's bytes/int slot split rejects up front
    rv64_must_reject(
        &format!("var a = {A_V1}\na += 1u64\nreturn a as bytes"),
        "mixes bytes with int",
    );
}

#[test]
fn t13_address_choose_arms() {
    // two Address arms unify as 2-entry values
    agree4(
        &format!(
            "var c = (byte(data, 0) % 2u64)\nvar a = choose(c == 0u64, {A_V1}, {A_V1B})\nvar r = (a == {A_V1B})\nreturn (r as u64) as bytes"
        ),
        &[vec![0], vec![1]],
    );
    // Address vs Bytes arms keep the mixed-arm reject
    rv64_must_reject(
        &format!(
            "var c = (byte(data, 0) % 2u64)\nvar a = choose(c == 0u64, {A_V1}, \"xx\")\nreturn a as bytes"
        ),
        "choose arms must have the same result type",
    );
}

/* ================================================================== */
/* Section 6: `return <address>` and address bytes consumers           */
/* ================================================================== */

#[test]
fn t13_address_bytes_consumers() {
    // CAT splices the 21 content bytes (extract_bytes coercion)
    agree4(
        &format!("var b = ({A_V1} ++ data)\nreturn (size(b)) as bytes"),
        &[vec![], vec![1, 2, 3]],
    );
    // byte() indexes into the 21 content bytes
    agree4(
        &format!("return (byte({A_V1}, 0)) as bytes"),
        &[vec![]],
    );
    agree4(
        &format!("return (byte({A_V1}, 20)) as bytes"),
        &[vec![]],
    );
    // buf_cut() on an address (extract_bytes coercion)
    agree4(
        &format!("var c = buf_cut({A_V1}, 0, 3)\nreturn c"),
        &[vec![]],
    );
}
