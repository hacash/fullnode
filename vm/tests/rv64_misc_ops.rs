//! Pure-compute subset extension · T1 (SQRT/SQRTUP), T9 (XLG), T11
//! (constant-index GETX/PUTX), T12 (type tests TNIL/TID/TIS/TLIST/TMAP).
//!
//! Two differential channels:
//!  * source-reachable ops (T11/T12 — `local_x`/`local_x_put`/`type_id`/
//!    `is`): the shared fitsh source through Interp vs all three lowering
//!    modes (the c3lib four-way knock);
//!  * hand-built IR (T1/T9 — SQRT/XLG have no fitsh source syntax): the
//!    same `IRNodeArray` drives the interpreter (via its own `codegen`
//!    bytecode) and the rv64 lowering, so both channels consume identical
//!    semantics by construction.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

use vm::ir::rv64::{lower_body_mode_arena, Rv64Mode, ARENA_SIZE_MAX};
use vm::ir::{IRNode, IRNodeArray, IRNodeParam1, IRNodeParam1Single, IRNodeSingle};
use vm::{encode_local_logic_mark, LxLg};
use vm::Bytecode;

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/* ================= channel 1: source differential ==================== */

/// Interp vs rv64 (3 modes): results AND fault codes must agree.
fn agree4(body: &str, input: &[u8]) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let want = it.run(input);
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
    for (mode, got) in rv.run(input) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}] input {input:?}"),
            (Err(w), Err(code)) => {
                assert_eq!(w.code as u64, *code, "{body} [{mode:?}] input {input:?}")
            }
            _ => panic!("{body} [{mode:?}] input {input:?}: interp={want:?} rv64={got:?}"),
        }
    }
}

/// agree4 + the expected u64 model (returned through `(r as u64) as bytes`).
fn agree4_model(body: &str, input: &[u8], want: u64) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    match it.run(input) {
        Ok(g) => assert_eq!(g, want.to_be_bytes().to_vec(), "{body} input {input:?}"),
        Err(e) => panic!("{body} input {input:?}: interp faulted {e:?}, wanted {want}"),
    }
    agree4(body, input);
}

/* ================= channel 2: hand-built IR differential ============= */

/// The `var r = <expr>` source of a compiled template, swapped for a
/// hand-built node — both channels then run the SAME IR tree.
fn swap_ir(repl: Box<dyn IRNode>, tpl: &str, r_slot: u8) -> IRNodeArray {
    let (mut ir, _) = c3lib::frontend(tpl).expect("template frontend");
    let mut hit = false;
    for i in 0..ir.subs.len() {
        if let Some(p) = ir.subs[i].as_any().downcast_ref::<IRNodeParam1Single>() {
            if p.inst == Bytecode::PUT && p.para == r_slot {
                let np =
                    IRNodeParam1Single { hrtv: p.hrtv, inst: p.inst, para: p.para, subx: repl };
                ir.subs[i] = Box::new(np);
                hit = true;
                break;
            }
        }
    }
    assert!(hit, "template PUT {r_slot} not found");
    ir
}

fn run_ir_interp(ir: &IRNodeArray, input: &[u8]) -> Result<Vec<u8>, c3lib::InterpFault> {
    let codes = ir.codegen().expect("ir codegen");
    Interp::from_body_codes(codes).run(input)
}

fn run_ir_rv64(
    ir: &IRNodeArray,
    input: &[u8],
) -> Vec<(Rv64Mode, Result<Vec<u8>, u64>)> {
    let cap = vm::calc::SpaceCap::new(0);
    let mut out = Vec::new();
    for &mode in MODES.iter() {
        let lo = lower_body_mode_arena(ir, true, mode, ARENA_SIZE_MAX)
            .unwrap_or_else(|e| panic!("rv64 lower {mode:?}: {e}"));
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let c = lo
            .pack_segments_profile(&cap, 64, 4800, 16, profile)
            .expect("pack");
        let fno =
            vm::calc::CalcFnObj::build(c, &cap, vm::calc::CalcHostConfig::new()).expect("build");
        let ex = fno.run(
            input,
            vm::calc::CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
        );
        out.push((mode, ex.result.map_err(|_| ex.exit_code)));
    }
    out
}

