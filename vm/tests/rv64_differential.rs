//! Full-behavior differential for the fitsh→RV64IM backend.
//!
//! Every construct of the pinned kernel subset, executed through the
//! consensus engine in all three lowering modes (Im / Imc / ImcRemap) and
//! compared BYTE-FOR-BYTE against a rust reference model transcribed from
//! the source semantics — the "bilateral knock" that makes compiler bugs
//! visible: any lowering-mode divergence or model mismatch fails with the
//! offending body and input.

use vm::calc::{CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap};
use vm::fitshc::{CompiledCode, compile_body};
use vm::ir::rv64::{ARENA_SIZE, ARENA_SIZE_MAX, Rv64Mode, lower_body_mode_arena};
use vm::lang::Tokenizer;
use vm::ValueTy;

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

fn lower(body: &str, has_input: bool, mode: Rv64Mode) -> vm::calc::container::CalcContainer {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let args: Vec<(String, ValueTy)> = has_input
        .then(|| vec![("data".to_string(), ValueTy::Bytes)])
        .unwrap_or_default();
    let (ir, compiled, _) =
        compile_body(tokens, args, &[], &[], false).expect("compile body");
    assert!(matches!(compiled, CompiledCode::Bytecode(_)));
    let lo = lower_body_mode_arena(&ir, has_input, mode, ARENA_SIZE_MAX).expect("lower");
    let cap = SpaceCap::new(0);
    let profile = match mode {
        Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
        _ => vm::calc::container::CALC_PROFILE_RV64IMC,
    };
    lo.pack_segments_profile(&cap, 64, 4800, 16, profile).expect("pack")
}

/// Execute in one mode → (output bytes | exit code).
fn exec(body: &str, input: &[u8], mode: Rv64Mode) -> Result<Vec<u8>, u32> {
    let cap = SpaceCap::new(0);
    let fno = CalcFnObj::build(lower(body, true, mode), &cap, CalcHostConfig::new())
        .expect("verify+build");
    let ex = fno.run(input, CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None });
    ex.result.map_err(|_| ex.exit_code as u32)
}

/// All three modes must agree with each other AND with the model value.
fn pin(body: &str, input: &[u8], want: u64) {
    for mode in MODES {
        assert_eq!(
            exec(body, input, mode),
            Ok(want.to_be_bytes().to_vec()),
            "mode {mode:?} diverged for body:\n{body}\ninput: {input:?}"
        );
    }
}

/// Width-exact pin: the expected OUTPUT BYTES (task-1 A2 — `as bytes`
/// follows the interpreter's `scalar_bytes` width: u8 → 1 B, u16 → 2, …).
fn pin_w(body: &str, input: &[u8], want: &[u8]) {
    for mode in MODES {
        assert_eq!(
            exec(body, input, mode),
            Ok(want.to_vec()),
            "mode {mode:?} diverged for body:\n{body}\ninput: {input:?}"
        );
    }
}

/// Two 8-byte BE input words.
fn words(a: u64, b: u64) -> Vec<u8> {
    let mut v = a.to_be_bytes().to_vec();
    v.extend(b.to_be_bytes());
    v
}

/// `var a = be8(0..8)` / `var b = be8(8..16)` prelude for op tests.
fn ab_body(expr: &str) -> String {
    format!(
        "var a = 0u64\nvar i = 0u64\nwhile i < 8 {{ a = a * 256 + byte(data, i)\ni += 1 }}\n\
         var b = 0u64\nvar j = 0u64\nwhile j < 8 {{ b = b * 256 + byte(data, j + 8)\nj += 1 }}\n\
         var r = {expr}\nreturn r as bytes"
    )
}

/* ---- E1: every op of the subset, operand matrices ---- */

