//! Targeted differential for the rv64 size-min candidates
//! B11 (buf_set constant-length memcpy specialization) and
//! C9 (item_concat over a Rodata constant table -> pool literal).
//!
//! Same mounting as `calc_aot_differential.rs` (calc container, default
//! ImcRemap + a per-case Im/Imc sweep): interpreter vs AOT agreement plus
//! byte-exact goldens. The goldens are built with the INTERPRETER'S OWN
//! value constructors — `Value::extract_bytes` (canbe.rs, the exact
//! function `item_concat`/`bufset` call) spliced exactly like `operand.rs
//! bufset` — so a pass means the rv64 output is byte-for-byte the
//! interpreter's.
//!
//! B11 matrix: every scalar source width (bool/u8/u16/u32/u64/u128) x
//! alias / non-alias receivers (premise 24: the receiver's bytes are
//! copy-then-patched; a pool/input/alias-backed descriptor must never be
//! written in place) x the 42 boundary (== length legal, past it traps).
//!
//! C9 matrix: Rodata tables at every stride (u8/u16/u32/u64/u128) with
//! 1/3 elements, the generic-path fallbacks (Empty / Growable /
//! FrameStatic), the dynamic 39 face at the exact output_max boundary
//! (== total legal, total-1 traps; custom `output_max` containers), a
//! pool-immutability sink (buf_set over the itemcat result then a second
//! item_concat of the same list), and the mixed-width pack rejection
//! (only same-width tables are admitted — recorded).
//!
//! Run:
//!   cargo test -p testkit --offline --features calc-aot --test b11_c9_bufset_itemcat -- --nocapture

#![cfg(all(feature = "calc-aot", target_arch = "x86_64"))]

use std::collections::HashMap;
use std::sync::Arc;

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::{verify_container_with_cap, CalcModule};
use vm::calc::aot::{aot_compile, AotFunc};
use vm::calc::{CalcAbort2, SpaceCap};
use vm::fitshc::{compile_rv64im_body_kernel, Rv64imBodyMeta};
use vm::ir::rv64::Rv64Mode;
use vm::lang::Tokenizer;
use vm::Value;

const BUDGET: u64 = 50_000_000;

fn cap() -> SpaceCap {
    SpaceCap::new(0)
}

/* ---------------- interpreter-authority golden builders ---------------- */

/// The interpreter's `extract_bytes` for a uint literal (canbe.rs /
/// item.rs scalar_bytes): the width's big-endian bytes.
fn ex_bytes(v: u128, width_bits: u32) -> Vec<u8> {
    let val = match width_bits {
        1 => Value::Bool(v != 0),
        8 => Value::U8(v as u8),
        16 => Value::U16(v as u16),
        32 => Value::U32(v as u32),
        64 => Value::U64(v as u64),
        _ => Value::U128(v),
    };
    val.extract_bytes().unwrap()
}

/// The interpreter's `bufset` (operand.rs): splice the source's
/// extract_bytes at `ost` into a copy of the receiver.
fn bufset_golden(recv: &[u8], ost: usize, src_v: u128, src_bits: u32) -> Vec<u8> {
    let src = ex_bytes(src_v, src_bits);
    let end = ost + src.len();
    assert!(end <= recv.len(), "golden bufset out of range");
    let mut out = recv.to_vec();
    out[ost..end].copy_from_slice(&src);
    out
}