fn agree_ir(ir: &IRNodeArray, input: &[u8], label: &str) {
    let want = run_ir_interp(ir, input);
    for (mode, got) in run_ir_rv64(ir, input) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{label} [{mode:?}] input {input:?}"),
            (Err(w), Err(code)) => {
                assert_eq!(w.code as u64, *code, "{label} [{mode:?}] input {input:?}")
            }
            _ => panic!("{label} [{mode:?}] input {input:?}: interp={want:?} rv64={got:?}"),
        }
    }
}

fn get_node(para: u8) -> Box<dyn IRNode> {
    Box::new(IRNodeParam1 {
        hrtv: true,
        inst: Bytecode::GET,
        para,
        text: String::new(),
    })
}

fn u64_word(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/* ================= T12: the type-test family ========================= */

#[test]
fn t12_type_id_scalar_widths() {
    // U8..U128 → ids 2..6, Bool → 1, Bytes (the input param) → 8
    agree4_model(
        "var a = (byte(data, 0) as u8)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        2,
    );
    agree4_model(
        "var a = (byte(data, 0) as u16)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        3,
    );
    agree4_model(
        "var a = (byte(data, 0) as u32)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        4,
    );
    agree4_model(
        "var a = (byte(data, 0) as u64)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        5,
    );
    agree4_model(
        "var a = (byte(data, 0) as u128)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        6,
    );
    agree4_model(
        "var r = type_id(data)\nreturn (r as u64) as bytes",
        &[255],
        8,
    );
    agree4_model(
        "var a = (byte(data, 0) < 1u64)\nvar r = type_id(a)\nreturn (r as u64) as bytes",
        &[255],
        1,
    );
}

#[test]
fn t12_type_is_and_predicates() {
    let pre = "var a = (byte(data, 0) as u64)\n";
    agree4_model(&format!("{pre}var r = a is nil\nreturn (r as u64) as bytes"), &[7], 0);
    agree4_model(&format!("{pre}var r = a is u64\nreturn (r as u64) as bytes"), &[7], 1);
    agree4_model(&format!("{pre}var r = a is u8\nreturn (r as u64) as bytes"), &[7], 0);
    agree4_model(&format!("{pre}var r = a is bytes\nreturn (r as u64) as bytes"), &[7], 0);
    agree4_model(&format!("{pre}var r = a is list\nreturn (r as u64) as bytes"), &[7], 0);
    agree4_model(&format!("{pre}var r = a is map\nreturn (r as u64) as bytes"), &[7], 0);
    agree4_model("var r = data is bytes\nreturn (r as u64) as bytes", &[7], 1);
    agree4_model("var r = data is u64\nreturn (r as u64) as bytes", &[7], 0);
    agree4_model(
        "var a = (byte(data, 0) < 1u64)\nvar r = a is bool\nreturn (r as u64) as bytes",
        &[7],
        1,
    );
    agree4_model(
        "var a = (byte(data, 0) < 1u64)\nvar r = a is u64\nreturn (r as u64) as bytes",
        &[7],
        0,
    );
    // fault face: the operand evaluates first (div by zero → 90 on both)
    agree4(
        "var a = 0u64\nvar r = ((1u64 / a) is nil)\nreturn (r as u64) as bytes",
        &[0],
    );
}

#[test]
fn t12_tnil_reads_the_init_flag() {
    // `var u` inside the if is slot 1 — allocated but written only on the
    // taken branch: `local_x(1) is nil` reads the init flag both channels
    let tpl = "if byte(data, 0) == 1u64 { var u = 5u64 }\nvar r = (local_x(1) is nil)\nreturn (r as u64) as bytes";
    agree4_model(tpl, &[1], 0); // written → not nil
    agree4_model(tpl, &[2], 1); // skipped → nil
    let tpl2 = "if byte(data, 0) == 1u64 { var u = 5u64 }\nvar r = type_id(local_x(1))\nreturn (r as u64) as bytes";
    agree4_model(tpl2, &[1], 5); // u64 id
    agree4_model(tpl2, &[2], 0); // nil id
    let tpl3 = "if byte(data, 0) == 1u64 { var u = 5u64 }\nvar r = (local_x(1) is u64)\nreturn (r as u64) as bytes";
    agree4_model(tpl3, &[1], 1);
    agree4_model(tpl3, &[2], 0);
}

#[test]
fn t12_may_nil_wide_and_bytes_operands() {
    // u128 slot: the tag rides the LOW entry of the pair (2 vstack entries)
    let tpl = "if byte(data, 0) == 1u64 { var u = 7u128 }\nvar r = (local_x(1) is nil)\nreturn (r as u64) as bytes";
    agree4_model(tpl, &[1], 0);
    agree4_model(tpl, &[2], 1);
    let tpl2 = "if byte(data, 0) == 1u64 { var u = 7u128 }\nvar r = type_id(local_x(1))\nreturn (r as u64) as bytes";
    agree4_model(tpl2, &[1], 6); // u128 id
    agree4_model(tpl2, &[2], 0); // nil id
    // bytes slot: the tag rides the ptr entry (2 vstack entries)
    let tpl3 = "if byte(data, 0) == 1u64 { var s = buf_left(2, data) }\nvar r = (local_x(1) is nil)\nreturn (r as u64) as bytes";
    agree4_model(tpl3, &[1, 9, 9], 0);
    agree4_model(tpl3, &[2, 9, 9], 1);
    let tpl4 = "if byte(data, 0) == 1u64 { var s = buf_left(2, data) }\nvar r = type_id(local_x(1))\nreturn (r as u64) as bytes";
    agree4_model(tpl4, &[1, 9, 9], 8); // bytes id
    agree4_model(tpl4, &[2, 9, 9], 0); // nil id
    // the input param is never Nil
    agree4_model("var r = (data is nil)\nreturn (r as u64) as bytes", &[9], 0);
}

/* ================= T11: constant-index GETX/PUTX ===================== */

#[test]
fn t11_local_x_get_put_roundtrip() {
    agree4_model(
        "var a = 0u64\nlocal_x_put(1, 42u64)\nvar r = local_x(1)\nreturn (r as u64) as bytes",
        &[],
        42,
    );
    agree4_model(
        "var a = 7u64\nvar r = local_x(1)\nreturn (r as u64) as bytes",
        &[],
        7,
    );
    // narrow-width store through the folded PUT keeps its width face
    agree4_model(
        "var a = 0u64\nvar b = 0u64\nlocal_x_put(2, 300u16)\nvar r = (local_x(2) as u16)\nreturn (r as u64) as bytes",
        &[],
        300,
    );
    // name and index views alias the same slot
    agree4_model(
        "var a = 0u64\nvar b = 0u64\nlocal_x_put(1, 9u64)\nvar r = (a + b)\nreturn (r as u64) as bytes",
        &[],
        9,
    );
    // u128 slots through the fold (16-byte return; the interpreter is the
    // model — u128::MAX round-trips through the folded PUT/GET pair)
    agree4(
        "var a = 0u128\nlocal_x_put(1, 340282366920938463463374607431768211455u128)\nvar r = local_x(1)\nreturn (r as u128) as bytes",
        &[],
    );
    // reads participate in the loop machinery
    agree4(
        "var a = 3u64\nvar i = 0u64\nvar s = 0u64\nwhile i < 4 { s += local_x(1)\ni += 1 }\nvar r = local_x(3)\nreturn (r as u64) as bytes",
        &[],
    );
}

#[test]
fn t11_local_x_reject_faces() {
    // dynamic index: outside the statically addressed kernel frame
    let e = match Rv64::try_build("var i = 1u64\nvar a = 7u64\nvar r = local_x(i)\nreturn (r as u64) as bytes") {
        Err(e) => e,
        Ok(_) => panic!("dynamic index must reject"),
    };
    assert!(e.contains("GETX") && e.contains("dynamic local index"), "{e}");
    // index beyond the declared local frame (interpreter faults 42)
    let e = match Rv64::try_build("var a = 7u64\nvar r = local_x(9)\nreturn (r as u64) as bytes") {
        Err(e) => e,
        Ok(_) => panic!("out-of-frame index must reject"),
    };
    assert!(e.contains("local frame") && e.contains("42"), "{e}");
    // literal beyond u16 (interpreter's extract_u16 → CastParamFail 72)
    let e = match Rv64::try_build("var a = 7u64\nvar r = local_x(70000)\nreturn (r as u64) as bytes") {
        Err(e) => e,
        Ok(_) => panic!("u16-overflow index must reject"),
    };
    assert!(e.contains("72"), "{e}");
    // PUTX into the read-only input slot
    let e = match Rv64::try_build("local_x_put(0, 1u64)\nreturn 0u64 as bytes") {
        Err(e) => e,
        Ok(_) => panic!("input-slot PUTX must reject"),
    };
    assert!(e.contains("input parameter cannot be assigned"), "{e}");
}

/* ================= T1: SQRT / SQRTUP ================================== */

/// `var a` = the first 8 input bytes as u64 (slot 1), `var r` (slot 2).
const TPL_U64: &str = "var a = 0u64\nvar i = 0u64\nwhile i < 8 { a = a * 256 + byte(data, i)\ni += 1 }\nvar r = 0u64\nreturn (r as u64) as bytes";

#[test]
fn t1_sqrt_u64_matrix() {
    let vals: Vec<u64> = vec![
        0,
        1,
        2,
        3,
        4,
        15,
        16,
        17,
        255,
        256,
        65535,
        65536,
        1_000_003,
        1_000_000_000_000,
        u64::from_be_bytes([0xFF; 8]),         // 2^64-1: floor = 2^32-1
        u64::from_be_bytes([0xFF; 8]) - 1,     // not a perfect square
        4_611_686_018_427_387_904,             // 2^62 (floor 2^31)
        4_611_686_018_427_387_903,
    ];
    for &v in &vals {
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_ir(
                Box::new(IRNodeSingle { hrtv: true, inst, subx: get_node(1) }),
                TPL_U64,
                2,
            );
            agree_ir(&ir, &u64_word(v), &format!("sqrt({v})"));
        }
    }
}