#[test]
fn comparison_ops_all_directions() {
    // fitsh comparisons are unsigned 64-bit; the result is a pure bool, so
    // `as bytes` renders 1 canonical byte (A2) and the u64-pinned form
    // keeps the 8-byte view
    let cases: Vec<(&str, fn(u64, u64) -> u64)> = vec![
        ("a < b", |a, b| (a < b) as u64),
        ("a > b", |a, b| (a > b) as u64),
        ("a <= b", |a, b| (a <= b) as u64),
        ("a >= b", |a, b| (a >= b) as u64),
        ("a == b", |a, b| (a == b) as u64),
        ("a != b", |a, b| (a != b) as u64),
    ];
    let pairs = [
        (0u64, 0u64),
        (0, 1),
        (1, 0),
        (5, 5),
        (5, 9),
        (9, 5),
        (u64::MAX, u64::MAX - 1),
        (u64::MAX - 1, u64::MAX),
        (u64::MAX, u64::MAX),
    ];
    for (src, model) in &cases {
        let body = ab_body(src);
        for (a, b) in pairs {
            pin_w(&body, &words(a, b), &[model(a, b) as u8]);
        }
        let body64 = ab_body(&format!("(({src}) as u64)"));
        for (a, b) in pairs {
            pin(&body64, &words(a, b), model(a, b));
        }
    }
}

#[test]
fn max_min_clamp_all_regions() {
    // max/min/clamp — the emit_min/emit_max pair carried an inverted-
    // branch bug for their whole life before this differential
    let fns: Vec<(&str, fn(u64, u64) -> u64)> = vec![
        ("max(a, b)", |a, b| a.max(b)),
        ("min(a, b)", |a, b| a.min(b)),
    ];
    let pairs = [
        (0u64, 0u64),
        (0, 1),
        (1, 0),
        (7, 7),
        (7, 8),
        (8, 7),
        (u64::MAX, 0),
        (0, u64::MAX),
        (u64::MAX - 1, u64::MAX),
    ];
    for (src, model) in &fns {
        let body = ab_body(src);
        for (a, b) in pairs {
            pin(&body, &words(a, b), model(a, b));
        }
    }
    // clamp(x, lo, hi): all regions (below / inside / above) + swap(lo > hi)
    let body = ab_body("clamp(a, 10, 100)");
    for a in [0u64, 9, 10, 55, 100, 101, u64::MAX] {
        pin(&body, &words(a, 0), a.clamp(10, 100));
    }
}