/// The interpreter's `item_concat` (execute.rs ITEMCAT -> concat_call_chunks):
/// each element's extract_bytes concatenated in element order.
fn itemcat_golden(vals: &[u128], width_bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for v in vals {
        out.extend_from_slice(&ex_bytes(*v, width_bits));
    }
    out
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/* ---------------- engine mounting (same as calc_aot_differential) ----- */

struct Engines {
    container: CalcContainer,
    module: CalcModule,
    aot: AotFunc,
}

type Cache = HashMap<(&'static str, String, u16), Arc<Engines>>;

fn mode_key(m: Rv64Mode) -> &'static str {
    match m {
        Rv64Mode::Im => "Im",
        Rv64Mode::Imc => "Imc",
        Rv64Mode::ImcRemap => "ImcRemap",
    }
}

fn compile_body(body: &str, m: Rv64Mode, output_max: u16) -> Vec<u8> {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let meta = Rv64imBodyMeta { mode: m, output_max, ..Rv64imBodyMeta::default() };
    compile_rv64im_body_kernel("b11c9_kernel", tokens, Some("data"), &[], &[], &[], &meta, &cap())
        .unwrap_or_else(|e| panic!("body must compile ({m:?}): {e}\n{body}"))
}

fn engines_for(cache: &mut Cache, body: &str, m: Rv64Mode, output_max: u16) -> Arc<Engines> {
    let key = (mode_key(m), body.to_string(), output_max);
    cache
        .entry(key)
        .or_insert_with(|| {
            let bytes = compile_body(body, m, output_max);
            let (container, _) = CalcContainer::decode(&bytes).expect("container decodes");
            let module = verify_container_with_cap(&container, &cap()).expect("verify");
            let aot = aot_compile(&module).expect("aot compiles");
            Arc::new(Engines { container, module, aot })
        })
        .clone()
}

enum Obs {
    Ok(Vec<u8>),
    Trap(u64),
}

fn observe(o: &Obs) -> String {
    match o {
        Obs::Ok(b) => format!("Ok(0x{})", hexs(b)),
        Obs::Trap(c) => format!("Trap({c})"),
    }
}

fn interp_obs(eng: &Engines, input: &[u8]) -> Obs {
    let (res, _regs, _mem) =
        interp::run_lockstep(&eng.module, &eng.container, input, BUDGET, None);
    match res {
        Ok(r) => Obs::Ok(r.output),
        Err((CalcAbort::GuestExit(code), _used)) => Obs::Trap(code),
        Err((CalcAbort::Trap(_), _used)) => Obs::Trap(0),
        Err(_) => Obs::Trap(u64::MAX),
    }
}

fn aot_obs(eng: &Engines, input: &[u8]) -> Obs {
    let (out, _regs, _mem) = eng.aot.run_with_state(&eng.module, &eng.container, input, BUDGET);
    match out.abort {
        None => Obs::Ok(out.output.unwrap_or_default()),
        Some(CalcAbort2::Trap) => Obs::Trap(out.exit_code),
        Some(_) => Obs::Trap(u64::MAX),
    }
}

/// One case through every mode: interpreter vs AOT must agree, then the
/// golden face is pinned.
fn run_case(
    cache: &mut Cache,
    id: &str,
    body: &str,
    input: &[u8],
    output_max: u16,
    sweep: bool,
    want: &dyn Fn(&Obs) -> Result<(), String>,
) {
    let mut modes = vec![Rv64Mode::ImcRemap];
    if sweep {
        modes.push(Rv64Mode::Im);
        modes.push(Rv64Mode::Imc);
    }
    for m in modes {
        let eng = engines_for(cache, body, m, output_max);
        let io = interp_obs(&eng, input);
        let ao = aot_obs(&eng, input);
        assert_eq!(
            std::mem::discriminant(&io),
            std::mem::discriminant(&ao),
            "[{id} mode {m:?}] interp {} vs aot {} — class mismatch\n{body}",
            observe(&io),
            observe(&ao)
        );
        match (&io, &ao) {
            (Obs::Ok(a), Obs::Ok(b)) => {
                assert_eq!(a, b, "[{id} mode {m:?}] output bytes mismatch\n{body}")
            }
            (Obs::Trap(a), Obs::Trap(b)) => {
                assert_eq!(a, b, "[{id} mode {m:?}] exit code mismatch\n{body}")
            }
            _ => {}
        }
        if let Err(e) = want(&io) {
            panic!("[{id} mode {m:?}] golden mismatch: {e}\n  got {}\n{body}", observe(&io));
        }
    }
}

fn want_ok(expect: &[u8]) -> impl Fn(&Obs) -> Result<(), String> + '_ {
    move |o: &Obs| match o {
        Obs::Ok(b) if b == expect => Ok(()),
        other => Err(format!("expected Ok(0x{}), got {}", hexs(expect), observe(other))),
    }
}

fn want_trap(code: u64) -> impl Fn(&Obs) -> Result<(), String> {
    move |o: &Obs| match o {
        Obs::Trap(c) if *c == code => Ok(()),
        other => Err(format!("expected Trap({code}), got {}", observe(other))),
    }
}

/* ================================================================== */
/* B11: buf_set scalar sources (constant-length patch copy)            */
/* ================================================================== */