#[test]
fn t1_sqrt_narrow_widths_and_faults() {
    // u8 operand: isqrt(255)=15 (the narrow result keeps the input width)
    let ir = swap_ir(
        Box::new(IRNodeSingle {
            hrtv: true,
            inst: Bytecode::SQRT,
            subx: Box::new(IRNodeSingle {
                hrtv: true,
                inst: Bytecode::CU8,
                subx: get_node(1),
            }),
        }),
        TPL_U64,
        2,
    );
    agree_ir(&ir, &u64_word(255), "sqrt(cu8)");
    agree_ir(&ir, &u64_word(0), "sqrt(cu8, 0)");
    // operand fault order: div-by-zero inside the sqrt operand → 90 both
    let ir = swap_ir(
        Box::new(IRNodeSingle {
            hrtv: true,
            inst: Bytecode::SQRT,
            subx: Box::new(vm::ir::IRNodeDouble {
                hrtv: true,
                inst: Bytecode::DIV,
                subx: Box::new(vm::ir::IRNodeLeaf {
                    hrtv: true,
                    inst: Bytecode::P1,
                    text: String::new(),
                }),
                suby: get_node(1),
            }),
        }),
        TPL_U64,
        2,
    );
    agree_ir(&ir, &u64_word(0), "sqrt(1/0) → 90");
    agree_ir(&ir, &u64_word(2), "sqrt(1/2)");
}