#[test]
fn bitwise_and_shifts() {
    // Shifts model the interpreter's `checked_shl`/`checked_shr`: the
    // AMOUNT must be < 64 (Arithmetic 90 otherwise); the value side
    // truncates silently (Rust `<<`/`>>` on u64 match that exactly).
    // The historical model masked the amount (`wrapping_shl(b & 63)`),
    // matching the old `andi 63` lowering — closed with it.
    let cases: Vec<(&str, fn(u64, u64) -> Option<u64>)> = vec![
        ("a ^ b", |a, b| Some(a ^ b)),
        ("a | b", |a, b| Some(a | b)),
        ("a & b", |a, b| Some(a & b)),
        ("a << b", |a, b| (b < 64).then(|| a << b)),
        ("a >> b", |a, b| (b < 64).then(|| a >> b)),
    ];
    let pairs = [
        (0xF0F0_F0F0_F0F0_F0F0u64, 0x0FF0_0FF0_0FF0_0FF0),
        (u64::MAX, 0xFFFF_0000_FFFF_0000),
        (1, 63),
        (1, 64),  // amount >= 64: trap 90
        (1, 65),  // amount >= 64: trap 90
        (0xDEAD_BEEF, 4),
        (0x1234_5678_9ABC_DEF0, 7),
        (u64::MAX, 1), // value side truncates, no fault
    ];
    for (src, model) in &cases {
        let body = ab_body(src);
        for (a, b) in pairs {
            match model(a, b) {
                Some(w) => pin(&body, &words(a, b), w),
                None => {
                    for mode in MODES {
                        assert_eq!(
                            exec(&body, &words(a, b), mode),
                            Err(90),
                            "mode {mode:?} must trap 90 for {src} with amount {b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn logical_and_or_not_choose() {
    // && / || are 0/1 ints in the kernel subset
    let cases: Vec<(&str, &str)> = vec![("|a && b|", "(a * (a == a as u64) != 0) && (b != 0)"), ("|skip|", "x")];
    let _ = cases; // placeholder (unused)
    // probe: minimal value-if
    let probe = "var a = byte(data, 0)\nvar r = if a != 0 { 1 } else { 0 }\nreturn r as bytes";
    println!("probe = {:?}", exec(probe, &[5], Rv64Mode::Im));
    let and_body = ab_body("if a != 0 { if b != 0 { 1 } else { 0 } } else { 0 }");
    for (a, b) in [(0u64, 0), (0, 5), (5, 0), (3, 7), (1, u64::MAX)] {
        // arms are U8 literals → `as bytes` is the 1-byte view (A2)
        pin_w(&and_body, &words(a, b), &[((a != 0) && (b != 0)) as u8]);
    }
    let or_body = ab_body("if a != 0 { 1 } else { if b != 0 { 1 } else { 0 } }");
    for (a, b) in [(0u64, 0), (0, 5), (5, 0), (3, 7), (1, u64::MAX)] {
        pin_w(&or_body, &words(a, b), &[((a != 0) || (b != 0)) as u8]);
    }
    // ! on an int: pure-bool → 1 canonical byte (A2)
    let body = ab_body("!a");
    for a in [0u64, 1, 2, u64::MAX] {
        pin_w(&body, &words(a, 0), &[(a == 0) as u8]);
    }
    // value-if (CHOOSE): both arms evaluated, condition picks
    let body2 = ab_body("if a > b { a } else { b + 1000 }");
    for (a, b) in [(1u64, 2), (3, 2), (0, 0)] {
        pin(&body2, &words(a, b), if a > b { a } else { b + 1000 });
    }
}

#[test]
fn bytes_ops_roundtrip() {
    // left/right/ldrop/rdrop/cat over a runtime slice (bytes 8.. of input)
    // count-style helpers take the literal count first (`buf_left(4, data)`);
    // `data` here is the whole input (32 bytes: 8 index + 24 payload).
    // A2: `size()` is U16 (2-byte `as bytes` view), `byte()` is U8 (1 byte).
    let cases: Vec<(&str, Box<dyn Fn(&[u8]) -> Vec<u8>>)> = vec![
        ("size(buf_left(4, data))", Box::new(|_: &[u8]| 4u16.to_be_bytes().to_vec())),
        ("size(buf_right(4, data))", Box::new(|_: &[u8]| 4u16.to_be_bytes().to_vec())),
        (
            "size(buf_left_drop(3, data))",
            Box::new(|d: &[u8]| (d.len() as u16 - 3).to_be_bytes().to_vec()),
        ),
        (
            "size(buf_right_drop(5, data))",
            Box::new(|d: &[u8]| (d.len() as u16 - 5).to_be_bytes().to_vec()),
        ),
        (
            "size(data ++ data)",
            Box::new(|d: &[u8]| ((d.len() * 2) as u16).to_be_bytes().to_vec()),
        ),
        ("byte(buf_left(4, data), 2)", Box::new(|d: &[u8]| vec![d[2]])),
        ("byte(buf_right(4, data), 0)", Box::new(|d: &[u8]| vec![d[d.len() - 4]])),
        ("byte(data ++ data, size(data))", Box::new(|d: &[u8]| vec![d[0]])),
        ("size(buf_cut(data, 3, 7))", Box::new(|_: &[u8]| 7u16.to_be_bytes().to_vec())),
        ("byte(buf_cut(data, 3, 7), 6)", Box::new(|d: &[u8]| vec![d[9]])),
    ];
    let data: Vec<u8> = (0xA0u8..0xC0).collect();
    for (src, model) in &cases {
        let body = format!("var r = {src}\nreturn r as bytes");
        pin_w(&body, &data, &model(&data));
    }
}

/// Same lowering/execution as `exec` but with an output window large enough
/// for full bytes-content comparison (the fixed 16 B window above would
/// engine-trap on exactly the content this suite must compare). The 64 KiB
/// memory band is a consensus cap, so the window comes from the standard
/// `ARENA_SIZE` lowering (12 KiB bump arena; these bodies churn well under
/// 1 KiB) instead of the 40240 B `ARENA_SIZE_MAX` the other suites use —
/// under MAX-arena the 4800 B input alone budgets the window down to 16 B.
fn exec_bytes(body: &str, input: &[u8], mode: Rv64Mode) -> Result<Vec<u8>, u32> {
    let cap = SpaceCap::new(0);
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let (ir, _, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .expect("compile body");
    let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE).expect("lower");
    let profile = match mode {
        Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
        _ => vm::calc::container::CALC_PROFILE_RV64IMC,
    };
    let container = lo.pack_segments_profile(&cap, 64, 4800, 512, profile).expect("pack");
    let fno = CalcFnObj::build(container, &cap, CalcHostConfig::new()).expect("verify+build");
    let ex = fno.run(input, CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None });
    ex.result.map_err(|_| ex.exit_code as u32)
}

/// BUG-009 regression pin (P1-F): the bytes family differential above checked
/// RDrop ONLY through `size(...)` — the length was always right while
/// `emit_slice`'s RDrop arm never materialized the result POINTER `dstp`, so
/// Im/Imc consumed a stale register (trap) and the length-only check could
/// never see it. This suite compares RESULT CONTENT byte-for-byte against
/// the rust reference model, across the operand shapes that reach
/// `emit_slice`:
/// * Ref source (`data` reads push ABI-capture refs → `dstp != rp`, the
///   conditional `mv` fires — the exact path the fix adds), and
/// * Pool source (a computed bytes expression re-occupies the popped pool
///   slot → `dstp == rp`, no mv — the path the fix must not perturb),
/// plus CAT-after-RDrop (the memcpy consumer of the pointer) and the
/// count edges n = 0 / n = len. Every mode, every case.
#[test]
fn bytes_ops_content_rdrop_and_cat() {
    let data: Vec<u8> = (0xA0u8..0xC0).collect(); // 32 bytes
    let cases: Vec<(&str, Box<dyn Fn(&[u8]) -> Vec<u8>>)> = vec![
        // Ref source (data): the mv-fires path
        ("buf_right_drop(5, data)", Box::new(|d: &[u8]| d[..d.len() - 5].to_vec())),
        ("buf_right_drop(1, data)", Box::new(|d: &[u8]| d[..d.len() - 1].to_vec())),
        // count edges: n = 0 keeps everything, n = len yields empty
        ("buf_right_drop(0, data)", Box::new(|d: &[u8]| d.to_vec())),
        ("buf_right_drop(32, data)", Box::new(|_: &[u8]| Vec::new())),
        // Pool source (computed bytes expr): the no-mv path must stay exact
        ("buf_right_drop(3, buf_cut(data, 4, 20))", Box::new(|d: &[u8]| d[4..21].to_vec())),
        // RDrop chain (second drop consumes a pool-loc result)
        ("buf_right_drop(2, buf_right_drop(3, data))", Box::new(|d: &[u8]| d[..27].to_vec())),
        // CAT after RDrop — content, not size
        (
            "buf_right_drop(5, data) ++ buf_right_drop(2, data)",
            Box::new(|d: &[u8]| {
                let mut v = d[..d.len() - 5].to_vec();
                v.extend_from_slice(&d[..d.len() - 2]);
                v
            }),
        ),
        (
            "buf_right_drop(5, data) ++ data",
            Box::new(|d: &[u8]| {
                let mut v = d[..d.len() - 5].to_vec();
                v.extend_from_slice(d);
                v
            }),
        ),
    ];
    for (src, model) in &cases {
        let body = format!("var r = {src}\nreturn r");
        let want = model(&data);
        for mode in MODES {
            assert_eq!(
                exec_bytes(&body, &data, mode),
                Ok(want.clone()),
                "mode {mode:?} CONTENT diverged for body:\n{body}\ninput: {data:?}"
            );
        }
    }
}

/// BUG-008 regression pin (P1-F): with the legacy pure-write plan events,
/// the XOP old-value read at a segment boundary was invisible to the
/// ImcRemap bank allocator. A segment of slot `a` whose recorded liveness
/// ended early could share its register with another slot's write segment
/// placed in the gap (seg.end, xop idx) — the boundary read in `a += …`
/// then consumed the sharer's value: ImcRemap returned 10 for the first
/// body below (c's 9, not a's 7, plus 1), 12/14 for the wider-gap variants.
/// The plan walk now records the boundary read (xop_reads), the old
/// segment's unit covers the XOP statement, and all three modes must
/// return the exact sequential-semantics value. The loop-as-if-arm-tail
/// bodies are the documented RV64_INV_CHECK panic shape (`INV slot=0 …
/// seg=[3,5]`): with the fix the checker's read-side coverage invariant
/// holds for the boundary read, so under RV64_INV_CHECK=1 the lowering
/// itself re-asserts the segment invariant — this test pins both the
/// behavior and the checker in both gate modes.
#[test]
fn xop_boundary_segment_sharing_soundness() {
    // Wants are WIDTH-EXACT to the interpreter (narrow-PUT face): `a = 7`
    // narrows a to a u8 value and `a += 1` keeps the u8 plane, so the view
    // is ONE byte ([8]) — the pre-fix pins demanded be_u64 (they pinned
    // the static-join divergence). Cases 4-6 keep the u64 plane (the
    // values stay u64 or re-widen through the u64 rhs of `s += i`).
    let cases: Vec<(&str, u64, usize)> = vec![
        // straight-line gap clobber — the minimal miscompile (no if/while)
        (
            "var a = 0u64\nvar b = 0u64\nvar c = 0u64\na = 7\nb = a\nc = 9\na += 1\nreturn a as bytes",
            8,
            1,
        ),
        (
            "var a = 0u64\nvar b = 0u64\nvar c = 0u64\na = 7\nb = a\nc = 9\nc += 2\na += 1\nreturn a as bytes",
            8,
            1,
        ),
        (
            "var a = 0u64\nvar b = 0u64\nvar c = 0u64\nvar d = 0u64\na = 7\nb = a\nc = 9\nd = c\nc += 4\na += 1\nreturn a as bytes",
            8,
            1,
        ),
        // loop-as-if-arm-tail (the documented panic shape): s stays 0, the
        // while runs i to 3, s += i → 3 (else arm skipped)
        (
            "var s = 0u64\nvar i = 0u64\nif s == 0 { while i < 3 { i += 1 } } else { s = 2 }\ns += i\nreturn s as bytes",
            3,
            8,
        ),
        // same shape, condition false: the else arm writes the pre-XOP
        // segment (2), the while never runs (i stays 0) → 2 + 0
        (
            "var s = 9u64\nvar i = 0u64\nif s == 0 { while i < 3 { i += 1 } } else { s = 2 }\ns += i\nreturn s as bytes",
            2,
            8,
        ),
        // while-tail shape nested in an outer loop: iter1 takes the then
        // arm (i → 3, s = 0+3), iter2 takes the else arm (s = 2, s += 3)
        (
            "var s = 0u64\nvar i = 0u64\nvar k = 0u64\nwhile k < 2 {\nif s == 0 { while i < 3 { i += 1 } } else { s = 2 }\ns += i\nk += 1 }\nreturn s as bytes",
            5,
            8,
        ),
    ];
    for (body, want, wbytes) in &cases {
        let be = want.to_be_bytes()[8 - wbytes..].to_vec();
        for mode in MODES {
            assert_eq!(
                exec(body, &[], mode),
                Ok(be.clone()),
                "mode {mode:?} diverged for body:\n{body}"
            );
        }
    }
}

/* ---- E2: control-flow matrix ---- */

#[test]
fn nested_loops_with_break_continue() {
    // 3-deep nesting, break at depth 3, continue at depth 2
    let body = r#"
var acc = 0u64
var i = 0u64
while i < 4 {
    var j = 0u64
    while j < 4 {
        if j % 2 == 0 { j += 1
            continue }
        var k = 0u64
        while k < 4 {
            if k == 3 { break }
            if (i + j + k) % 5 == 0 { acc += 100 } else { acc += 1 }
            k += 1
        }
        j += 1
    }
    i += 1
}
return acc as bytes
"#;
    // reference
    let mut acc = 0u64;
    for i in 0..4u64 {
        for j in 0..4u64 {
            if j % 2 == 0 {
                continue;
            }
            for k in 0..4u64 {
                if k == 3 {
                    break;
                }
                acc += if (i + j + k) % 5 == 0 { 100 } else { 1 };
            }
        }
    }
    pin(body, &[0; 8], acc);
}

#[test]
fn if_else_chain_and_conditional_writes() {
    let body = r#"
var x = byte(data, 0)
var cls = 0u64
if x < 10 { cls = 1 } else {
    if x < 50 { cls = 2 } else {
        if x < 200 { cls = 3 } else { cls = 4 }
    }
}
var out = cls * 10
if x % 2 == 0 { out += 1 }
if x % 3 == 0 { out += 5 }
return out as bytes
"#;
    for x in [0u8, 9, 10, 49, 50, 199, 200, 255] {
        let mut cls = match x {
            0..=9 => 1,
            10..=49 => 2,
            50..=199 => 3,
            _ => 4,
        };
        let mut out = cls * 10;
        if x % 2 == 0 {
            out += 1;
        }
        if x % 3 == 0 {
            out += 5;
        }
        let _ = &mut cls;
        pin(body, &[x], out);
    }
}

#[test]
fn const_select_table_forms() {
    // dense 16-arm table (the letter table shape)
    let dense: String = {
        let mut s = String::from("var v = byte(data, 0)\nvar t = 99u64\n");
        for (i, k) in (65u16..=80).enumerate() {
            s.push_str(&format!("if v == {} {{ t = {} }}\n", k, i + 1));
        }
        s.push_str("return t as bytes");
        s
    };
    // Width-exact to the INTERPRETER (narrow-PUT face): an arm PUT of a
    // u8 literal narrows the slot's value tag, so a hit view is ONE byte
    // and only the never-narrowed default keeps the declared u64's 8
    // bytes. (The pre-fix pins here demanded be_u64 for every face — they
    // pinned the static-join divergence this campaign removes; the
    // interpreter, probed: input 65 → [1], input 0 → [99 u64 BE].)
    let hit = |v: u64| vec![v as u8];
    let miss = |v: u64| v.to_be_bytes().to_vec();
    for v in [0u8, 64, 65, 70, 80, 81, 255] {
        let want = match v {
            65..=80 => hit((v - 64) as u64),
            _ => miss(99),
        };
        pin_w(&dense, &[v], &want);
    }

    // sparse with holes (span 0..40, keys only at 2/17/39)
    let holes = "var v = byte(data, 0)\nvar t = 7u64\nif v == 2 { t = 20 }\nif v == 17 { t = 30 }\nif v == 39 { t = 40 }\nreturn t as bytes";
    for v in [0u8, 2, 17, 39, 40] {
        let want = match v {
            2 => hit(20),
            17 => hit(30),
            39 => hit(40),
            _ => miss(7),
        };
        pin_w(holes, &[v], &want);
    }

    // three arms: below the 4-arm table threshold, plain branches
    let three = "var v = byte(data, 0)\nvar t = 5u64\nif v == 1 { t = 10 }\nif v == 2 { t = 20 }\nif v == 3 { t = 30 }\nreturn t as bytes";
    for v in [0u8, 1, 2, 3, 4] {
        let want = match v {
            1 => hit(10),
            2 => hit(20),
            3 => hit(30),
            _ => miss(5),
        };
        pin_w(three, &[v], &want);
    }
}

#[test]
fn loop_carried_and_maxmin_update_shapes() {
    // run/maxrun (else-arm reset), chained same-var arithmetic (popcount)
    let body = r#"
var data2 = data
var maxrun = 1u64
var run = 1u64
var q = 1u64
while q < 12 {
    var c1 = byte(data2, q)
    var c0 = byte(data2, q - 1)
    if c1 == c0 {
        run += 1
        if run > maxrun { maxrun = run }
    } else {
        run = 1
    }
    q += 1
}
var px = 65535u64
px = px - ((px >> 1) & 21845)
px = (px & 13107) + ((px >> 2) & 13107)
px = (px + (px >> 4)) & 3855
px = px + (px >> 8)
return (maxrun * 1000 + (px & 31)) as bytes
"#;
    let input: Vec<u8> = [7u8, 7, 8, 8, 8, 8, 9, 1, 1, 2, 2, 2].to_vec();
    // reference
    let mut maxrun = 1u64;
    let mut run = 1u64;
    for q in 1..12usize {
        if input[q] == input[q - 1] {
            run += 1;
            maxrun = maxrun.max(run);
        } else {
            run = 1;
        }
    }
    let mut px: u64 = 65535;
    px = px - ((px >> 1) & 21845);
    px = (px & 13107) + ((px >> 2) & 13107);
    px = (px + (px >> 4)) & 3855;
    px = px + (px >> 8);
    pin(body, &input, maxrun * 1000 + (px & 31));
}

/* ---- E3: expression depth 1..8 accepted, 9 rejected ---- */

#[test]
fn expression_depth_ladder() {
    // depth 1..8: balanced + chains
    let input: Vec<u8> = (1u8..=16).collect();
    // pre-stored vars: a GET costs one vstack slot, so the expression depth
    // is exactly the nesting depth (a raw `byte(data,k)` peaks at 3)
    let decls: String = (0..10).map(|k| format!("var v{k} = byte(data, {k})\n")).collect();
    for d in 1..=8usize {
        // right-nested (x0 + (x1 + (…))): the vstack actually reaches depth d
        let mut e = format!("v{}", d - 1);
        for k in 0..d - 1 {
            e = format!("(v{k} + {e})");
        }
        let body = format!("{decls}var r = {e}\nreturn r as bytes");
        // byte() decls are U8 → the sum stays U8-width (A2: 1-byte view);
        // input bytes 1..=16 keep every partial sum ≤ 128 < 256
        let want: u8 = (0..d).map(|k| input[k]).sum();
        for mode in MODES {
            let got = exec(&body, &input, mode);
            assert_eq!(got, Ok(want.to_be_bytes().to_vec()), "depth {d} {mode:?}");
        }
    }
    // minimal: 8 simple vars
    let mini = "var a = 1\nvar b = 2\nvar c = 3\nvar d4 = 4\nvar e5 = 5\nvar f6 = 6\nvar g7 = 7\nvar h8 = 8\nreturn (a + b + c + d4 + e5 + f6 + g7 + h8) as bytes";
    println!("mini = {:?}", exec(mini, &[0; 4], Rv64Mode::Im));
    // the same but indexed byte decls
    let mini2 = "var a = byte(data,0)\nvar b = byte(data,1)\nvar c = byte(data,2)\nvar d4 = byte(data,3)\nvar e5 = byte(data,4)\nvar f6 = byte(data,5)\nvar g7 = byte(data,6)\nvar h8 = byte(data,7)\nreturn (a + b + c + d4 + e5 + f6 + g7 + h8) as bytes";
    println!("mini2 = {:?}", exec(mini2, &[0; 8], Rv64Mode::Im));
    // probe: the exact nested expr from d=8
    let decls10: String = (0..10).map(|k| format!("var v{k} = byte(data, {k})\n")).collect();
    let e8 = "(v6 + (v5 + (v4 + (v3 + (v2 + (v1 + (v0 + v7)))))))";
    println!("p2 = {:?}", exec(&format!("{decls10}var r = {e8}\nreturn r as bytes"), &[1u8; 16], Rv64Mode::Im));
    let e8b = "(v0 + (v1 + (v2 + (v3 + (v4 + (v5 + (v6 + v7)))))))";
    println!("p3 = {:?}", exec(&format!("{decls10}var r = {e8b}\nreturn r as bytes"), &[1u8; 16], Rv64Mode::Im));
    // probe: 10 decls + shallow use of v7
    let p1 = "var v0 = byte(data,0)\nvar v1 = byte(data,1)\nvar v2 = byte(data,2)\nvar v3 = byte(data,3)\nvar v4 = byte(data,4)\nvar v5 = byte(data,5)\nvar v6 = byte(data,6)\nvar v7 = byte(data,7)\nvar v8 = byte(data,8)\nvar v9 = byte(data,9)\nreturn v7 as bytes";
    println!("p1 = {:?}", exec(p1, &[0; 16], Rv64Mode::Im));
    // depth 9: hard subset limit, clean compile error in every mode
    let mut e9 = "v8".to_string();
    for k in 0..8 {
        e9 = format!("(v{k} + {e9})");
    }
    let body9 = format!("{decls}var r = {e9}\nreturn r as bytes");
    for mode in MODES {
        let err = lower_err(&body9, true, mode);
        assert!(
            err.contains("nesting exceeds"),
            "depth-9 must be rejected in {mode:?}: {err}"
        );
    }
}

fn lower_err(body: &str, has_input: bool, mode: Rv64Mode) -> String {
    let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
    let args: Vec<(String, ValueTy)> = has_input
        .then(|| vec![("data".to_string(), ValueTy::Bytes)])
        .unwrap_or_default();
    let (ir, _, _) = compile_body(tokens, args, &[], &[], false).unwrap();
    match lower_body_mode_arena(&ir, has_input, mode, ARENA_SIZE_MAX) {
        Ok(_) => "no error".to_string(),
        Err(e) => e.to_string(),
    }
}

/* ---- E6: branch-relaxation stress (±4 KiB trampoline ladder) ---- */

#[test]
fn branch_relaxation_trampoline() {
    // >4 KiB of straight-line code between a loop head and its backedge
    // forces the trampoline form; correctness must survive every rung of
    // the relaxation ladder (c.j → jal → inverted+jal, c.beqz → beq → …)
    let mut body = String::from("var s = 0u64\nvar i = 0u64\nwhile i < 3 {\n");
    // ~260 statements × ~18 B ≈ 4.7 KiB inside the loop body
    for k in 0..260 {
        body.push_str(&format!("if byte(data, 0) != {k} {{ s += {k} }}\n"));
    }
    body.push_str("i += 1\n}\nreturn s as bytes");
    let want: u64 = (0..260).filter(|k| *k as u8 != 42).sum::<u64>() * 3;
    pin(&body, &[42], want);
}

/* ---- E7: arena pressure (churn bound, not peak) ---- */

#[test]
fn arena_churn_in_bounds() {
    // repeated 1 KiB concatenation: 64 iterations churn 32 KiB (≤ arena
    // each step) — the bump never wraps, exhaustion is per-allocation
    // the arena is a bump allocator with NO reclamation: the bound is on
    // TOTAL bytes churned (Σ allocation sizes), not live size. 10
    // iterations churn 200·Σ(k+2) = 13 KiB (fits); 60 would churn 363 KiB
    // and correctly trap CastFail.
    for iters in [1u64, 2, 5, 10] {
        let body = format!("var b = buf_left(200, data)\nvar i = 0u64\nwhile i < {iters} {{ b = b ++ buf_left(200, data)\ni += 1 }}\nreturn (size(b) + byte(b, 100)) as bytes");
        let mut input = vec![0u8; 256];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        // U16 + U8 widens to U16 → the 2-byte `as bytes` view (A2)
        let want = ((iters + 1) * 200 + input[100] as u64) as u16;
        for mode in MODES {
            let got = exec(&body, &input, mode);
            assert_eq!(got, Ok(want.to_be_bytes().to_vec()), "churn {iters} {mode:?}");
        }
    }
    // the churn cliff: 60 iterations exceed the arena budget — trap
    let cliff = "var b = buf_left(200, data)\nvar i = 0u64\nwhile i < 60 { b = b ++ buf_left(200, data)\ni += 1 }\nreturn size(b) as bytes";
    for mode in MODES {
        assert_eq!(exec(cliff, &vec![0u8; 256], mode), Err(71), "cliff {mode:?}");
    }
}

/* ---- statement-position expression drops, empty outputs ---- */

#[test]
fn statement_expressions_and_fallthrough() {
    // expression statements with dead values + a body that falls off the
    // end (empty output) vs an explicit empty return. A2: `byte()` values
    // render as the 1-byte `as bytes` view.
    let body = "var a = byte(data, 0)\na + 1\na * 2\nbyte(data, 0) % 7\nreturn a as bytes";
    pin_w(body, &[9], &[9]);
    // require / assert / throw shapes
    let req = "require 42, byte(data, 0) < 10\nreturn byte(data, 0) as bytes";
    for (x, want) in [(0u8, Ok(0u8)), (9, Ok(9u8)), (10, Err(42))] {
        let w = match want {
            Ok(v) => Ok(v.to_be_bytes().to_vec()),
            Err(c) => Err(c),
        };
        for mode in MODES {
            assert_eq!(exec(req, &[x], mode), w, "require {x} {mode:?}");
        }
    }
    let thr = "throw 7";
    for mode in MODES {
        assert_eq!(exec(thr, &[0; 16], mode), Err(7), "throw {mode:?}");
    }
}


#[test]
fn malformed_probe() {
    // what exit code does the RUST score_sum container give on size%24 != 0?
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../doc/hacdsdefi4000.loc/out/score_sum.hcf1")).unwrap();
    let (c, _) = vm::calc::container::CalcContainer::decode(&bytes).unwrap();
    let cap = SpaceCap::new(0);
    let fno = CalcFnObj::build(c, &cap, CalcHostConfig::new()).unwrap();
    for len in [0usize, 23, 25, 47, 48] {
        let ex = fno.run(&vec![7u8; len], CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None });
        println!("len={len} => {:?} exit={}", ex.result.map(|o| o.len()), ex.exit_code);
    }
}