#[test]
fn b11_bufset_scalar_widths() {
    let mut cache = Cache::new();
    let recv = b"abcdefgh";
    // every scalar width, non-alias receiver, static ost
    for (id, bits, val, ost) in [
        ("bool", 1u32, 1u128, 3usize),
        ("u8", 8, 0x5A, 0),
        ("u16", 16, 0xBEEF, 1),
        ("u32", 32, 0x01020304, 2),
        ("u64", 64, 0x1122334455667788, 0),
    ] {
        let lit = if bits == 1 {
            "true".to_string()
        } else {
            format!("{val}u{bits}")
        };
        let body = format!("var b = \"abcdefgh\"\nvar x = buf_set(b, {ost}u64, {lit})\nreturn x");
        let want_bytes = bufset_golden(recv, ost, val, bits);
        run_case(&mut cache, id, &body, &[0, 0, 0, 0], 1280, true, &want_ok(&want_bytes));
    }
    // u128: the receiver must be 16 bytes for the full patch
    let body = "var b = \"abcdefghijklmnop\"\nvar x = buf_set(b, 0u64, 1339673755198158349044581307228491536u128)\nreturn x";
    run_case(
        &mut cache,
        "u128",
        body,
        &[0, 0, 0, 0],
        1280,
        true,
        &want_ok(&bufset_golden(b"abcdefghijklmnop", 0, 1339673755198158349044581307228491536, 128)),
    );
}