#[test]
fn t1_sqrt_u128_pair() {
    /// `var a` = the first 16 input bytes as u128 (slot 1), r slot 2.
    let tpl = "var a = 0u128\nvar i = 0u64\nwhile i < 16 { a = a * 256u128 + (byte(data, i) as u128)\ni += 1 }\nvar r = 0u128\nreturn (r as u128) as bytes";
    let mk = |hi: u64, lo: u64| {
        let mut v = hi.to_be_bytes().to_vec();
        v.extend(lo.to_be_bytes());
        v
    };
    let cases: Vec<(u64, u64)> = vec![
        (0, 0),
        (0, 1),
        (0, 4),
        (0, u64::MAX),
        (1, 0),                    // 2^64 (sqrt = 2^32)
        (0, 1 << 63),
        (u64::MAX, u64::MAX),      // 2^128-1 (floor = 2^64-1)
        (u64::MAX - 1, u64::MAX),
        (4, 0),                    // 2^66
    ];
    for &(hi, lo) in &cases {
        for &inst in &[Bytecode::SQRT, Bytecode::SQRTUP] {
            let ir = swap_ir(
                Box::new(IRNodeSingle { hrtv: true, inst, subx: get_node(1) }),
                tpl,
                3,
            );
            agree_ir(&ir, &mk(hi, lo), &format!("sqrt128({hi}:{lo})"));
        }
    }
}