#[test]
fn b11_bufset_alias_receivers() {
    let mut cache = Cache::new();
    // premise 24, slot alias: `var c = b` aliases the same bytes — after
    // buf_set(b, ..) BOTH b and c must read the pre-patch bytes, only the
    // result carries the patch
    let body = "var b = \"abcdefgh\"\nvar c = b\nvar x = buf_set(b, 0u64, 43707u16)\nvar r = (byte(c, 0) as u64) * 1000000u64 + (byte(b, 0) as u64) * 1000u64 + (byte(x, 0) as u64)\nreturn (r as u64) as bytes";
    let want = {
        let c0 = b'a' as u128;
        let r = c0 * 1_000_000 + c0 * 1000 + 0xAA;
        ex_bytes(r, 64)
    };
    run_case(&mut cache, "alias.slot", body, &[0, 0, 0, 0], 1280, true, &want_ok(&want));

    // premise 24, input receiver: buf_set patches a COPY — the data slot
    // itself only changes through the explicit rebind
    let body = "data = buf_set(data, 2u64, 43707u16)\nreturn data";
    let input = [17u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    run_case(&mut cache, "alias.input", body, &input, 1280, true, &want_ok(&bufset_golden(&input, 2, 0xAABB, 16)));
}

#[test]
fn b11_bufset_boundary_and_dynamic_ost() {
    let mut cache = Cache::new();
    // ost + src.len == receiver len is legal
    let body = "var b = \"abcdefgh\"\nvar x = buf_set(b, 6u64, 43707u16)\nreturn x";
    run_case(&mut cache, "end.eq", body, &[0, 0, 0, 0], 1280, false, &want_ok(&bufset_golden(b"abcdefgh", 6, 0xAABB, 16)));
    // one past the end traps StackError 42
    let body = "var b = \"abcdefgh\"\nvar x = buf_set(b, 7u64, 43707u16)\nreturn x";
    run_case(&mut cache, "end.over", body, &[0, 0, 0, 0], 1280, false, &want_trap(42));
    // dynamic ost with a u16 scalar source (runtime patch offset)
    let body = "var b = \"abcdefgh\"\nvar o = (byte(data, 0) as u64) % 8u64\nvar x = buf_set(b, o, 3823u16)\nreturn x";
    for o in [0u8, 3, 6] {
        let want = bufset_golden(b"abcdefgh", o as usize, 0x0EEF, 16);
        run_case(&mut cache, "dyn.ost", body, &[o, 0, 0, 0], 1280, false, &want_ok(&want));
    }
}

/* ================================================================== */
/* C9: item_concat over Rodata constant tables -> pool literal         */
/* ================================================================== */

#[test]
fn c9_itemcat_rodata_widths() {
    let mut cache = Cache::new();
    // every table stride, 1..3 elements — golden = the interpreter's
    // item_concat (extract_bytes per element, concatenated in order)
    for (id, bits, vals) in [
        ("u8.3", 8u32, vec![1u128, 2, 0xFF]),
        ("u16.2", 16, vec![258u128, 0x0304]),
        ("u32.1", 32, vec![16909060u128]),
        ("u64.1", 64, vec![7u128]),
        ("u64.2", 64, vec![1u128, 2]),
        ("u64.3", 64, vec![72623859790382856u128, 0xFF, 0]),
        ("u128.1", 128, vec![1339673755198158349044581307228491536u128]),
        ("u128.2", 128, vec![1u128, 0x000102030405060708090a0b0c0d0e0f10]),
    ] {
        let lits: Vec<String> = vals.iter().map(|v| format!("{v}u{bits}")).collect();
        let body = format!("var l = [{}]\nreturn item_concat(l)", lits.join(", "));
        let want = itemcat_golden(&vals, bits);
        assert_eq!(want.len(), vals.len() * (bits as usize / 8));
        run_case(&mut cache, id, &body, &[0, 0, 0, 0], 1280, true, &want_ok(&want));
    }
}

#[test]
fn c9_itemcat_fallbacks() {
    let mut cache = Cache::new();
    // Empty class: empty bytes
    let body = "var l = []\nreturn item_concat(l)";
    run_case(&mut cache, "empty", body, &[0, 0, 0, 0], 1280, true, &want_ok(&[]));
    // Growable (APPEND reachable): the generic runtime splice
    let body = "var l = []\nappend(l, 1u64)\nappend(l, 2u64)\nreturn item_concat(l)";
    run_case(&mut cache, "growable", body, &[0, 0, 0, 0], 1280, true, &want_ok(&itemcat_golden(&[1, 2], 64)));
    // FrameStatic (computed elements): the generic runtime splice
    let body = "var a = (byte(data, 0) as u64)\nvar l = [a, a + 1u64]\nreturn item_concat(l)";
    run_case(&mut cache, "framestatic", body, &[5, 0, 0, 0], 1280, true, &want_ok(&itemcat_golden(&[5, 6], 64)));
}

#[test]
fn c9_itemcat_39_boundary() {
    let mut cache = Cache::new();
    // [1u64, 2u64] -> total 16 bytes: output_max == total is legal,
    // output_max == total-1 traps OutOfValueSize 39
    let body = "var l = [1u64, 2u64]\nreturn item_concat(l)";
    let want = itemcat_golden(&[1, 2], 64);
    assert_eq!(want.len(), 16);
    run_case(&mut cache, "om.eq", body, &[0, 0, 0, 0], 16, false, &want_ok(&want));
    run_case(&mut cache, "om.gt", body, &[0, 0, 0, 0], 17, false, &want_ok(&want));
    run_case(&mut cache, "om.lt", body, &[0, 0, 0, 0], 15, false, &want_trap(39));
    // u128 table: 2 x 16 = 32 bytes at the boundary
    let body = "var l = [1u128, 2u128]\nreturn item_concat(l)";
    let want = itemcat_golden(&[1, 2], 128);
    assert_eq!(want.len(), 32);
    run_case(&mut cache, "om128.eq", body, &[0, 0, 0, 0], 32, false, &want_ok(&want));
    run_case(&mut cache, "om128.lt", body, &[0, 0, 0, 0], 31, false, &want_trap(39));
}

#[test]
fn c9_itemcat_pool_immutability() {
    let mut cache = Cache::new();
    // the itemcat result is a read-only pool descriptor: buf_set over it
    // must copy-then-patch (premise 24), and a second item_concat of the
    // same list must still read the ORIGINAL table bytes
    let body = "var l = [1u64, 2u64]\nvar x = item_concat(l)\nvar y = buf_set(x, 0u64, 255u8)\nvar z = item_concat(l)\nreturn y ++ z";
    let base = itemcat_golden(&[1, 2], 64);
    let mut y = base.clone();
    y[0] = 0xFF;
    let mut want = y;
    want.extend_from_slice(&base);
    run_case(&mut cache, "sink.bufset", body, &[0, 0, 0, 0], 1280, true, &want_ok(&want));
}

/// Only same-width constant tables are admitted — a mixed-width literal
/// pack is a compile reject (the slot's fixed element stride cannot
/// represent two widths). Recorded per the C9 checklist.
#[test]
fn c9_itemcat_mixed_width_rejected() {
    let body = "var l = [1u8, 2u64]\nreturn item_concat(l)";
    let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
    let meta = Rv64imBodyMeta::default();
    let r = compile_rv64im_body_kernel("mixed", tokens, Some("data"), &[], &[], &[], &meta, &cap());
    assert!(r.is_err(), "mixed-width pack must be rejected: {r:?}");
}