/* ================= T9: XLG =========================================== */

/// `var a` = byte0, `var b` = byte1 (u64 slots 1/2), `var r` slot 3.
const TPL_AB: &str = "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = (a + b)\nreturn (r as u64) as bytes";

fn xlg_node(lg: LxLg, idx: u8, rhs: Box<dyn IRNode>) -> Box<dyn IRNode> {
    let mark = encode_local_logic_mark(lg, idx).unwrap();
    Box::new(IRNodeParam1Single { hrtv: true, inst: Bytecode::XLG, para: mark, subx: rhs })
}

#[test]
fn t9_xlg_all_ops_u64() {
    let pairs: Vec<(u64, u64)> =
        vec![(0, 0), (0, 1), (1, 0), (5, 9), (9, 5), (5, 5), (255, 254), (1, 255)];
    let ops = [
        LxLg::And,
        LxLg::Or,
        LxLg::Eq,
        LxLg::Ne,
        LxLg::Gt,
        LxLg::Ge,
        LxLg::Lt,
        LxLg::Le,
    ];
    for &(a, b) in &pairs {
        for &lg in &ops {
            let ir = swap_ir(xlg_node(lg, 1, get_node(2)), TPL_AB, 3);
            agree_ir(&ir, &[a as u8, b as u8], &format!("xlg {lg:?}({a},{b})"));
        }
    }
}

#[test]
fn t9_xlg_may_nil_lhs() {
    // may-Nil lhs (slot 1 written only when data[0]==1); rhs = slot 2 (b)
    let tpl = "if byte(data, 0) == 1u64 { var u = 7u64 }\nvar b = (byte(data, 1) as u64)\nvar r = 0u64\nreturn (r as u64) as bytes";
    // Eq: taken → eq(7, b); skipped → Nil vs uint → 90
    let ir = swap_ir(xlg_node(LxLg::Eq, 1, get_node(2)), tpl, 3);
    agree_ir(&ir, &[1, 7], "xlg eq taken(=1)");
    agree_ir(&ir, &[0, 7], "xlg eq nil-lhs → 90");
    // ordered: skipped → Nil is not uint → 90
    let ir = swap_ir(xlg_node(LxLg::Le, 1, get_node(2)), tpl, 3);
    agree_ir(&ir, &[1, 9], "xlg le taken");
    agree_ir(&ir, &[0, 9], "xlg le nil-lhs → 90");
    // And/Or with a may-Nil local: Nil → false (truthiness, no fault)
    let ir = swap_ir(xlg_node(LxLg::And, 1, get_node(2)), tpl, 3);
    agree_ir(&ir, &[1, 7], "xlg and taken");
    agree_ir(&ir, &[0, 7], "xlg and nil-lhs → false");
    // BOTH sides may-Nil (rhs = local_x of the same conditional slot):
    // Nil x Nil compares equal (no fault), u64 x u64 numeric
    let getx1 = Box::new(IRNodeSingle {
        hrtv: true,
        inst: Bytecode::GETX,
        subx: Box::new(vm::ir::IRNodeLeaf {
            hrtv: true,
            inst: Bytecode::P1,
            text: String::new(),
        }),
    }) as Box<dyn IRNode>;
    let ir = swap_ir(xlg_node(LxLg::Eq, 1, getx1), tpl, 3);
    agree_ir(&ir, &[1, 7], "xlg eq both set → true");
    agree_ir(&ir, &[0, 7], "xlg eq both nil → true");
    // may-Nil u128 rhs (tag on the low pair entry) against a set u128 lhs
    let tpl128 = "if byte(data, 0) == 1u64 { var u = 7u128 }\nvar b = 7u128\nvar r = 0u64\nreturn (r as u64) as bytes";
    let ir = swap_ir(xlg_node(LxLg::Eq, 2, get_node(1)), tpl128, 3);
    agree_ir(&ir, &[1], "xlg eq u128 nil-rhs → 90");
    let ir = swap_ir(xlg_node(LxLg::Gt, 2, get_node(1)), tpl128, 3);
    agree_ir(&ir, &[1], "xlg gt u128 nil-rhs → 90");
    let ir = swap_ir(xlg_node(LxLg::Gt, 1, get_node(2)), tpl128, 3);
    agree_ir(&ir, &[0], "xlg gt u128 nil-lhs → 90");
}

#[test]
fn t9_xlg_u128_and_mixed_widths() {
    // u128 lhs (slot 1) vs its u128 copy (slot 3); r = slot 4
    let tpl = "var a = 0u128\nvar i = 0u64\nwhile i < 16 { a = a * 256u128 + (byte(data, i) as u128)\ni += 1 }\nvar b = a\nvar r = 0u128\nreturn (r as u128) as bytes";
    let mk = |v: u128| {
        let mut out = Vec::new();
        out.extend(v.to_be_bytes());
        out
    };
    for &(v, lg) in &[
        (0u128, LxLg::Eq),
        (u128::MAX, LxLg::Ne),
        (1u128 << 100, LxLg::Ge),
        (1u128 << 100, LxLg::Lt),
    ] {
        let ir = swap_ir(xlg_node(lg, 1, get_node(3)), tpl, 4);
        agree_ir(&ir, &mk(v), &format!("xlg128 {lg:?}({v})"));
    }
    // mixed widths: u64 rhs against a u128 local (numeric compare)
    let tpl2 = "var a = 0u128\nvar i = 0u64\nwhile i < 16 { a = a * 256u128 + (byte(data, i) as u128)\ni += 1 }\nvar b = 0u64\nvar r = 0u128\nreturn (r as u128) as bytes";
    let ir = swap_ir(xlg_node(LxLg::Eq, 1, get_node(3)), tpl2, 4);
    let mut inp = vec![0u8; 15];
    inp.push(7); // a = 7 (u128), b = 0
    agree_ir(&ir, &inp, "xlg eq u128 lhs vs u64 rhs");
}

#[test]
fn t9_xlg_reject_faces() {
    // bool-ish lhs (a comparison slot): ordered compare is uint-only
    // (the interpreter faults 90 at run time — fail fast at compile)
    let tpl = "var a = (byte(data, 0) < 1u64)\nvar b = (byte(data, 1) as u64)\nvar r = 0u64\nreturn (r as u64) as bytes";
    let ir = swap_ir(xlg_node(LxLg::Lt, 1, get_node(2)), tpl, 3);
    let e = lower_body_mode_arena(&ir, true, Rv64Mode::Im, ARENA_SIZE_MAX)
        .expect_err("bool lhs ordered XLG must reject");
    assert!(e.contains("XLG") && e.contains("uint operands"), "{e}");
    // bool vs uint Eq: value_content_eq faults 90 (mixed types)
    let ir = swap_ir(xlg_node(LxLg::Eq, 1, get_node(2)), tpl, 3);
    let e = lower_body_mode_arena(&ir, true, Rv64Mode::Im, ARENA_SIZE_MAX)
        .expect_err("bool/uint XLG eq must reject");
    assert!(e.contains("XLG") && e.contains("Arithmetic(90)"), "{e}");
    // bytes rhs (data): truthiness/content-eq both unsupported → reject
    let tpl2 = "var a = (byte(data, 0) as u64)\nvar r = 0u64\nreturn (r as u64) as bytes";
    let ir = swap_ir(xlg_node(LxLg::And, 1, get_node(0)), tpl2, 2);
    let e = lower_body_mode_arena(&ir, true, Rv64Mode::Im, ARENA_SIZE_MAX)
        .expect_err("bytes-rhs XLG must reject");
    assert!(e.contains("XLG"), "{e}");
}
