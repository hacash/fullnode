//! C3: coverage denominator + instrumentation + saturation proof (G2.3/G2.4).
//!
//! P2-C3 deliverable (docs.loc/rv64im/71-c3-coverage-denominator.md). This
//! file turns "coverage" from a vibe into an explicit, enumerated, pinned
//! denominator and a reproducible hit-measurement test:
//!
//! * `DENOM` — the denominator as an explicit item table across SIX
//!   dimensions (each with a file:line-sourced universe):
//!     ① IR opcodes of the pinned translatable subset (rv64.rs:82-96 —
//!        77 opcodes) × ② the three lowering modes {Im, Imc, ImcRemap}
//!        (rv64.rs:231-233) → `op:<OP>:<MODE>` items;
//!     ③ guard/trap classes (interpreter fault sites, 30-c1 E2 + P2-F2
//!        doc 13 §6.1 consumer table) → `g:<class>` items;
//!     ④ branch-relaxation & materialization tiers (rv64.rs FixKind
//!        ladder + literal pool) → `t:<tier>:<MODE>` items;
//!     ⑤ backend emit-site classes observable via `LowerDbg`
//!        (lower_body_mode_arena_dbg, rv64.rs:2031) and/or consensus
//!        decode (calc::decode::decode_at with the mode profile) →
//!        `s:<site>` items;
//!     ⑥ error paths: frontend-reject family + rv64-compile-reject
//!        family + runtime budget → `fe:`/`rx:`/`rt:` items.
//!   The total item count is explicit (`DENOM_LEN`, pinned by test).
//!
//! * Hit measurement is PURE test-side instrumentation: IR-tree walk (the
//!   rv64.rs `children` downcast pattern), one extra `lower_body_mode_
//!   arena_dbg` twin lowering per mode (byte-identical to production —
//!   pinned by rv64.rs `dbg_engine_matches_production`), decode of the
//!   production-lowered stream, and knock outcomes through the C2
//!   classifier. Zero production code is touched; no cfg(test) counters
//!   were needed inside vm/src.
//!
//! * Saturation (G2.4): `c3_saturation_feedback` iterates deterministic
//!   seed ranges (plateau-detected), then fills gaps with TARGETED probes
//!   (static per-item bodies — the "directed generation" for unhit items;
//!   no randomness enters any artifact), and finally asserts
//!   hits ∪ unreachable-with-argument == DENOM. "100% coverage" is exactly
//!   that equality, reproducible from this file.
//!
//! * New engine findings from the B1 unfreeze probe (2026-11-14), FIXED
//!   2026-11-15 by P2-C3FIX (each case = one rv64.rs commit, pin flipped):
//!   BUG-017 — the EAGER CHOOSE selection (D9-fix arm, rv64.rs CHOOSE => …)
//!   popped arm values top-first WITHOUT re-reversing pairs, so a
//!   BYTES×BYTES choose/ternary copied (len, ptr) into (ptr, len)
//!   positions (interp returned the bytes, all three rv64 modes trapped
//!   71); same root as BUG-011 (which had fixed the IRIFR join only).
//!   Fixed by take_arm_values + an up-front arm-type reject for the
//!   mixed-order facets (bytes-first used to index-underflow PANIC).
//!   Pinned: `c3_pin_bug017_choose_bytes_pair_swap`. CHOOSE bytes arms
//!   stay out of the generator face (constraint ⑥ re-frozen pending soak).

#![allow(dead_code)]

#[path = "c3lib/mod.rs"]
pub mod c3lib;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use c3lib::*;
use vm::calc::decode::{decode_at, Inst};
use vm::ir::rv64::{
    lower_body_mode_arena, lower_body_mode_arena_dbg, BANK_KIND_BYTES, BANK_KIND_INT,
    ARENA_SIZE_MAX, Rv64Mode,
};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

fn mode_name(m: Rv64Mode) -> &'static str {
    match m {
        Rv64Mode::Im => "Im",
        Rv64Mode::Imc => "Imc",
        Rv64Mode::ImcRemap => "ImcRemap",
    }
}

fn mode_profile(m: Rv64Mode) -> u8 {
    match m {
        Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
        _ => vm::calc::container::CALC_PROFILE_RV64IMC,
    }
}

/* ================================================================== */
/* Section 1: the denominator — explicit six-dimension enumeration.    */
/* ================================================================== */

/// The 76-opcode pinned translatable subset: verbatim from the rv64.rs
/// module contract (vm/src/ir/rv64.rs:84-92, "Pinned translatable subset")
/// PLUS `ROLL0` — the data-parameter prelude node (`PUT 0 (ROLL0)`,
/// context.rs build_param_prelude; rv64.rs:78-80 documents the binding is
/// consumed by the lowerer). ROLL0 appears in the IR of every body with an
/// input parameter, so it belongs to the executable-subset universe.
const SUBSET_OPS: [&str; 76] = [
    // literals/casts (16)
    "P0", "P1", "P2", "P3", "PU8", "PU16", "PBUF", "PBUFL", "PNBUF", "PTRUE", "PFALSE",
    "CU8", "CU16", "CU32", "CU64", "CBYTES",
    // ints (28)
    "ADD", "SUB", "MUL", "DIV", "MOD", "POW", "MAX", "MIN", "CLAMP", "ABSDIFF", "DIVUP",
    "DIVEXACT", "BSHL", "BSHR", "BXOR", "BOR", "BAND", "EQ", "NEQ", "LT", "GT", "LE", "GE",
    "NOT", "AND", "OR", "CHOOSE",
    // bytes (8)
    "CAT", "BYTE", "CUT", "LEFT", "RIGHT", "LDROP", "RDROP", "SIZE",
    // locals (7)
    "GET", "GET0", "GET1", "GET2", "GET3", "PUT", "XOP",
    // control (18) — the frame prelude ALLOC/POP included (ALLOC is an IR
    // node; POP is bytecodegen-only, see UNREACHABLE)
    "IRLIST", "IRBLOCK", "IRBLOCKR", "IRIF", "IRIFR", "IRWHILE", "IRBREAK", "IRCONTINUE",
    "REQ", "AST", "ERR", "ABT", "RET", "END", "NOP", "ALLOC", "POP",
    // data-parameter prelude (consumed by the lowerer, rv64.rs:78-80)
    "ROLL0",
];

/// ③ guard/trap classes (the interpreter's consensus fault sites — C1 E2
/// table + P2-F2 doc 13 §6.1 consumer table + the pinned model-divergence
/// rows). One targeted probe per class (Section 4).
const GUARD_CLASSES: [&str; 38] = [
    "g:add-ovf-90",
    "g:sub-ovf-90",
    "g:mul-ovf-90",
    "g:div-zero-90",
    "g:mod-zero-90",
    "g:divup-zero-90",
    "g:divexact-zero-90",
    "g:divexact-inexact-90",
    "g:pow-ovf-90",
    "g:clamp-lohi-90",
    "g:cast-u8-71",
    "g:cast-u16-71",
    "g:cast-u32-71",
    "g:byte-oob-d2",
    "g:slice-cut-oob-d2",
    "g:slice-drop-oob-d2",
    "g:arena-churn-d4",
    "g:bytes-nil-91",
    "g:int-nil-arith-71",
    "g:nil-cmp-eq-90",
    "g:nil-cmp-ord-90",
    "g:nil-xop-old-71",
    "g:nil-cbytes-71",
    "g:nil-param-72",
    "g:nil-reqvalue-76",
    "g:require-user-150",
    "g:throw-user-151",
    "g:assert-abort-d3",
    "g:shift-mask-d1",
    "g:bool-uint-d5",
    "g:narrow-width-d5",
    "g:and-chain-pure",
    "g:mul-merge-reloc",
    // P3-V2IMPL: the compo-list trap family (interpreter fault sites ↔
    // the runtime len/index/cap guards; list.rs / convert.rs / compo.rs)
    "g:list-oob-85",
    "g:list-idx-72",
    "g:list-cap-40",
    // P3-PAIRPKG: the pair's own guard faces (direct-attribution probes;
    // the 85/72 classes are shared with the list/byte/Nil families, the
    // probe bodies carry exactly one fault site each)
    "g:pair-oob-85",
    "g:pair-idx-72",
];

/// ④ relaxation/materialization tiers × mode. c.* forms exist only in the
/// RVC profiles (Imc/ImcRemap); the ladder and pool forms exist in all.
const TIER_ITEMS: &[(&str, &[&str])] = &[
    ("t:branch4", &["Im", "Imc", "ImcRemap"]),
    ("t:jal", &["Im", "Imc", "ImcRemap"]),
    ("t:trampoline", &["Im", "Imc", "ImcRemap"]),
    ("t:li", &["Im", "Imc", "ImcRemap"]),
    ("t:poolld", &["Im", "Imc", "ImcRemap"]),
    ("t:pooloff", &["Im", "Imc", "ImcRemap"]),
    ("t:pcrhi", &["Im", "Imc", "ImcRemap"]),
    ("t:cbranch2", &["Imc", "ImcRemap"]),
    ("t:cjump2", &["Imc", "ImcRemap"]),
    ("t:cldsd", &["Imc", "ImcRemap"]),
];

/// ⑤ emit-site classes (LowerDbg / listing / IR-presence observable).
/// Mode-suffixed where per-mode; bare ids are mode-triple assertions.
const SITE_ITEMS: &[&str] = &[
    "s:bank-int:ImcRemap",
    "s:bank-bytes:ImcRemap",
    "s:frame-slot:Im",
    "s:frame-slot:Imc",
    "s:frame-slot:ImcRemap",
    "s:pool-active:Im",
    "s:pool-active:Imc",
    "s:pool-active:ImcRemap",
    "s:ref-mv:ImcRemap",
    "s:join-u64:Im",
    "s:join-u64:Imc",
    "s:join-u64:ImcRemap",
    "s:join-bytes:Im",
    "s:join-bytes:Imc",
    "s:join-bytes:ImcRemap",
    "s:choose-eager-int:Im",
    "s:choose-eager-int:Imc",
    "s:choose-eager-int:ImcRemap",
    // BUG-017 pinned: the eager-CHOOSE bytes-arm selection EMITS but
    // miscompiles; the item counts as hit via the pinned probe with its
    // divergence recorded (not an agreement).
    "s:choose-eager-bytes:Im",
    "s:choose-eager-bytes:Imc",
    "s:choose-eager-bytes:ImcRemap",
    "s:xop-bank:ImcRemap",
    "s:xop-frame:Im",
    "s:xop-frame:Imc",
    "s:xop-frame:ImcRemap",
    // nil_plan consumer arms (guard classes g:nil-* prove them at runtime;
    // these items pin the EMIT side per consumer family, mode-triple)
    "s:nil-arm-eqneq",
    "s:nil-arm-ord",
    "s:nil-arm-arith",
    "s:nil-arm-bytes",
    "s:nil-arm-cbytes",
    "s:nil-arm-xopold",
    "s:nil-arm-param72",
    "s:nil-arm-req76",
    // shared fault thunks by exit code (listing: li a0,N … ecall)
    "s:thunk-90",
    "s:thunk-71",
    "s:thunk-91",
    "s:thunk-72",
    "s:thunk-76",
    "s:thunk-user",
    // wide-output copy path (output_max 64 envelope, bigout family)
    "s:output-copy-wide",
    // prologue zero-fill of uninit slots (needs_zero) — proven at runtime
    // by the tolerant-path agreement probes (SIZE/AND on uninit slots)
    "s:needs-zero",
    // A-19 dominated-mul-check merge applied (LowerDbg.mul_merges ≠ ∅)
    "s:mul-merge-applied",
    // P3-V2IMPL: list emit sites (IR-presence keyed; a growable mutation
    // forces the 1024B frame block + len shadow, an all-literal
    // never-mutated table lives in the rodata pool)
    "s:list-frame-block",
    "s:list-rodata",
    "s:len-shadow",
    // P3-IDIOMS M3-a: fused byte-assembly groups (constant-offset byte
    // idiom → single wide load). The wide-load emitters (emit_fused_le /
    // emit_fused_be) are the ONLY lhu (f3=5) producers in the backend, so
    // a decoded lhu pins the fusion site unambiguously (lwu is shared with
    // u32 rodata tables, ld with slot reads). Detector = listing decode.
    "s:byte-fuse-wide",
    // P3-PAIRPKG: the fused remove+insert pair (IR-presence keyed: the
    // fused form emits no uniquely-decodable instruction — its sd/addi/li
    // forms are shared with the append/itemget/const idioms — so the
    // detector mirrors the plan() recognizer's adjacency core)
    "s:pair-fuse-sd",
];

/// ⑥ error paths: frontend rejects (fe:), rv64 compile rejects (rx:),
/// runtime budget (rt:). Sources: c2_negative_frontend_rejects (17 incl.
/// the C3 u64-literal-overflow addition), c2_negative_rv64_compile_rejects
/// (task 3/B3 shrank the u128 family; T5/T6 removed the last four u128 rows
/// — the mixed-width folds and the u128 shift now lower and agree in
/// rv64_u128_arith.rs), budget row.
const FRONTEND_ERR_ITEMS: [&str; 17] = [
    "fe:truncated-var",
    "fe:var-name-integer",
    "fe:unclosed-brace",
    "fe:as-missing-type",
    "fe:require-no-cond",
    "fe:prefix-operator",
    "fe:bind-no-name",
    "fe:break-outside-loop",
    "fe:continue-outside-loop",
    "fe:literal-overflows-u8",
    "fe:literal-overflows-u64",
    "fe:bytecode-unknown-op",
    "fe:cast-to-src-form",
    "fe:unknown-function",
    "fe:if-expr-missing-else",
    "fe:macro-unknown-invocation",
    "fe:macro-unterminated-invocation",
];

// T12 (pure-compute subset extension): rx:type-id left the negative
// channel — type_id/is now lower and agree (vm/tests/rv64_misc_ops.rs)
const RV64_ERR_ITEMS: [&str; 9] = [
    "rx:depth-9",
    // task 3/B3: the four u128 agreement faces (cast / nested-arith /
    // compare / literal-suffix) left the negative channel; T2 removed
    // rx:mul-mod / rx:add-mod (add_mod/mul_mod lower at every width up to
    // 128, rv64_modarith.rs); T5/T6 removed the mixed-width folds and the
    // u128 shift — all lower and agree now (rv64_u128_arith.rs)
    // T3/T4 removed rx:mul-add / rx:mul-div — mul_add/mul_sub lower at every
    // width through the u256 library (rv64_rowc.rs) and mul_div/mul_div_up
    // lower at the u64/u128 widths too (rv64_muldiv.rs)
    "rx:log-op",
    "rx:nil-value",
    // P3-V2IMPL (A-25 M1): in-shape list literals lower now. T17 flipped
    // the whole escape family — rx:list-literal-alias / -rebind / -nested
    // / -eq / -ordered-cmp / -cast / -xop / -choose-arm all compile now
    // (their differentials: rv64_t17_list_values.rs; removed above, so the
    // rv64 err-item count drops 17 → 9 and DENOM_LEN follows).
    "rx:list-literal-return",
    "rx:list-itemget-bytes-receiver",
    "rx:bytecode-splice",
    "rx:choose-mixed-arm",
    // T13: the deliberately kept CTO fail-fast rejects
    "rx:cast-int-as-address",
    "rx:address-ordered-cmp",
];

const RUNTIME_ERR_ITEMS: [&str; 2] = ["rt:budget-exhaustion", "rx:mirror-overflow"];

/// Items that CANNOT be hit, with the argument (G2.4: coverage = hits +
/// unreachable arguments == denominator).
const UNREACHABLE: &[(&str, &str)] = &[
    (
        "op:POP:Im",
        "POP is a bytecodegen artifact (compile.rs compile_if/compile_while push POP during \
         code emission, vm/src/ir/compile.rs:35,206,232,240) — it is never an IRNode, so no \
         fitsh body lowers through it. The only route to a POP in HVM bytecode is the \
         bytecode{} splice, which rv64 rejects as IRBYTECODE (a denominator rx: item).",
    ),
    (
        "op:POP:Imc",
        "Same bytecodegen-artifact argument as op:POP:Im — POP never appears as an IRNode in any fitsh body, in any lowering mode.",
    ),
    (
        "op:POP:ImcRemap",
        "Same bytecodegen-artifact argument as op:POP:Im — POP never appears as an IRNode in any fitsh body, in any lowering mode.",
    ),
    // op:IRLIST×3 was REMOVED from the unreachable registry by P3-V2IMPL
    // (A-25 M1): the compo-list literal wrapper now LOWERS in-shape (the
    // v2 shape set admits `[e0..eN-1]` creation + append/item_get/...), so
    // IRLIST is an ordinary hit-able denominator item (opcode_probe's
    // IRLIST shape is the list-literal body) — the same refuted-argument
    // pattern as t:cldsd:Imc below.
    (
        "t:poolld:Im",
        "The literal-pool scheme (pool base register + pooled loads/stores) is constructed ONLY inside the `if mode == Rv64Mode::ImcRemap` block (rv64.rs:2077, pool_base_reg set at :2141-2145 when lit_sites >= 4) — Im/Imc never activate a pool, so no PoolLd/PoolOff emission can exist in them.",
    ),
    ("t:poolld:Imc", "Pool scheme is ImcRemap-only (rv64.rs:2077,2141-2145)."),
    ("t:pooloff:Im", "Pool scheme is ImcRemap-only (rv64.rs:2077,2141-2145)."),
    ("t:pooloff:Imc", "Pool scheme is ImcRemap-only (rv64.rs:2077,2141-2145)."),
    ("s:pool-active:Im", "Pool scheme is ImcRemap-only (rv64.rs:2077,2141-2145)."),
    ("s:pool-active:Imc", "Pool scheme is ImcRemap-only (rv64.rs:2077,2141-2145)."),
    // t:cldsd:Imc was REMOVED from the unreachable registry by the P2-C3
    // saturation corpus: feedback seed 0xC3FB…+229 (bigout-family frame
    // traffic under the constraint-⑥/⑧ grammar reflow) emits a compressed
    // c.ld/c.sd in Imc — the predecessor's refutable argument ("a
    // constructing shape would flip it to hit") was refuted exactly as
    // anticipated. The item is now an ordinary hit-able denominator item.
];

/// The full denominator (deterministic order: ①×②, then ③④⑤⑥).
fn denom_items() -> Vec<&'static str> {
    let mut v = Vec::new();
    for op in SUBSET_OPS.iter() {
        for m in ["Im", "Imc", "ImcRemap"] {
            v.push(Box::leak(format!("op:{op}:{m}").into_boxed_str()) as &'static str);
        }
    }
    v.extend(GUARD_CLASSES.iter().copied());
    for (tier, modes) in TIER_ITEMS.iter() {
        for m in modes.iter() {
            v.push(Box::leak(format!("{tier}:{m}").into_boxed_str()) as &'static str);
        }
    }
    v.extend(SITE_ITEMS.iter().copied());
    v.extend(FRONTEND_ERR_ITEMS.iter().copied());
    v.extend(RV64_ERR_ITEMS.iter().copied());
    v.extend(RUNTIME_ERR_ITEMS.iter().copied());
    v
}

// P3-PAIRPKG: guards 36→38 (g:pair-oob-85 / g:pair-idx-72), sites += s:pair-fuse-sd (SITE_ITEMS.len())
// task 3: rx err items 28→24 — the four u128 reject faces (u128-cast /
// nested-arith / compare / literal-suffix) now LOWER and agree four-way in
// rv64_u128_arith.rs, so they left the reject-universe denominator
// T2: rx 24→22 — rx:mul-mod / rx:add-mod left the reject universe (add_mod
// and mul_mod lower at every width up to 128, rv64_modarith.rs)
// T5/T6: 22→18 — the mixed-width u128 folds and the u128 shift now LOWER
// and agree four-way in rv64_u128_arith.rs, so they left the reject universe
// T12: 18→17 — rx:type-id left the reject universe (type_id/is lower and
// agree now, vm/tests/rv64_misc_ops.rs)
// T3/T4: 17→15 — rx:mul-add / rx:mul-div left the reject universe
// (mul_add/mul_sub lower at every width via the u256 library, rv64_rowc.rs;
// mul_div/mul_div_up extend to the u64/u128 widths, rv64_muldiv.rs)
// T13: 15→17 — two NEW reject pins (rx:cast-int-as-address /
// rx:address-ordered-cmp): int→`as address` and ordered comparison on
// bytes/address stay B-family compile rejects (interpreter faults 71/90)
// T17: 17→9 — the eight list-literal escape rows (alias/rebind/nested/eq/
// ordered-cmp/cast/xop/choose-arm) left the reject universe; only the ABI
// list return and the bytes-receiver itemget stay pinned
pub const DENOM_LEN: usize = 76 * 3 + 38 + 27 + SITE_ITEMS.len() + 17 + 9 + 2;

/* ================================================================== */
/* Section 2: detectors (IR walk, listing decode, LowerDbg, knocks).   */
/* ================================================================== */

use vm::ir::{
    IRNode, IRNodeArray, IRNodeDouble, IRNodeParam1Single, IRNodeParam2Single,
    IRNodeParamsSingle, IRNodeSingle, IRNodeTriple, IRNodeWrapOne,
};

/// Every child of `node` in evaluation order (the rv64.rs:965 pattern).
fn children(node: &dyn IRNode) -> Vec<&dyn IRNode> {
    fn push<'a>(out: &mut Vec<&'a dyn IRNode>, b: &'a Box<dyn IRNode>) {
        out.push(&**b);
    }
    let any = node.as_any();
    let mut out = Vec::new();
    if let Some(n) = any.downcast_ref::<IRNodeWrapOne>() {
        push(&mut out, &n.node);
    } else if let Some(n) = any.downcast_ref::<IRNodeSingle>() {
        push(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeDouble>() {
        push(&mut out, &n.subx);
        push(&mut out, &n.suby);
    } else if let Some(n) = any.downcast_ref::<IRNodeTriple>() {
        push(&mut out, &n.subx);
        push(&mut out, &n.suby);
        push(&mut out, &n.subz);
    } else if let Some(n) = any.downcast_ref::<IRNodeParamsSingle>() {
        push(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam1Single>() {
        push(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeParam2Single>() {
        push(&mut out, &n.subx);
    } else if let Some(n) = any.downcast_ref::<IRNodeArray>() {
        for s in &n.subs {
            push(&mut out, s);
        }
    }
    out
}

/// Opcode multiset + shape flags of one body's IR.
#[derive(Default)]
pub struct IrFacts {
    pub ops: BTreeSet<String>,
    pub has_choose_node: bool,
    pub has_irifr: bool,
    pub has_xop: bool,
    /// P3-V2IMPL: an all-literal `[..]` table exists (rodata class)
    pub has_list_rodata: bool,
    /// P3-V2IMPL: a frame-resident list exists (computed elements, or any
    /// APPEND/CLEAR reachability — the growable 1024B block)
    pub has_list_frame: bool,
    /// P3-V2IMPL: the len shadow is live (any append/clear)
    pub has_list_mut: bool,
    /// P3-PAIRPKG: an adjacent REMOVE+INSERT statement pair exists
    /// (statement-array siblings — the fused single-sd form)
    pub has_pair: bool,
}

/// P3-V2IMPL: the shape-admitted compo opcodes are NOT dimension-① subset
/// members (doc 73 E11: opset churn +0 — they lower via the v2 shape
/// special case, not subset membership); their coverage rides on the
/// s:list-* sites, g:list-* guards and rx:list-* escape rows.
const SHAPE_ADMITTED_COMPO: [&str; 9] =
    ["PACKLIST", "NEWLIST", "ITEMGET", "APPEND", "CLEAR", "LENGTH", "HASKEY",
     // P3-PAIRPKG: the pair's opcodes ride the s:pair-fuse-sd / g:pair-*
     // rows like the other shape-admitted compo ops (opset churn +0)
     "REMOVE", "INSERT"];

fn ir_facts(body: &str) -> Result<IrFacts, String> {
    let (ir, _) = frontend(body)?;
    let mut f = IrFacts::default();
    fn walk(node: &dyn IRNode, f: &mut IrFacts) {
        let name = match vm::Bytecode::try_from_u8(node.bytecode()) {
            Ok(b) => format!("{b:?}"),
            Err(_) => return,
        };
        if SHAPE_ADMITTED_COMPO.contains(&name.as_str()) {
            // children still count (elements/keys are dimension-① ops);
            // P3-V2IMPL site flags: the packlist wrapper's storage class
            // mirrors the lowerer's (all-literal → rodata, computed →
            // frame), mutations force the growable frame block + shadow
            if name == "APPEND" || name == "CLEAR" {
                f.has_list_mut = true;
                f.has_list_frame = true;
            }
            for c in children(node) {
                walk(c, f);
            }
            return;
        }
        // the `[e0..eN-1, count, PACKLIST]` wrapper: all-literal elements
        // mean the rodata class (the plan-side ListKind decision)
        if name == "IRLIST" {
            if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                if arr.subs.len() >= 2 {
                    let last = &*arr.subs[arr.subs.len() - 1];
                    let is_packlist = last
                        .as_any()
                        .downcast_ref::<vm::ir::IRNodeLeaf>()
                        .map(|l| format!("{:?}", l.inst) == "PACKLIST")
                        .unwrap_or(false);
                    if is_packlist {
                        let n = arr.subs.len() - 2;
                        // all-literal elements → the rodata storage class.
                        // Structural check (the literal-node whitelist —
                        // `ir_literal_value` is crate-private); mirrors the
                        // plan()-side decision exactly: a literal element
                        // is a P0..PU16 leaf or a CU32/CU64-wrapped PBUF
                        // chain, nothing else.
                        fn lit_only(n: &dyn IRNode) -> bool {
                            let Ok(b) = vm::Bytecode::try_from_u8(n.bytecode()) else {
                                return false;
                            };
                            use vm::Bytecode::*;
                            if !matches!(b, P0 | P1 | P2 | P3 | PU8 | PU16 | PBUF | PBUFL | CU32 | CU64) {
                                return false;
                            }
                            children(n).into_iter().all(|c| lit_only(c))
                        }
                        if (0..n).all(|k| lit_only(&*arr.subs[k])) {
                            f.has_list_rodata = true;
                        } else {
                            // computed elements → the frame-static block
                            f.has_list_frame = true;
                        }
                    }
                }
            }
        }
        if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
            // the container's own opcode (IRLIST/IRBLOCK/IRBLOCKR) counts
            // P3-PAIRPKG: statement-array sibling adjacency — REMOVE at i
            // with INSERT at i+1 is the pair shape (the corpus generates
            // only recognizer-valid pairs; a hypothetical invalid pair
            // would overcount the site by a whitelist-mirror epsilon, the
            // doc 92 E1 precedent)
            for w in arr.subs.windows(2) {
                let a = vm::Bytecode::try_from_u8(w[0].bytecode()).ok();
                let b = vm::Bytecode::try_from_u8(w[1].bytecode()).ok();
                if a == Some(vm::Bytecode::REMOVE) && b == Some(vm::Bytecode::INSERT) {
                    f.has_pair = true;
                }
            }
            f.ops.insert(name);
        } else {
            if name == "CHOOSE" {
                f.has_choose_node = true;
            }
            if name == "IRIFR" {
                f.has_irifr = true;
            }
            if name == "XOP" {
                f.has_xop = true;
            }
            f.ops.insert(name);
        }
        for c in children(node) {
            walk(c, f);
        }
    }
    walk(&ir, &mut f);
    Ok(f)
}

/// Listing facts from decoding the production-lowered stream.
#[derive(Default)]
pub struct ListingFacts {
    pub tiers: BTreeSet<String>,
    pub sites: BTreeSet<String>,
}

fn listing_facts(code: &[u8], m: Rv64Mode, pool_base: Option<u8>) -> ListingFacts {
    let mut lf = ListingFacts::default();
    let profile = mode_profile(m);
    let mn = mode_name(m);
    let mut off = 0usize;
    let mut offs = Vec::new();
    let mut insts = Vec::new();
    while off < code.len() {
        let (inst, len) = match decode_at(code, off, profile) {
            Ok(x) => x,
            Err(_) => return lf, // defensive: measured code always decodes
        };
        offs.push(off);
        insts.push(inst);
        off += len as usize;
    }
    let mut add_tier = |lf: &mut ListingFacts, t: &str| {
        lf.tiers.insert(format!("{t}:{mn}"));
    };
    for (i, inst) in insts.iter().enumerate() {
        let len = if offs.get(i + 1).is_some() {
            offs[i + 1] - offs[i]
        } else {
            code.len() - offs[i]
        };
        match inst {
            Inst::Branch { .. } => {
                if len == 2 {
                    add_tier(&mut lf, "t:cbranch2");
                } else {
                    add_tier(&mut lf, "t:branch4");
                }
            }
            Inst::Jal { target, .. } => {
                if len == 2 {
                    add_tier(&mut lf, "t:cjump2");
                } else {
                    add_tier(&mut lf, "t:jal");
                }
                // trampoline tier: a jal whose displacement exceeds the
                // ±4 KiB B-type range — such a hop only exists because the
                // relaxation ladder routed a conditional through a
                // jal-trampoline (FinishErr::BranchOverflow retry)
                if target.abs() > 4096 {
                    add_tier(&mut lf, "t:trampoline");
                }
            }
            Inst::RegAlu { op: vm::calc::decode::RegAluOp::Add, rs1: 0, rd: 10, rs2 } => {
                // c.mv a0, x<rs2> + forward jal: user-value fault exit
                if let Some(Inst::Jal { target, .. }) = insts.get(i + 1) {
                    if *target > 0 && *rs2 != 0 {
                        lf.sites.insert("s:thunk-user".into());
                    }
                }
            }
            // P3-IDIOMS M3-b: lhu is emitted only by the fused byte-assembly
            // wide-load paths — presence = the fusion site fired
            Inst::Load { op: vm::calc::decode::LoadOp::Lhu, .. } => {
                lf.sites.insert("s:byte-fuse-wide".into());
            }
            Inst::Load { .. } | Inst::Store { .. } => {
                if len == 2 {
                    add_tier(&mut lf, "t:cldsd");
                }
                if let Some(pb) = pool_base {
                    if matches!(inst, Inst::Load { op: vm::calc::decode::LoadOp::Ld, rs1, .. } if *rs1 == pb)
                    {
                        add_tier(&mut lf, "t:poolld");
                    }
                }
            }
            Inst::Lui { .. } => {
                add_tier(&mut lf, "t:li");
            }
            Inst::ImmAlu { op, rd, rs1, imm } => {
                use vm::calc::decode::ImmAluOp::*;
                if matches!(op, Addi) && *rs1 == 0 && *imm != 0 {
                    add_tier(&mut lf, "t:li");
                }
                if matches!(op, Addi) && *imm != 0 {
                    if let Some(pb) = pool_base {
                        if *rs1 == pb {
                            add_tier(&mut lf, "t:pooloff");
                        }
                    }
                }
                // fault-exit staging signature: a write of the exit code
                // into a0 IMMEDIATELY followed by a forward jal to the
                // shared trap tail (li a0,N / mv a0,x + jal). A pool value
                // write is followed by a use, and the return path stages
                // a0=len before a jalr — neither matches.
                if *rd == 10 {
                    if let Some(Inst::Jal { target, .. }) = insts.get(i + 1) {
                        if *target > 0 {
                            if *rs1 == 0 && *imm > 0 {
                                match *imm {
                                    90 => { lf.sites.insert("s:thunk-90".into()); }
                                    71 => { lf.sites.insert("s:thunk-71".into()); }
                                    91 => { lf.sites.insert("s:thunk-91".into()); }
                                    72 => { lf.sites.insert("s:thunk-72".into()); }
                                    76 => { lf.sites.insert("s:thunk-76".into()); }
                                    _ => { lf.sites.insert("s:thunk-user".into()); }
                                }
                            } else if *imm == 0 && *rs1 != 0 {
                                lf.sites.insert("s:thunk-user".into());
                            }
                        }
                    }
                }
                // PcRelHiLo partner: auipc rd,X then addi rd,rd,imm
                if matches!(op, Addi) && i > 0 {
                    if let Inst::Auipc { rd: prd, .. } = &insts[i - 1] {
                        if prd == rd {
                            add_tier(&mut lf, "t:pcrhi");
                        }
                    }
                }
            }
            _ => {}
        }
    }
    lf
}

/// One measurement pass over a case: IR facts, per-mode LowerDbg + listing,
/// and (optionally) four-way knocks with guard-class attribution.
pub struct Hits {
    pub map: BTreeMap<String, String>, // item -> provenance (first hitter)
}

impl Hits {
    fn new() -> Self {
        Hits { map: BTreeMap::new() }
    }
    fn hit(&mut self, item: &str, provenance: &str) {
        self.map.entry(item.to_string()).or_insert_with(|| provenance.to_string());
    }
    fn contains(&self, item: &str) -> bool {
        self.map.contains_key(item)
    }
}

/// Compile-side measurement of one body (no knocks): op × mode items, tier
/// items, LowerDbg site items. `label` is the provenance string.
fn measure_compile(hits: &mut Hits, body: &str, label: &str) -> Result<(), String> {
    let (ir, _) = frontend(body)?;
    let facts = ir_facts(body)?;
    for mode in MODES {
        let m = mode_name(mode);
        // production lowering (the same call Rv64::try_build makes)
        let lo = lower_body_mode_arena(&ir, true, mode, ARENA_SIZE_MAX)
            .map_err(|e| format!("{label} {m}: {e}"))?;
        for op in facts.ops.iter() {
            hits.hit(&format!("op:{op}:{m}"), label);
        }
        // LowerDbg twin (byte-identical code, pinned by rv64.rs tests)
        let (_lo2, dbg) = lower_body_mode_arena_dbg(&ir, true, mode, ARENA_SIZE_MAX)
            .map_err(|e| format!("{label} {m} dbg: {e}"))?;
        if dbg.banked.iter().any(|&b| b == BANK_KIND_INT) {
            hits.hit("s:bank-int:ImcRemap", label);
        }
        if dbg.banked.iter().any(|&b| b == BANK_KIND_BYTES) {
            hits.hit("s:bank-bytes:ImcRemap", label);
        }
        let frame_slots = dbg.frame_off.iter().any(|&o| o >= 0);
        if frame_slots {
            hits.hit(&format!("s:frame-slot:{m}"), label);
        }
        if facts.has_xop {
            if frame_slots {
                hits.hit(&format!("s:xop-frame:{m}"), label);
            }
            if mode == Rv64Mode::ImcRemap && !dbg.banked.iter().all(|&b| b == 0) {
                hits.hit("s:xop-bank:ImcRemap", label);
            }
        }
        if dbg.pool_base_reg.is_some() {
            hits.hit(&format!("s:pool-active:{m}"), label);
        }
        if !dbg.mul_merges.is_empty() {
            hits.hit("s:mul-merge-applied", label);
            hits.hit("g:mul-merge-reloc", label);
        }
        let lf = listing_facts(&lo.code, mode, dbg.pool_base_reg);
        for t in lf.tiers.iter() {
            hits.hit(t, label);
        }
        for s in lf.sites.iter() {
            hits.hit(s, label);
        }
        // ref materialization: an mv whose source is a bank register
        if !dbg.bank_regs.is_empty() {
            let mut off = 0usize;
            let profile = mode_profile(mode);
            while off < lo.code.len() {
                let (inst, len) = decode_at(&lo.code, off, profile).map_err(|e| format!("{e:?}"))?;
                let is_mv_from_bank = match &inst {
                    Inst::ImmAlu { op: vm::calc::decode::ImmAluOp::Addi, imm: 0, rs1, rd } => {
                        *rs1 != 0 && rd != rs1 && dbg.bank_regs.contains(rs1)
                    }
                    Inst::RegAlu { op: vm::calc::decode::RegAluOp::Add, rs1: 0, rs2, .. } => {
                        dbg.bank_regs.contains(rs2)
                    }
                    _ => false,
                };
                if is_mv_from_bank {
                    hits.hit("s:ref-mv:ImcRemap", label);
                }
                off += len as usize;
            }
        }
    }
    // IR-presence site classes
    if facts.has_irifr {
        for mode in MODES {
            hits.hit(&format!("s:join-u64:{}", mode_name(mode)), label);
        }
    }
    if facts.has_choose_node {
        for mode in MODES {
            hits.hit(&format!("s:choose-eager-int:{}", mode_name(mode)), label);
        }
    }
    // P3-V2IMPL list emit sites (IR-presence keyed, see IrFacts)
    if facts.has_list_frame {
        hits.hit("s:list-frame-block", label);
    }
    if facts.has_list_rodata {
        hits.hit("s:list-rodata", label);
    }
    if facts.has_list_mut {
        hits.hit("s:len-shadow", label);
    }
    if facts.has_pair {
        hits.hit("s:pair-fuse-sd", label);
    }
    Ok(())
}

/// Guard-class attribution from one knock's interpreter fault + verdict.
fn guard_items_for(
    code: u16,
    msg: &str,
    verdict: &Verdict,
    feat: &Feat,
    hits: &mut Hits,
    label: &str,
) {
    let last_num = trailing_number(msg).unwrap_or(u64::MAX);
    let nar = narrow_payload(msg);
    let g = |h: &mut Hits, id: &str| h.hit(id, label);
    if msg.contains("out of gas") {
        g(hits, "rt:budget-exhaustion");
        return;
    }
    // agree/pinned trap families first
    if code == 90 && msg.contains("arithmetic add check failed") {
        g(hits, "g:add-ovf-90");
    }
    if code == 90 && msg.contains("arithmetic sub check failed") {
        g(hits, "g:sub-ovf-90");
    }
    if code == 90 && (msg.contains("arithmetic mul check failed") && !nar) {
        g(hits, "g:mul-ovf-90");
    }
    if code == 90 && msg.contains("arithmetic div check failed") && last_num == 0 {
        g(hits, "g:div-zero-90");
    }
    if code == 90 && msg.contains("arithmetic mod check failed") {
        g(hits, "g:mod-zero-90");
    }
    if code == 90 && msg.contains("arithmetic div_up check failed") {
        g(hits, "g:divup-zero-90");
    }
    if code == 90 && msg.contains("div_exact_op check failed") {
        if last_num == 0 {
            g(hits, "g:divexact-zero-90");
        } else {
            g(hits, "g:divexact-inexact-90");
        }
    }
    if code == 90 && msg.contains("arithmetic pow check failed") {
        g(hits, "g:pow-ovf-90");
    }
    if code == 90 && msg.contains("clamp check failed") {
        g(hits, "g:clamp-lohi-90");
    }
    if code == 71 && msg.contains(" to U8") {
        g(hits, "g:cast-u8-71");
    }
    if code == 71 && msg.contains(" to U16") {
        g(hits, "g:cast-u16-71");
    }
    if code == 71 && msg.contains(" to U32") {
        g(hits, "g:cast-u32-71");
    }
    if code == 42 && msg.contains("read buf byte overflow") {
        g(hits, "g:byte-oob-d2");
    }
    if code == 42 && msg.contains("cutout buf overflow") {
        g(hits, "g:slice-cut-oob-d2");
    }
    if code == 42 && (msg.contains("drop buf left overflow") || msg.contains("drop buf right overflow") || msg.contains("cut buf left overflow") || msg.contains("cut buf right overflow")) {
        g(hits, "g:slice-drop-oob-d2");
    }
    if code == 39 {
        g(hits, "g:arena-churn-d4");
    }
    if code == 91 {
        g(hits, "g:bytes-nil-91");
    }
    if code == 90 && msg.contains("shift overflow") && !nar {
        g(hits, "g:shift-mask-d1");
    }
    if code == 90 && msg.contains("cannot compare different types") {
        if msg.contains("Nil and") || msg.contains("and Nil") {
            g(hits, "g:nil-cmp-eq-90");
        } else {
            g(hits, "g:nil-cmp-ord-90");
        }
    }
    if code == 90 && msg.contains("ordered compare") {
        g(hits, "g:nil-cmp-ord-90");
    }
    if code == 71 && msg.contains("cannot cast Nil to uint") {
        g(hits, "g:int-nil-arith-71");
    }
    if code == 71 && msg.contains("cannot cast Bool") {
        g(hits, "g:bool-uint-d5");
    }
    if code == 71 && msg == "" || (code == 71 && msg.ends_with("(71): ")) {
        // empty-message CBYTES-on-Nil form (canbe.rs)
        g(hits, "g:nil-cbytes-71");
    }
    if code == 72 {
        g(hits, "g:nil-param-72");
    }
    if code == 76 {
        g(hits, "g:nil-reqvalue-76");
    }
    if code == 150 {
        g(hits, "g:require-user-150");
    }
    if code == 151 && msg.contains("VM return failed: nil") {
        g(hits, "g:assert-abort-d3");
    }
    if code == 151 && !msg.contains("nil") {
        g(hits, "g:throw-user-151");
    }
    if code == 90 && nar {
        g(hits, "g:narrow-width-d5");
    }
    if feat.and_chain && matches!(verdict, Verdict::AgreeOk | Verdict::AgreeTrap90 | Verdict::AgreeTrap71 | Verdict::AgreeUserValue(_)) {
        g(hits, "g:and-chain-pure");
    }
    if feat.bytes_if_arm {
        for m in ["Im", "Imc", "ImcRemap"] {
            hits.hit(&format!("s:join-bytes:{m}"), label);
        }
    }
}

/// Knock a built case (interp + 3 modes) with guard attribution.
fn measure_knock(
    hits: &mut Hits,
    rv: &Rv64,
    it: &Interp,
    feat: &Feat,
    input: &[u8],
    label: &str,
) -> Verdict {
    let ir = it.run(input);
    let mut rr = rv.run(input);
    if ir.is_ok() && rr.iter().any(|(_, r)| matches!(r, Err(0))) {
        rr = rv.run_with_limits(
            input,
            vm::calc::CalcRunLimits { cycles: RV64_CYCLES_RETRY, resource: -1, deadline: None },
        );
    }
    let v = classify(feat, &ir, &rr);
    if let Err(f) = &ir {
        guard_items_for(f.code, &f.msg, &v, feat, hits, label);
    } else {
        // fault-free attribution rows (feat-keyed, e.g. the eager-AND
        // chain over pure comparisons agreeing)
        if feat.and_chain
            && matches!(v, Verdict::AgreeOk | Verdict::AgreeUserValue(_))
        {
            hits.hit("g:and-chain-pure", label);
        }
    }
    if feat.bytes_if_arm {
        for m in ["Im", "Imc", "ImcRemap"] {
            hits.hit(&format!("s:join-bytes:{m}"), label);
        }
    }
    v
}

/* ================================================================== */
/* Section 3: corpus measurement (feedback loop, plateau detection).   */
/* ================================================================== */

/// Measure one generated case end-to-end (compile-side + knocks). Returns
/// unexpected verdicts (asserted empty by callers).
fn measure_case(hits: &mut Hits, seed: u64, n_stmts: u64, label: &str) -> Vec<String> {
    // deterministic retry, mirroring run_campaign (the campaign's downsizing
    // levers are part of the corpus contract — doc 70 §1.2: depth_err →
    // halve budgets ≤6, mirror overflow → halve stmts ≤6, both pure
    // functions of the seed). Without it, seeds whose first-draft body
    // exceeds the 8-value pool or the 16383 B mirror would surface as
    // spurious "rv64 reject" unexpecteds (seen on the feedback domain
    // 0xC3FB…, 5 seeds in rounds 0-2).
    let mut n = n_stmts;
    let mut budget = GenBudget::default();
    let mut tries = 0u64;
    let case = loop {
        let case = generate_case(seed, n, budget);
        if case.feat.mixed_arm {
            break case; // negative channel: never builds, no retry
        }
        let build = if case.family == "bigout" {
            Rv64::try_build_out(&case.body, 64, 0x7000)
        } else {
            Rv64::try_build(&case.body)
        };
        match build {
            Ok(_) => break case,
            Err(e) if e.contains("calcfunc size overflow") => {
                tries += 1;
                if tries > 6 {
                    // mirror budget exhausted even downsized — same skip
                    // semantics as run_campaign ("still over mirror budget")
                    return vec![];
                }
                n = (n / 2).max(2);
            }
            Err(e) if e.contains("nesting exceeds") => {
                tries += 1;
                if tries > 6 {
                    return vec![format!("seed {seed}: still over depth after halving")];
                }
                budget.halve();
            }
            Err(e) => return vec![format!("seed {seed}: rv64 reject {e}")],
        }
    };
    if case.feat.mixed_arm {
        // negative channel (same rule as run_campaign): interp accepts+runs,
        // rv64 rejects with the pinned text; hits the rx item + choose site.
        let it = Interp::try_build(&case.body).expect("mixed-arm interp build");
        for input in case.inputs.iter() {
            let _ = it.run(input);
        }
        let err = match Rv64::try_build(&case.body) {
            Err(e) => e,
            Ok(_) => panic!("mixed-arm case must reject"),
        };
        assert!(
            err.contains("choose arms must have the same result type")
                // P2-C3FIX-BUG017: the up-front arm-type reject; the three
                // leak texts below are the PRE-fix faces (kept defensive —
                // the leak is gone, but a regression would land here)
                || err.contains("value stack not empty")
                || err.contains("leaves values on the stack")
                // the leak's cascade face: the leaked entry makes the NEXT
                // statement's IRWHILE illegal (same root, see run_campaign)
                || err.contains("while loop in expression position"),
            "mixed-arm reject text: {err}"
        );
        hits.hit("rx:choose-mixed-arm", label);
        return vec![];
    }
    let build = if case.family == "bigout" {
        Rv64::try_build_out(&case.body, 64, 0x7000)
    } else {
        Rv64::try_build(&case.body)
    };
    let (rv, it) = match (build, Interp::try_build(&case.body)) {
        (Ok(r), Ok(i)) => (r, i),
        (Err(e), _) if e.contains("calcfunc size overflow") => {
            // deterministic mirror-overflow reject (bigbody tail) — the
            // campaign downsizes; for measurement the REJECT itself is the
            // rx:mirror-overflow hit
            hits.hit("rx:mirror-overflow", label);
            return vec![];
        }
        (Err(e), _) => {
            return vec![format!("seed {seed}: rv64 reject {e}")];
        }
        (_, Err(e)) => {
            return vec![format!("seed {seed}: frontend reject {e}")];
        }
    };
    if let Err(e) = measure_compile(hits, &case.body, label) {
        return vec![format!("seed {seed}: measure {e}")];
    }
    if case.feat.bigout {
        hits.hit("s:output-copy-wide", label);
    }
    let mut bad = Vec::new();
    for input in case.inputs.iter() {
        let v = measure_knock(hits, &rv, &it, &case.feat, input, label);
        if let Verdict::Unexpected(r) = v {
            bad.push(format!("seed {seed} input {}B: {r}", input.len()));
        }
    }
    bad
}

/// Deterministic seed-range measurement with plateau detection: rounds of
/// 250 cases each; stops when a full round adds zero new denominator items.
fn measure_corpus(hits: &mut Hits, seed0: u64, rounds: u64, label_prefix: &str) -> Vec<String> {
    let mut all_bad = Vec::new();
    for r in 0..rounds {
        let before = hits.map.len();
        for i in 0..250u64 {
            let seed = seed0.wrapping_add(r * 1000).wrapping_add(i);
            all_bad.extend(measure_case(hits, seed, 10, &format!("{label_prefix}r{r}+{i}")));
        }
        let added = hits.map.len() - before;
        println!("[c3-corpus {label_prefix} round {r}: +{added} items, total {}]", hits.map.len());
        if added == 0 {
            break;
        }
    }
    all_bad
}

/* ================================================================== */
/* Section 4: targeted probes — the directed generator for unhit items */
/* (static, deterministic bodies; no randomness enters any artifact).  */
/* ================================================================== */

const IN16: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const IN1: [u8; 1] = [3];

/// One minimal body per subset opcode (the on-demand directed generator:
/// the feedback loop pulls a shape whenever `op:X:*` is unhit after a
/// corpus round). Bodies are deliberately tiny and knock-clean.
fn opcode_probe(op: &str) -> Option<&'static str> {
    let s: &str = match op {
        "P0" => "var a = 0u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "P1" => "var a = 1u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "P2" => "var a = 2u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "P3" => "var a = 3u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "PU8" => "var a = 200u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "PU16" => "var a = 1000u64\nvar b = (byte(data,0) as u64)\nreturn (a + b) as bytes",
        "PBUF" => "var s = \"AB\"\nreturn (size(s) as u64) as bytes",
        "PBUFL" => {
            // 300-char bytes literal: push_bytes selects PBUFL for len>255
            // (mod.rs push_bytes, isl = bl > u8::MAX)
            return Some(Box::leak(
                format!("var s = \"{}\"\nreturn (size(s) as u64) as bytes", "Q".repeat(300))
                    .into_boxed_str(),
            ));
        }
        "PNBUF" => "var s = \"\"\nreturn (size(s) as u64) as bytes",
        "PTRUE" | "PFALSE" => {
            "var r = ((true) as u64)\nvar t = ((false) as u64)\nreturn (r + t) as bytes"
        }
        "CU8" => "var a = (byte(data,0) as u64)\nvar r = ((a as u8) as u64)\nreturn (r) as bytes",
        "CU16" => "var a = (byte(data,0) as u64)\nvar r = ((a as u16) as u64)\nreturn (r) as bytes",
        "CU32" => "var a = (byte(data,0) as u64)\nvar r = ((a as u32) as u64)\nreturn (r) as bytes",
        "CU64" => "var a = (byte(data,0) as u64)\nvar r = ((a as u64) + 1u64)\nreturn (r) as bytes",
        "CBYTES" => "var a = (byte(data,0) as u64)\nreturn (a) as bytes",
        "ADD" => "var a = (byte(data,0) as u64)\nreturn (a + 2u64) as bytes",
        "SUB" => "var a = (byte(data,0) as u64)\nreturn (a - 1u64) as bytes",
        "MUL" => "var a = (byte(data,0) as u64)\nreturn (a * 3u64) as bytes",
        "DIV" => "var a = (byte(data,0) as u64)\nreturn (a / 3u64) as bytes",
        "MOD" => "var a = (byte(data,0) as u64)\nreturn (a % 3u64) as bytes",
        "POW" => "var a = (byte(data,0) as u64)\nreturn (a ** 3u64) as bytes",
        "MAX" => "var a = (byte(data,0) as u64)\nreturn (max(a, 5u64)) as bytes",
        "MIN" => "var a = (byte(data,0) as u64)\nreturn (min(a, 5u64)) as bytes",
        "CLAMP" => "var a = (byte(data,0) as u64)\nreturn (clamp(a, 1u64, 9u64)) as bytes",
        "ABSDIFF" => "var a = (byte(data,0) as u64)\nreturn (abs_diff(a, 5u64)) as bytes",
        "DIVUP" => "var a = (byte(data,0) as u64)\nreturn (div_up(a, 3u64)) as bytes",
        "DIVEXACT" => {
            "var a = (byte(data,0) as u64)\nreturn (div_exact_op((a * 3u64), 3u64)) as bytes"
        }
        "BSHL" => "var a = (byte(data,0) as u64)\nreturn (a << 3u64) as bytes",
        "BSHR" => "var a = (byte(data,0) as u64)\nreturn (a >> 3u64) as bytes",
        "BXOR" => "var a = (byte(data,0) as u64)\nreturn (a ^ 5u64) as bytes",
        "BOR" => "var a = (byte(data,0) as u64)\nreturn (a | 5u64) as bytes",
        "BAND" => "var a = (byte(data,0) as u64)\nreturn (a & 5u64) as bytes",
        "EQ" => "var a = (byte(data,0) as u64)\nreturn ((a == 5u64) as u64) as bytes",
        "NEQ" => "var a = (byte(data,0) as u64)\nreturn ((a != 5u64) as u64) as bytes",
        "LT" => "var a = (byte(data,0) as u64)\nreturn ((a < 5u64) as u64) as bytes",
        "GT" => "var a = (byte(data,0) as u64)\nreturn ((a > 5u64) as u64) as bytes",
        "LE" => "var a = (byte(data,0) as u64)\nreturn ((a <= 5u64) as u64) as bytes",
        "GE" => "var a = (byte(data,0) as u64)\nreturn ((a >= 5u64) as u64) as bytes",
        "NOT" => "var a = (byte(data,0) as u64)\nreturn ((!(a == 5u64)) as u64) as bytes",
        "AND" => "var a = (byte(data,0) as u64)\nreturn ((a == 5u64 && a < 9u64) as u64) as bytes",
        "OR" => "var a = (byte(data,0) as u64)\nreturn ((a == 5u64 || a < 9u64) as u64) as bytes",
        "CHOOSE" => {
            "var a = (byte(data,0) as u64)\nvar r = (a != 0u64) ? (a + 1u64) : 7u64\nreturn (r) as bytes"
        }
        "CAT" => "var s = (buf_left(2, data) ++ \"x\")\nreturn (size(s) as u64) as bytes",
        "BYTE" => "return (byte(data, 0) as u64) as bytes",
        "CUT" => "var s = buf_cut(data, 1u64, 4u64)\nreturn (size(s) as u64) as bytes",
        "LEFT" => "var s = buf_left(4, data)\nreturn (size(s) as u64) as bytes",
        "RIGHT" => "var s = buf_right(4, data)\nreturn (size(s) as u64) as bytes",
        "LDROP" => "var s = buf_left_drop(2, data)\nreturn (size(s) as u64) as bytes",
        "RDROP" => "var s = buf_right_drop(2, data)\nreturn (size(s) as u64) as bytes",
        "SIZE" => "return (size(data) as u64) as bytes",
        "GET0" => "return (byte(data, 0) as u64) as bytes", // data = slot 0
        "ROLL0" => "return (byte(data, 0) as u64) as bytes", // every data prelude carries it
        "GET1" | "GET2" | "GET3" | "GET" => {
            // five live locals: slots 1..3 use GET1..GET3, >=4 uses GET
            "var a = (byte(data,0) as u64)\nvar b = 1u64\nvar c = 2u64\nvar d = 3u64\nvar e = 4u64\nreturn ((a + b + c + d + e)) as bytes"
        }
        "PUT" | "XOP" | "ALLOC" => {
            // two live vars: the single-live-var `a += 1u64; return a`
            // shape is the BUG-018 candidate (stale forward in Im/Imc),
            // pinned separately — the opcode shape uses the working form
            "var a = (byte(data,0) as u64)\nvar b = (byte(data,1) as u64)\na += 1u64\nb += a\nreturn ((a + b)) as bytes"
        }
        // P3-V2IMPL: IRLIST lowers in-shape as the compo-list literal
        // wrapper ([e0..eN-1, count, PACKLIST], context.rs build_list_node)
        "IRLIST" => {
            "var k = [3u64, 5u64, 7u64]\nvar i = (byte(data,0) as u64) % 3u64\nvar r = k[i] + k[0]\nreturn (r as u64) as bytes"
        }
        "IRBLOCK" | "IRBLOCKR" | "IRIF" | "IRIFR" | "IRWHILE" | "IRBREAK"
        | "IRCONTINUE" => {
            "var q = (byte(data,0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile i < 4 { i += 1\nif i == 2 { continue }\nif i == 3 { break }\ns += i }\nvar t = if (q == 0u64) { s } else { 1u64 }\nreturn (t) as bytes"
        }
        "REQ" => "var a = (byte(data,0) as u64)\nrequire 42u64, (a < 200u64)\nreturn (a) as bytes",
        "AST" => "var a = (byte(data,0) as u64)\nassert (a < 200u64)\nreturn (a) as bytes",
        "ERR" => "var a = (byte(data,0) as u64)\nif (a > 200u64) { throw 7u64 }\nreturn (a) as bytes",
        "ABT" => "var a = (byte(data,0) as u64)\nif (a > 200u64) { abort }\nreturn (a) as bytes",
        "RET" => "return 5u64 as bytes",
        "END" => "",
        "NOP" => "var q = 0u64\nif (q == 1u64) { q = 2u64 }\nreturn (q) as bytes",
        _ => return None,
    };
    Some(s)
}

/// Guard-class probes: (item-id, body, input, accepted verdict debug names).
const GUARD_PROBES: &[(&str, &str, &[u8], &[&str])] = &[
    ("g:add-ovf-90", "var a = (byte(data,0) as u64)\nvar hi = 9223372036854775808u64\nvar r = (hi + (hi + (a - a)))\nreturn (r) as bytes", &[9], &["AgreeTrap90"]),
    ("g:sub-ovf-90", "var a = (byte(data,0) as u64)\nvar r = ((a % 10u64) - 100u64)\nreturn (r) as bytes", &[5], &["AgreeTrap90"]),
    ("g:mul-ovf-90", "var a = (byte(data,0) as u64)\nvar r = ((a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64) * (a | 16u64))\nreturn (r) as bytes", &[1], &["AgreeTrap90"]),
    ("g:div-zero-90", "var a = (byte(data,0) as u64)\nvar z = (a - a)\nvar r = (7u64 / z)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:mod-zero-90", "var a = (byte(data,0) as u64)\nvar z = (a - a)\nvar r = (7u64 % z)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:divup-zero-90", "var a = (byte(data,0) as u64)\nvar z = (a - a)\nvar r = div_up(7u64, z)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:divexact-zero-90", "var a = (byte(data,0) as u64)\nvar z = (a - a)\nvar r = div_exact_op(7u64, z)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:divexact-inexact-90", "var a = (byte(data,0) as u64)\nvar r = div_exact_op(7u64, 2u64)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:pow-ovf-90", "var a = (byte(data,0) as u64)\nvar b = (a | 4u64)\nvar r = (b ** 40u64)\nreturn (r) as bytes", &[9], &["AgreeTrap90"]),
    ("g:clamp-lohi-90", "var a = (byte(data,0) as u64)\nvar r = clamp(a, 9u64, 2u64)\nreturn (r) as bytes", &[3], &["AgreeTrap90"]),
    ("g:cast-u8-71", "var a = (byte(data,0) as u64)\nvar r = ((a * 100u64) as u8)\nreturn (r as u64) as bytes", &[9], &["AgreeTrap71"]),
    ("g:cast-u16-71", "var a = (byte(data,0) as u64)\nvar r = ((a * 100000u64) as u16)\nreturn (r as u64) as bytes", &[9], &["AgreeTrap71"]),
    ("g:cast-u32-71", "var a = (byte(data,0) as u64)\nvar r = ((a * 10000000000u64) as u32)\nreturn (r as u64) as bytes", &[9], &["AgreeTrap71"]),
    ("g:byte-oob-d2", "var r = (byte(data, 999) as u64)\nreturn (r) as bytes", &[1, 2, 3], &["PinD2BoundsClass"]),
    ("g:slice-cut-oob-d2", "var r = buf_cut(data, 1u64, 999u64)\nreturn r", &[1, 2, 3], &["PinD2BoundsClass"]),
    ("g:slice-drop-oob-d2", "var r = buf_left_drop(9, data)\nreturn r", &[1, 2, 3], &["PinD2BoundsClass"]),
    ("g:arena-churn-d4", "var cb = buf_left(200, data)\nvar ci = 0u64\nwhile ci < 60 { cb = cb ++ buf_left(200, data)\nci += 1 }\nreturn (size(cb) as u64) as bytes", &[0x61; 1280], &["PinD4ChurnBand"]),
    ("g:bytes-nil-91", "var q = 0u64\nif (q == 1u64) { var s1 = buf_left(2, data) }\nvar r = (byte(s1, 0) as u64)\nreturn (r) as bytes", &[0], &["AgreeTrap91"]),
    ("g:int-nil-arith-71", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = (n + 1u64)\nreturn (r) as bytes", &[0], &["AgreeTrap71"]),
    ("g:nil-cmp-eq-90", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = (n == 5u64)\nreturn (r as u64) as bytes", &[0], &["AgreeTrap90"]),
    ("g:nil-cmp-ord-90", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar r = (n > 1u64)\nreturn (r as u64) as bytes", &[0], &["AgreeTrap90"]),
    ("g:nil-xop-old-71", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nn += 1u64\nreturn (n) as bytes", &[0], &["AgreeTrap71"]),
    ("g:nil-cbytes-71", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nreturn (n) as bytes", &[0], &["AgreeTrap71"]),
    ("g:nil-param-72", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nvar s1 = buf_left(8, data)\nvar r = buf_cut(s1, n, 2u64)\nreturn r", &IN16, &["AgreeTrap71"]), // rv64 72, interp 72 -> class-equal row
    ("g:nil-reqvalue-76", "var q = 0u64\nif (q == 1u64) { var n = 5u64 }\nrequire n, (q == 0u64)\nreturn (q) as bytes", &[0], &["Unexpected"] /* replaced at runtime: 76,76 agree pair added below */),
    ("g:require-user-150", "var a = (byte(data,0) as u64)\nrequire 42u64, (a == 999u64)\nreturn (a) as bytes", &[3], &["AgreeUserValue"]),
    // P3-V2IMPL list trap family: idx 3..5 over a 2-element table →
    // CompoNoFindItem(85) on both engines at the same input
    ("g:list-oob-85", "var k = [7u64, 8u64]\nvar i = (byte(data, 0) as u64) % 6u64\nvar r = k[i]\nreturn (r as u64) as bytes", &[3, 4, 5], &["AgreeTrap85"]),
    // idx ≥ 2³² → CastParamFail(72) BEFORE the bounds compare (extract_u32)
    ("g:list-idx-72", "var k = [7u64, 8u64]\nvar i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\nvar r = k[i]\nreturn (r as u64) as bytes", &[0, 1], &["AgreeTrap72"]),
    // the 129th append → OutOfCompoLen(40) on both engines
    ("g:list-cap-40", "var w = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(w, i)\ni += 1\n}\nappend(w, 5u64)\nreturn 0u64 as bytes", &[0], &["AgreeTrap40"]),
    // P3-PAIRPKG pair trap family: idx = byte%3 over a 2-element list →
    // idx 2 ≥ len → the pair's remove bound 85 on both engines (the pair
    // is the body's only fault site — direct attribution)
    ("g:pair-oob-85", "var l = [1u64, 2u64]\nvar i = ((byte(data, 0) as u64) % 3u64)\nremove(l, i)\ninsert(l, i, 9u64)\nreturn (l[0] + l[1]) as bytes", &[2], &["AgreeTrap85"]),
    // pair idx ≥ 2³² → CastParamFail(72) at the extract face
    ("g:pair-idx-72", "var l = [1u64, 2u64]\nvar i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\nremove(l, i)\ninsert(l, i, 9u64)\nreturn (l[0] + l[1]) as bytes", &[0, 1], &["AgreeTrap72"]),
    ("g:throw-user-151", "var a = (byte(data,0) as u64)\nif (a == 3u64) { throw 7u64 }\nreturn (a) as bytes", &[3], &["AgreeUserValue"]),
    ("g:assert-abort-d3", "var a = (byte(data,0) as u64)\nassert (a == 999u64)\nreturn (a) as bytes", &[3], &["PinD3AssertAbort"]),
    ("g:shift-mask-d1", "var a = (byte(data,0) as u64)\nvar r = (a << 64u64)\nreturn (r) as bytes", &[3], &["PinD1ShiftMask"]),
    // bool-uint arithmetic is a pinned D5 deviation (interp to_uint → 71,
    // rv64 computes 0/1): the mixed-width add leaves `r` with no static
    // uint width, so the A2 `as bytes` gate requires the `as u64` pin
    // (runtime verdict unchanged — the class is keyed on the interp fault)
    ("g:bool-uint-d5", "var a = (byte(data,0) as u64)\nvar r = (a + (!(a == 0u64)))\nreturn ((r as u64)) as bytes", &[3], &["PinD5BoolUint"]),
    ("g:narrow-width-d5", "var r = (((byte(data, 0) * byte(data, 1)) + 1u8) as u64)\nreturn (r) as bytes", &[200, 200], &["PinD5NarrowWidth"]),
    ("g:and-chain-pure", "var a = (byte(data,0) as u64)\nif (a > 2u64) && (a < 200u64) { a = a + 1u64 }\nreturn (a) as bytes", &[3], &["AgreeOk", "AgreeTrap90", "AgreeTrap71"]),
    ("rt:budget-exhaustion", "var i = 0u64\nwhile i == 0u64 { i = 0u64 }\nreturn (i) as bytes", &IN1, &["Budget"]),
];

/// Emit-site / tier / error probes beyond the guard table.
const SITE_PROBES: &[(&str, &str, &[u8])] = &[
    // pool family: many big literals + bytes literals activate the pool
    // base (pcrhi in the prologue), PoolLd (li_k >0xFFFF) and PoolOff
    // (lit_addr for bytes literal addresses)
    ("pool-family", "var a = (byte(data,0) as u64)\nvar s = \"ABCDEFGH\"\nvar x = (a + 1099511627776u64)\nvar y = (a + 8589934592u64)\nvar z = (a + 2199023255552u64)\nvar t = (size(s) as u64)\nreturn ((x + y + z + t)) as bytes", &IN16),
    // bigout wide-output copy (output_max 64 envelope)
    ("bigout-64", "return buf_left(64, data)", &[0xA5; 64]),
    // needs-zero: tolerant-path reads of uninit slots (SIZE / AND) agree
    ("needs-zero", "var q = 0u64\nif (q == 1u64) { var s1 = buf_left(2, data) }\nif (q == 1u64) { var n = 5u64 }\nvar r = (size(s1) as u64)\nvar w = ((n == 0u64) || (q == 0u64))\nreturn ((r + (w as u64))) as bytes", &[0]),
    // bytes-if join (constraint ⑥ unfreeze) incl. banked arms
    ("join-bytes", "var c0 = (byte(data, 0) as u64)\nvar r1 = buf_left(2, data)\nvar r2 = buf_right(2, data)\nvar r3 = (r1 ++ r2)\nvar rr = if ((c0 % 2u64) == 0u64) { (r1 ++ r3) } else { (r2 ++ r1) }\nreturn (rr)", &IN16),
    // choose bytes arms — the BUG-017 shape (pinned divergence, measured)
    ("choose-bytes-bug017", "var c = (byte(data, 0) as u64)\nvar r = choose(((c % 2u64) == 0u64), buf_left(2, data), buf_right(3, data))\nreturn r", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
    // XOP on a bank-promoted slot (self-ref — the BUG-013 fix path)
    ("xop-bank", "var c0 = (byte(data, 0) as u64)\nvar c1 = (byte(data, 1) as u64)\nvar c2 = (byte(data, 2) as u64)\nvar c3 = (byte(data, 3) as u64)\nvar c4 = (byte(data, 4) as u64)\nc0 += c0\nc1 *= c2\nc3 -= 1u64\nc4 /= (c4 | 1u64)\nreturn ((c0 + c1 + c3 + c4)) as bytes", &IN16),
    // frame traffic under pressure (defeats store-to-load forwarding):
    // many vars rotated in a loop force real c.ld/c.sd frame accesses in
    // the RVC modes, and more locals than the 10 bank registers leaves
    // frame slots even in ImcRemap
    ("frame-traffic", "var v0 = (byte(data,0) as u64)\nvar v1 = (byte(data,1) as u64)\nvar v2 = (byte(data,2) as u64)\nvar v3 = (byte(data,3) as u64)\nvar v4 = (byte(data,4) as u64)\nvar v5 = (byte(data,5) as u64)\nvar v6 = (byte(data,6) as u64)\nvar v7 = (byte(data,7) as u64)\nvar v8 = (byte(data,8) as u64)\nvar v9 = (byte(data,9) as u64)\nvar v10 = (byte(data,10) as u64)\nvar v11 = (byte(data,11) as u64)\nvar v12 = (byte(data,12) as u64)\nvar v13 = (byte(data,13) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile i < 4 { i += 1\nv0 = v0 + v7\nv1 = v1 + v8\nv2 = v2 + v9\nv3 = v3 + v10\nv4 = v4 + v11\nv5 = v5 + v12\nv6 = v6 + v13\ns = ((v0 + v1) + (v2 + v3)) + ((v4 + v5) + (v6 + s)) }\nreturn (s) as bytes", &IN16),
    // A-19 merge shape (score_sum stmt3-4 twin): dominated mul checks
    ("mul-merge", "var n = (size(data) / 24)\nvar ne = n * 6\nvar nue = ne + (n * 8)\nvar sum = nue\nvar i = 0u64\nwhile i < n { sum += i\ni += 1 }\nreturn (sum) as bytes", &[0x61; 48]),
    // compressed c.ld/c.sd in IMC — the predecessor's "refutable
    // argument" for t:cldsd:Imc was refuted by the P2-C3 saturation
    // corpus (feedback seed 0xC3FB…+229); this is the delta-minimized
    // shape (churn-family slot traffic): wide clamp/choose temporaries
    // over byte slots put a value in the x8-15 window at a frame access
    // in the non-remap Imc lowering. Knock lands on the pinned D4/D2
    // rows (churn tail + input-OOB), never Unexpected.
    ("cldsd-imc", "var c0 = 0u64\nvar k0 = 0u64\nwhile k0 < 8 { c0 = c0 * 256 + (byte(data, k0 + 0) as u64)\nk0 += 1 }\nvar c1 = 0u64\nvar k1 = 0u64\nwhile k1 < 8 { c1 = c1 * 256 + (byte(data, k1 + 8) as u64)\nk1 += 1 }\nvar c2 = 0u64\nvar k2 = 0u64\nwhile k2 < 8 { c2 = c2 * 256 + (byte(data, k2 + 16) as u64)\nk2 += 1 }\nvar c9 = (byte(data, 0) as u64)\nvar s0 = buf_right(4, buf_left(1, data))\nvar s1 = buf_left(1, s0)\nvar acc = 0u64\nvar t0 = clamp(((((c2) * (c9)) + (min((c9), (acc))))), ((((((c1) ^ (c1))) != ((((size(s1) as u64)) * (acc)))) as u64)), ((min(((((c9) as u32) as u64)), (choose(((c0) == (c0)), 255u64, ((c0) + ((size(s0) as u64)))))))))\nvar cb = buf_left(200, data)\nvar ci = 0u64\nwhile ci < 30 { cb = cb ++ buf_left(200, data)\nci += 1 }", &IN16),
    // P3-IDIOMS M3-b: fused BE-2 byte-assembly group (the hacd200 :629
    // shape) — detector = decoded lhu (s:byte-fuse-wide), all modes
    ("byte-fuse-wide", "var w = ((byte(data, 0u64) * 256u64) + byte(data, 1u64)) as u64\nreturn (w) as bytes", &IN16),
    // P3-PAIRPKG: the fused pair site (detector = IR sibling adjacency,
    // s:pair-fuse-sd) — in-body var-index pair + a length fold
    ("pair-fuse-sd", "var l = [1u64, 2u64, 3u64]\nvar i = ((byte(data, 0) as u64) % 3u64)\nremove(l, i)\ninsert(l, i, ((byte(data, 1) as u64) + 9u64))\nreturn (((l[0] + l[1]) + (l[2] + (length(l) as u64)))) as bytes", &IN16),
];

/// A >4 KiB straight-line body: forces the jal-trampoline tier in every
/// mode (the A-19 bulk shape).
fn bulk_body(n: usize) -> String {
    let mut s = String::from("var acc = (byte(data, 0) as u64)\n");
    for k in 0..n {
        s.push_str(&format!("if (byte(data, 0) as u64) != {k}u64 {{ acc += {}u64 }}\n", k % 97));
    }
    s.push_str("return acc as bytes\n");
    s
}

/* ================================================================== */
/* Section 5: tests.                                                   */
/* ================================================================== */

/// The denominator table itself is pinned: count, uniqueness, unreachable
/// subset, per-dimension sizes.
#[test]
fn c3_denominator_pinned() {
    let items = denom_items();
    let set: BTreeSet<&str> = items.iter().copied().collect();
    assert_eq!(set.len(), items.len(), "denominator ids must be unique");
    assert_eq!(items.len(), DENOM_LEN, "DENOM_LEN pin");
    // per-dimension counts
    let ops = items.iter().filter(|i| i.starts_with("op:")).count();
    let guards = items.iter().filter(|i| i.starts_with("g:")).count();
    let tiers = items.iter().filter(|i| i.starts_with("t:")).count();
    let sites = items.iter().filter(|i| i.starts_with("s:")).count();
    let errs = items.iter()
        .filter(|i| i.starts_with("fe:") || i.starts_with("rx:") || i.starts_with("rt:"))
        .count();
    println!(
        "DENOM total {DENOM_LEN} = ops {ops} (76×3) + guards {guards} + tiers {tiers} + sites {sites} + error-paths {errs}"
    );
    assert_eq!(ops, 76 * 3);
    assert_eq!(guards, GUARD_CLASSES.len());
    assert_eq!(tiers, 27);
    assert_eq!(errs, 17 + 9 + 2);
    for (id, arg) in UNREACHABLE.iter() {
        assert!(set.contains(id), "unreachable id {id} not in denominator");
        assert!(arg.len() > 40, "unreachable {id} needs an argument");
    }
    assert_eq!(UNREACHABLE.len(), 9, "unreachable registry pin (POP×3, pool-in-Im/Imc×6; t:cldsd:Imc refuted by the P2-C3 corpus, op:IRLIST×3 refuted by P3-V2IMPL list shapes — both moved to hit-able)");
}

/// One four-way knock + guard attribution, with a per-probe expected
/// verdict family. Returns the observed verdict.
fn run_probe(hits: &mut Hits, body: &str, input: &[u8], label: &str) -> Verdict {
    run_probe_feat(hits, body, input, label, Feat::default())
}

fn run_probe_feat(
    hits: &mut Hits,
    body: &str,
    input: &[u8],
    label: &str,
    feat: Feat,
) -> Verdict {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{label}: interp build: {e}"));
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{label}: rv64 build: {e}"));
    let v = measure_knock(hits, &rv, &it, &feat, input, label);
    if let Err(e) = measure_compile(hits, body, label) {
        panic!("{label}: measure_compile: {e}");
    }
    v
}

/// The full targeted-probe execution: directed generation covers every
/// non-corpus-guaranteed denominator item (guards, tiers incl. trampoline,
/// sites, error paths, every opcode shape). Single source of truth shared
/// by the always-on test and the feedback saturation run (P2-C3: the
/// feedback test previously re-ran a WEAK subset of this table — compile-
/// only opcode shapes, no feat-keyed guard attributions, no negative
/// tables — and its final equality failed on the omitted items).
fn run_targeted_probes(mut hits: &mut Hits) {
    {
    // ① opcode shapes — run every shape (the feedback loop would pull
    // these on demand; running all keeps the default tier exhaustive)
    for op in SUBSET_OPS.iter() {
        if UNREACHABLE.iter().any(|(u, _)| u == &format!("op:{op}:Im")) {
            continue; // POP: bytecodegen artifact, unreachable by argument
        }
        let body = opcode_probe(op).unwrap_or_else(|| panic!("no shape for {op}"));
        let it = Interp::try_build(body)
            .unwrap_or_else(|e| panic!("opcode {op} shape rejected by frontend: {e}"));
        let rv = Rv64::try_build(body)
            .unwrap_or_else(|e| panic!("opcode {op} shape rejected by rv64: {e}"));
        let feat = Feat::default();
        for input in [IN16.as_slice(), IN1.as_slice(), &[0u8; 32][..]] {
            let v = measure_knock(&mut hits, &rv, &it, &feat, input, &format!("opshape:{op}"));
            assert!(!v.is_unexpected(), "opcode {op} shape knock: {v:?} on {input:?}");
        }
        measure_compile(&mut hits, body, &format!("opshape:{op}"))
            .unwrap_or_else(|e| panic!("opcode {op}: {e}"));
    }
    // ③ guard probes with expected verdict families
    for (id, body, input, expect) in GUARD_PROBES.iter() {
        let mut feat = Feat::default();
        if *id == "g:arena-churn-d4" {
            feat.churn = true; // the classifier's D4 row keys on the feat
        }
        if *id == "g:and-chain-pure" {
            feat.and_chain = true; // attribution keys on the feat
        }
        let v = run_probe_feat(&mut hits, body, input, &format!("probe:{id}"), feat);
        let name = format!("{v:?}");
        // P3-V2IMPL list guard rows: DIRECT attribution — the 72/40/85
        // classes are shared with other families (byte params, Nil params,
        // user require values), so class-based guard_items_for attribution
        // would be ambiguous; the probe body carries exactly one list site
        // and the expected verdict pins it
        if matches!(
            *id,
            "g:list-oob-85" | "g:list-idx-72" | "g:list-cap-40" | "g:pair-oob-85" | "g:pair-idx-72"
        ) {
            assert!(
                expect.iter().any(|e| name.contains(e)),
                "{id}: verdict {name} not in {expect:?}"
            );
            hits.hit(id, &format!("probe:{id}"));
            continue;
        }
        // nil_plan consumer-arm EMIT attribution (all nil-* guards)
        if let Some(arm) = match *id {
            "g:nil-cmp-eq-90" => Some("s:nil-arm-eqneq"),
            "g:nil-cmp-ord-90" => Some("s:nil-arm-ord"),
            "g:int-nil-arith-71" => Some("s:nil-arm-arith"),
            "g:bytes-nil-91" => Some("s:nil-arm-bytes"),
            "g:nil-cbytes-71" => Some("s:nil-arm-cbytes"),
            "g:nil-xop-old-71" => Some("s:nil-arm-xopold"),
            "g:nil-param-72" => Some("s:nil-arm-param72"),
            "g:nil-reqvalue-76" => Some("s:nil-arm-req76"),
            _ => None,
        } {
            hits.hit(arm, &format!("probe:{id}"));
        }
        if *id == "g:nil-reqvalue-76" {
            assert!(name.contains("AgreeTrap76"), "{id}: {name}");
            assert!(hits.contains(id), "{id} not attributed");
            continue;
        }
        if *id == "g:nil-param-72" {
            assert!(name.contains("AgreeTrap72"), "{id}: {name}");
            assert!(hits.contains(id), "{id} not attributed");
            continue;
        }
        assert!(
            expect.iter().any(|e| name.contains(e)),
            "{id}: verdict {name} not in {expect:?}"
        );
        // message-collision case: the XOP-old Nil guard faults with the
        // SAME class+message as plain int-Nil arithmetic ("cannot cast Nil
        // to uint"); the site distinction is the probe body itself
        if *id == "g:nil-xop-old-71" {
            hits.hit(id, &format!("probe:{id}"));
        }
        assert!(hits.contains(id), "{id} not attributed");
    }
    // ⑤ site/tier probes
    for (id, body, input) in SITE_PROBES.iter() {
        match *id {
            "bigout-64" => {
                let rv = Rv64::try_build_out(body, 64, 0x7000)
                    .unwrap_or_else(|e| panic!("{id}: {e}"));
                let it = Interp::try_build(body).unwrap();
                let feat = Feat::default();
                for inp in [input, &[0xA5u8; 63][..], &[0xA5u8; 65][..]] {
                    let v = measure_knock(&mut hits, &rv, &it, &feat, inp, &format!("probe:{id}"));
                    assert!(!v.is_unexpected(), "{id}: {v:?}");
                }
                hits.hit("s:output-copy-wide", &format!("probe:{id}"));
                measure_compile(&mut hits, body, &format!("probe:{id}")).unwrap();
            }
            "needs-zero" => {
                let v = run_probe(&mut hits, body, input, &format!("probe:{id}"));
                assert!(!v.is_unexpected(), "{id}: {v:?}");
                hits.hit("s:needs-zero", &format!("probe:{id}"));
            }
            "choose-bytes-bug017" => {
                // the pinned divergence: interp Ok, rv64 71 — see the pin
                // test for the mechanism; here we only record the EMIT hit
                let it = Interp::try_build(body).unwrap();
                let rv = Rv64::try_build(body).unwrap();
                let _ = measure_knock(&mut hits, &rv, &it, &Feat::default(), input, "probe:bug017");
                measure_compile(&mut hits, body, "probe:bug017").unwrap();
                for m in ["Im", "Imc", "ImcRemap"] {
                    hits.hit(&format!("s:choose-eager-bytes:{m}"), "probe:bug017");
                }
            }
            "join-bytes" => {
                let v = run_probe(&mut hits, body, input, &format!("probe:{id}"));
                assert!(!v.is_unexpected(), "{id}: {v:?}");
                let mut feat = Feat::default();
                feat.bytes_if_arm = true;
                let it = Interp::try_build(body).unwrap();
                let rv = Rv64::try_build(body).unwrap();
                let _ = measure_knock(&mut hits, &rv, &it, &feat, input, "probe:join-bytes");
            }
            _ => {
                let v = run_probe(&mut hits, body, input, &format!("probe:{id}"));
                assert!(!v.is_unexpected(), "{id}: {v:?}");
            }
        }
    }
    // trampoline tier: >4KiB straight-line body (stays under the 16383 B
    // mirror), all modes
    {
        let body = bulk_body(300);
        let rv = Rv64::try_build(&body).unwrap();
        let it = Interp::try_build(&body).unwrap();
        let v = measure_knock(&mut hits, &rv, &it, &Feat::default(), &IN16, "probe:bulk800");
        assert!(!v.is_unexpected(), "bulk800: {v:?}");
        measure_compile(&mut hits, &body, "probe:bulk300").unwrap();
    }
    // ⑥ error paths: every negative-suite body through both channels
    for (name, body) in frontend_negative_cases() {
        let e1 = Interp::try_build(&body)
            .err()
            .unwrap_or_else(|| panic!("fe:{name} must frontend-reject"));
        let e2 = Rv64::try_build(&body).err().unwrap();
        println!("fe:{name}: {}", first_line(&e1));
        let _ = e2;
        hits.hit(&format!("fe:{name}"), &format!("neg:{name}"));
    }
    for (name, body, frags, interp_runs) in rv64_negative_cases() {
        let it = Interp::try_build(&body)
            .unwrap_or_else(|e| panic!("rx:{name}: frontend must accept: {e}"));
        if interp_runs {
            let _ = it.run(&IN1); // acceptance proof (any outcome)
        }
        let err = Rv64::try_build(&body)
            .err()
            .unwrap_or_else(|| panic!("rx:{name}: rv64 must reject"));
        assert!(
            frags.iter().any(|f| err.contains(f)),
            "rx:{name}: {err:?} must mention {frags:?}"
        );
        hits.hit(&format!("rx:{name}"), &format!("neg:{name}"));
    }
    // rx:mirror-overflow — a body over the 16383 B image budget
    {
        let body = bulk_body(2200);
        let err = Rv64::try_build(&body)
            .err()
            .expect("2200-stmt body must overflow the mirror");
        assert!(err.contains("calcfunc size overflow"), "{err}");
        hits.hit("rx:mirror-overflow", "neg:mirror-overflow");
    }
    // anchor kernels (compile-side): the HIP5 transcripts carry the
    // banked/pool/merge/cldsd site mixes real kernels produce
    let score_sum = include_str!("fitsh_kernels_score_sum.inc");
    let hacd200 = include_str!("fitsh_kernels_hacd200.inc");
    measure_compile(&mut hits, score_sum, "anchor:score_sum").unwrap();
    measure_compile(&mut hits, hacd200, "anchor:hacd200").unwrap();
    // mini-corpus sweep (40 cases of the smoke domain): corpus-dependent
    // mix items (compressed frame traffic shapes etc.) + zero-unexpected
    for i in 0..40u64 {
        let bad = measure_case(&mut hits, 0x5100_0000_0000_0001u64.wrapping_add(i), 8, "mini40");
        assert!(bad.is_empty(), "mini-corpus unexpected: {bad:?}");
    }
    }
}

/// The always-on targeted-probe tier (G2.4's executable half): runs the
/// full shared probe table and pins the saturation equality.
#[test]
fn c3_targeted_probes_cover_denominator() {
    let mut hits = Hits::new();
    run_targeted_probes(&mut hits);
    // saturation check: every item hit or unreachable-with-argument
    let items = denom_items();
    let missing: Vec<&str> = items
        .iter()
        .copied()
        .filter(|i| !hits.contains(i) && !UNREACHABLE.iter().any(|(u, _)| u == i))
        .collect();
    println!(
        "TARGETED-PROBE COVERAGE: {}/{} of denominator (unreachable {})",
        hits.map.len(),
        DENOM_LEN,
        UNREACHABLE.len()
    );
    for (k, v) in hits.map.iter() {
        println!("    {k} <- {v}");
    }
    let denom_set: BTreeSet<&str> = items.iter().copied().collect();
    let extra: Vec<&String> =
        hits.map.keys().filter(|k| !denom_set.contains(k.as_str())).collect();
    assert!(extra.is_empty(), "hits contain non-denominator items (detector overreach): {extra:?}");
    assert!(missing.is_empty(), "targeted probes left unhit: {missing:?}");
    assert_eq!(
        hits.map.len() + UNREACHABLE.len(),
        DENOM_LEN,
        "hits + unreachable must equal the denominator exactly"
    );
}

/// G2.3 first-light: the PRE-EXTENSION-equivalent measurement over the
/// existing default corpora (smoke-24 + campaign-150 seed domains) plus
/// the anchor kernels — the "before" coverage snapshot. Informational
/// printout; the saturation obligation lives in the tests above.
#[test]
fn c3_coverage_first_light() {
    let mut hits = Hits::new();
    // smoke-24 domain
    let bad1: Vec<String> = {
        let mut b = Vec::new();
        for i in 0..24u64 {
            b.extend(measure_case(&mut hits, 0x5100_0000_0000_0001u64.wrapping_add(i), 8, "smoke"));
        }
        b
    };
    // campaign-150 domain
    let bad2: Vec<String> = {
        let mut b = Vec::new();
        for i in 0..150u64 {
            b.extend(measure_case(
                &mut hits,
                0xC2DE_0000_0000_0001u64.wrapping_add(i),
                10,
                "campaign150",
            ));
        }
        b
    };
    assert!(bad1.is_empty(), "smoke measurement unexpected: {bad1:?}");
    assert!(bad2.is_empty(), "campaign measurement unexpected: {bad2:?}");
    // anchor kernels (compile-side items only; behavior is pinned by g6)
    let score_sum = include_str!("fitsh_kernels_score_sum.inc");
    let hacd200 = include_str!("fitsh_kernels_hacd200.inc");
    measure_compile(&mut hits, score_sum, "anchor:score_sum")
        .unwrap_or_else(|e| panic!("score_sum: {e}"));
    measure_compile(&mut hits, hacd200, "anchor:hacd200")
        .unwrap_or_else(|e| panic!("hacd200: {e}"));
    let items = denom_items();
    let unhit: Vec<&str> = items
        .iter()
        .copied()
        .filter(|i| !hits.contains(i) && !UNREACHABLE.iter().any(|(u, _)| u == i))
        .collect();
    println!(
        "FIRST LIGHT (smoke24 + campaign150 + anchors): {}/{} hit (+{} unreachable), {} unhit",
        hits.map.len(),
        DENOM_LEN,
        UNREACHABLE.len(),
        unhit.len()
    );
    for u in unhit.iter() {
        println!("    UNHIT {u}");
    }
}

/// BUG-017 (FIXED 2026-11-15, P2-C3FIX; found by the P2-C3 B1 probe):
/// the EAGER CHOOSE selection (rv64.rs `CHOOSE =>` arm, the D9-fix path)
/// popped arm values top-first (`for _ in 0..vals { no_regs.push(pop()) }`)
/// and copied `dests[i] <- no_regs[i]` / `yes_regs[i]` WITHOUT reversing
/// pairs — a BYTES arm's (ptr, len) pair was collected (len, ptr) and
/// copied swapped: the consumer read ptr=len-value / len=ptr-value. The
/// value-position `if` (IRIFR) join had been fixed for exactly this in
/// BUG-011 (take_arm_values + bottom-first re-push); the eager-CHOOSE
/// selection kept the reversed pops.
///
/// Pre-fix behavior (pinned evidence, deterministic, all profiles):
/// interpreter returned the taken bytes; rv64 {Im, Imc, ImcRemap} all
/// trapped 71 (the swapped len exceeded output_max at the return copy).
/// Facets of the same arm-count-from-suby assumption:
///   (a) bytes×bytes: pair swap (this pin);
///   (b) choose(c, int, bytes): rejected LATE with "value stack not empty
///       at function end" (a leaked vstack entry);
///   (c) choose(c, bytes, int): `vstack.len() - 1 - 2*vals` underflowed —
///       PANIC in debug AND release (never silently lowered).
///
/// Fix (rv64.rs CHOOSE arm): each arm's values are collected bottom-first
/// through take_arm_values (the BUG-011 treatment — Refs materialize into
/// their own consecutive pool positions) and the arm result types are
/// validated up front with a clean reject (the vstack is statically typed
/// per position; mixed int/bytes arms are unrepresentable). Post-fix:
/// three modes == interpreter on the taken arm; both mixed orders reject
/// early with the arm-type text. CHOOSE bytes arms STAY OUT of the
/// generator face (constraint ⑥ re-frozen pending fix soak — this task
/// does not touch the generation face).
#[test]
fn c3_pin_bug017_choose_bytes_pair_swap() {
    let body = "var c = (byte(data, 0) as u64)\nvar r = choose(((c % 2u64) == 0u64), buf_left(2, data), buf_right(3, data))\nreturn r\n";
    let inp: [u8; 16] = core::array::from_fn(|i| i as u8);
    let it = Interp::try_build(body).expect("interp builds");
    let rv = Rv64::try_build(body).expect("rv64 lowers (the selection EMITS)");
    let ir = it.run(&inp);
    let rr = rv.run(&inp);
    // interpreter: Ok(even→first 2 bytes / odd→last 3 bytes)
    let want = if inp[0] % 2 == 0 { vec![0u8, 1] } else { vec![13u8, 14, 15] };
    assert_eq!(ir.unwrap(), want, "interp returns the taken arm");
    for (m, r) in rr.iter() {
        // FIX FLIP: the pair now lands (ptr, len) — three modes == interp
        assert_eq!(
            r.as_ref(),
            Ok(&want),
            "BUG-017 fixed: {m:?} must return the taken arm, got {r:?}"
        );
    }
    // facet (c): bytes-first mixed choose — clean early reject (was an
    // index-underflow PANIC in both profiles)
    let bad = "var c = (byte(data, 0) as u64)\nvar q = choose(((c % 2u64) == 0u64), buf_left(3, data), 9u64)\nreturn (c) as bytes\n";
    let (bir, _) = frontend(bad).expect("frontend");
    let bir = unsafe {
        std::mem::transmute::<&vm::ir::IRNodeArray, &'static vm::ir::IRNodeArray>(&bir)
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lower_body_mode_arena(bir, true, Rv64Mode::Im, ARENA_SIZE_MAX)
            .err()
            .map(|e| first_line(&format!("{e}")).to_string())
            .unwrap_or_else(|| "LOWERED".to_string())
    }));
    match outcome {
        Err(_) => panic!("facet (c) PANICKED post-fix — must reject cleanly"),
        Ok(s) => assert!(
            s.contains("choose arms must have the same result type"),
            "facet (c) must reject with the arm-type text, got: {s}"
        ),
    }
    // facet (b): int-first mixed choose — the same early reject (was a
    // late "value stack not empty" leak reject)
    let clean = "var c = (byte(data, 0) as u64)\nvar q = choose(((c % 2u64) == 0u64), 1u64, buf_left(3, data))\nreturn (c) as bytes\n";
    let err = match Rv64::try_build(clean) {
        Err(e) => e,
        Ok(_) => panic!("int-first mixed must reject"),
    };
    assert!(
        err.contains("choose arms must have the same result type"),
        "{err}"
    );
}

/// BUG-018 (FIXED 2026-11-15, P2-C3FIX; found by C3 opcode-shape
/// instrumentation 2026-11-14): a CONSTANT-rhs `+=`/`-=` XOP store went to
/// memory through the dedicated scratch (ld sc1 / addi / sd) and left any
/// live forward record for the slot untouched — the next empty-stack GET
/// read the stale register. Pre-fix behavior (pinned evidence):
///   interp Ok(1) / Im Ok(0) / Imc Ok(0) / ImcRemap Ok(1)
/// (deterministic miscompile; ImcRemap's register-homed slots never take
/// the memory path). The generated corpus was immune BY SHAPE — every
/// statement-level PUT or label placement (`place()` wipes fwd_*) evicts
/// the record, which is why the 1200-case bombardments stayed green.
///
/// Fix (rv64.rs XOP constant arm): the store invalidates the slot's
/// forward record (sc1 is never a pool home, so there is nothing to
/// cache). Post-fix: all four engines agree on every facet below.
///
/// Contrast forms (over-narrow-fix guards; both verified BEFORE the fix):
///  * operand-position read in the NEXT statement (`var s = (a + 1u64)`) —
///    diverged pre-fix exactly like the immediate read (the operand GET
///    hits the record before the statement's own pushes evict it);
///  * loop-interior read — agreed pre-fix (IRWHILE `place()` wipes the
///    records at the loop labels) and must stay agreeing post-fix.
#[test]
fn c3_pin_bug018_xop_const_forward() {
    let inp: Vec<u8> = (0..16u8).collect();
    let inp5: Vec<u8> = (5..21u8).collect();
    // add facet: `a += 1` immediately read — all four engines agree
    {
        let add = "var a = (byte(data,0) as u64)\na += 1u64\nreturn (a) as bytes";
        let it = Interp::try_build(add).unwrap();
        let rv = Rv64::try_build(add).unwrap();
        let want = it.run(&inp).unwrap();
        assert_eq!(want, vec![0, 0, 0, 0, 0, 0, 0, 1], "interp sanity (add)");
        for (m, r) in rv.run(&inp) {
            assert_eq!(r.as_ref(), Ok(&want), "BUG-018 add facet fixed, {m:?}");
        }
    }
    // sub facet: `a -= 1` immediately read — all four engines agree
    {
        let sub = "var a = (byte(data,0) as u64)\na -= 1u64\nreturn (a) as bytes";
        let it = Interp::try_build(sub).unwrap();
        let rv = Rv64::try_build(sub).unwrap();
        let want = it.run(&inp5).unwrap();
        assert_eq!(want, vec![0, 0, 0, 0, 0, 0, 0, 4], "interp sanity (sub)");
        for (m, r) in rv.run(&inp5) {
            assert_eq!(r.as_ref(), Ok(&want), "BUG-018 sub facet fixed, {m:?}");
        }
    }
    // contrast 1: operand-position read in the NEXT statement (diverged
    // pre-fix: interp 2 / Im 1 / Imc 1 / ImcRemap 2)
    {
        let body =
            "var a = (byte(data,0) as u64)\na += 1u64\nvar s = (a + 1u64)\nreturn (s) as bytes";
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
        let want = it.run(&inp).unwrap();
        assert_eq!(want[7], 2, "interp sanity (operand read)");
        for (m, r) in rv.run(&inp) {
            assert_eq!(r.as_ref(), Ok(&want), "BUG-018 operand-read contrast, {m:?}");
        }
    }
    // contrast 2: loop-interior read (agreed pre-fix via place() eviction;
    // guards against an over-broad re-record of the scratch register)
    {
        let body = "var a = (byte(data,0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile i < 3 { a += 1u64\ns += a\ni += 1 }\nreturn (s) as bytes";
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
        let want = it.run(&inp).unwrap();
        for (m, r) in rv.run(&inp) {
            assert_eq!(r.as_ref(), Ok(&want), "BUG-018 loop-read contrast, {m:?}");
        }
    }
    // non-triggering controls (must be CORRECT everywhere)
    for body in [
        "var a = (byte(data,0) as u64)\na *= 2u64\nreturn (a) as bytes",
        "var a = (byte(data,0) as u64)\nvar b = 1u64\na += b\nreturn (a) as bytes",
        "var a = (byte(data,0) as u64)\nvar i = 0u64\nwhile i < 3 { a += 1u64\ni += 1 }\nreturn (a) as bytes",
        "var a = (byte(data,0) as u64)\nvar b = (byte(data,1) as u64)\na += 1u64\nb += a\nreturn ((a + b)) as bytes",
    ] {
        let it = Interp::try_build(body).unwrap();
        let rv = Rv64::try_build(body).unwrap();
        let want = it.run(&inp).unwrap();
        for (m, r) in rv.run(&inp) {
            assert_eq!(r.as_ref().ok(), Some(&want), "control body must agree, {m:?}: {body}");
        }
    }
}


/// BUG-019 (FIXED 2026-11-15, P2-C3FIX; found by the C3 feedback
/// saturation corpus 2026-11-15, seed 0xC3FB_0000_0000_041F input#2 —
/// loopmaze family, 1200-case bombard domains did not reach it):
/// interpreter Ok, all three rv64 modes trapped 71 on a live value.
///
/// Mechanism (decode-verified, doc 71 E5): mid-loop `var` decls make the
/// choose's YES arm base slot (t4) may-Nil at the choose point (IRWHILE
/// fixpoint entry = pre ∪ body), while the NO arm base (t0, declared
/// unconditionally before the loop) is proven non-Nil. In the eager-CHOOSE
/// lowering both tag-copy paths were gated `if my || mn` (no-path /
/// yes-path) — the no-path copy ran on `my` even though the no-arm's own
/// evaluation never wrote its vstack-depth tag byte (mn=false ⇒ no
/// machinery ⇒ stale prologue zero = "Nil"), and the consumer's Nil guard
/// (lbu tag; beq → fault 71) then faulted on the selected live value.
///
/// Fix (rv64.rs CHOOSE arm): the no-path copy is gated on `mn` and the
/// yes-path copy on `my` (independent predicates) — and a mechanism-less
/// arm publishes a non-Nil tag byte (1) on its select path instead (the
/// complementary quadrant mn-only/taken-yes ALSO diverged pre-fix — the
/// shared gate copied the YES arm's stale byte — so the gates alone are
/// not enough; the dest tag must be valid on every selectable path when
/// my||mn). With my=mn=false nothing is emitted (no guard can read the
/// tag) — all-non-Nil shapes stay byte-identical (census pins).
/// Post-fix: the repro agrees (interp Ok(empty) == rv64 Ok(empty)), and
/// the four-quadrant × four-consumer matrix
/// (`c3_pin_bug019_nil_tag_quadrants`) agrees everywhere.
#[test]
fn c3_pin_bug019_choose_stale_nil_tag() {
    let body = "var c0 = 0u64\nvar k0 = 0u64\nwhile k0 < 8 { c0 = c0 * 256 + (byte(data, k0 + 0) as u64)\nk0 += 1 }\nvar c1 = 0u64\nvar k1 = 0u64\nwhile k1 < 8 { c1 = c1 * 256 + (byte(data, k1 + 8) as u64)\nk1 += 1 }\nvar s0 = buf_right_drop(0, buf_left(6, data))\nvar acc = 0u64\nvar t0 = 0u64\nvar t1 = 0u64\nvar t2 = 0u64\nwhile t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\nvar t4 = 0u64\nwhile t4 < 3 { t4 += 1\nif ((255u64) != (18446744073709551613u64)) { t4 += 1\ncontinue }\n}\n}\nacc = (acc ^ ((max((c0), ((size(s0) as u64))) + choose(((t1) == (4u64)), t4, ((255u64) + (t0))))))\n}\n";
    let inp: Vec<u8> = vec![0u8; 16];
    let it = Interp::try_build(body).expect("interp builds");
    let rv = Rv64::try_build(body).expect("rv64 lowers");
    let ir = it.run(&inp);
    let want_interp: Vec<u8> = vec![];
    assert_eq!(ir.unwrap(), want_interp, "interp: fall-off-end → empty output");
    for (m, r) in rv.run(&inp) {
        // FIX FLIP: the stale-tag copy is gone — three modes == interp
        assert_eq!(
            r.as_ref(),
            Ok(&want_interp),
            "BUG-019 fixed: {m:?} must agree (empty output), got {r:?}"
        );
    }
    // control 1: hoisted decls — no may-Nil arms → no tag copies → agree
    let hoist = "var c0 = 0u64\nvar k0 = 0u64\nwhile k0 < 8 { c0 = c0 * 256 + (byte(data, k0 + 0) as u64)\nk0 += 1 }\nvar c1 = 0u64\nvar k1 = 0u64\nwhile k1 < 8 { c1 = c1 * 256 + (byte(data, k1 + 8) as u64)\nk1 += 1 }\nvar s0 = buf_right_drop(0, buf_left(6, data))\nvar acc = 0u64\nvar t0 = 0u64\nvar t1 = 0u64\nvar t2 = 0u64\nvar t3 = 0u64\nvar t4 = 0u64\nwhile t2 < 2 { t2 += 1\nwhile t3 < 1 { t3 += 1\nwhile t4 < 3 { t4 += 1\nif ((255u64) != (18446744073709551613u64)) { t4 += 1\ncontinue }\n}\n}\nacc = (acc ^ ((max((c0), ((size(s0) as u64))) + choose(((t1) == (4u64)), t4, ((255u64) + (t0))))))\n}";
    let it2 = Interp::try_build(hoist).unwrap();
    let rv2 = Rv64::try_build(hoist).unwrap();
    let want2 = it2.run(&inp).unwrap();
    for (m, r) in rv2.run(&inp) {
        assert_eq!(r.as_ref(), Ok(&want2), "control hoist must agree, {m:?}");
    }
    // control 2: no choose → agree
    let noch = body.replace("choose(((t1) == (4u64)), t4, ((255u64) + (t0)))", "t4");
    let it3 = Interp::try_build(&noch).unwrap();
    let rv3 = Rv64::try_build(&noch).unwrap();
    let want3 = it3.run(&inp).unwrap();
    for (m, r) in rv3.run(&inp) {
        assert_eq!(r.as_ref(), Ok(&want3), "control no-choose must agree, {m:?}");
    }
}

/// BUG-019 boundary matrix: the (my, mn) quadrant × Nil-consumer forms,
/// all four engines agree per input (value-by-value cross-check). The arms are BARE GETs of
/// the base slots so the may-Nil verdicts are exactly (my, mn); the cond
/// reads data byte 15 so both arms are taken across the two inputs (byte
/// 15 == 4 → yes arm, else no arm). The base slot of a quadrant's
/// "may-Nil" arm is declared INSIDE the nested (middle) loop with the
/// choose AFTER it — the loop-header fixpoint state (where the slot is
/// still unassigned) joins into the after-state, so the GET is may-Nil
/// there; a "proven non-Nil" arm's base is declared before the loop.
///
/// Pre-fix divergences (measured, all three modes, before P2-C3FIX):
///   * my-only, taken=no: Err(71)/Err(90)/Err(90)/Err(71) for
///     arith/eqneq/ord/cbytes — the BUG-019 shared-gate face;
///   * mn-only, taken=yes: the SAME four codes — the complementary
///     shared-gate face (the yes-path copied the yes arm's stale byte).
///     This quadrant is why the fix needs the non-Nil tag publication and
///     not just the gate split.
///   * both-true and neither-true agreed pre-fix (real copies both sides /
///     no guards at all) — kept as regression watches.
#[test]
fn c3_pin_bug019_nil_tag_quadrants() {
    let inputs = [inp15(4), inp15(0)];
    for (my, mn) in [(true, false), (false, true), (true, true), (false, false)] {
        for consumer in ["arith", "eqneq", "ord", "cbytes"] {
            let body = quad19_body(my, mn, consumer);
            let it = Interp::try_build(&body)
                .unwrap_or_else(|e| panic!("q19 my={my} mn={mn} {consumer}: interp build: {e}"));
            let rv = Rv64::try_build(&body)
                .unwrap_or_else(|e| panic!("q19 my={my} mn={mn} {consumer}: rv64 build: {e}"));
            for (k, input) in inputs.iter().enumerate() {
                let want = it
                    .run(input)
                    .unwrap_or_else(|f| panic!("q19 my={my} mn={mn} {consumer} input#{k}: interp must be Ok: {:?}", f));
                for (m, r) in rv.run(input) {
                    assert_eq!(
                        r.as_ref(),
                        Ok(&want),
                        "q19 my={my} mn={mn} {consumer} input#{k} (taken={}) must agree, {m:?}: got {r:?}",
                        if k == 0 { "yes" } else { "no" }
                    );
                }
            }
        }
    }
}

/// Build a BUG-019 quadrant body: `my_mid`/`mn_mid` place the yes/no arm's
/// base slot declaration inside the nested loop (may-Nil) or before it
/// (proven non-Nil); `consumer` selects how the choose result is consumed
/// (the interpreter's Nil-consumer fault classes: arith/cbytes → 71,
/// eqneq/ord → 90).
fn quad19_body(my_mid: bool, mn_mid: bool, consumer: &str) -> String {
    let mut b = String::from("var t1 = 0u64\nvar t2 = 0u64\nvar acc = 0u64\n");
    if !my_mid {
        b.push_str("var t4 = 0u64\n");
    }
    if !mn_mid {
        b.push_str("var t0 = 0u64\n");
    }
    b.push_str("while t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\n");
    if my_mid {
        b.push_str("var t4 = 0u64\n");
    }
    if mn_mid {
        b.push_str("var t0 = 0u64\n");
    }
    b.push_str("while t4 < 3 { t4 += 1 }\n}\n");
    let ch = "choose(((byte(data,15) as u64) == 4u64), t4, t0)";
    let line = match consumer {
        // arith on a may-Nil operand → CastFail(71) guard
        "arith" => format!("acc = (acc + ({ch}))"),
        // EQ with exactly-one may-Nil side → 90 guard
        "eqneq" => format!("if (({ch}) == (0u64)) {{ acc = (acc + 1u64) }}"),
        // ordered comparison on a may-Nil operand → 90 guard
        "ord" => format!("if (({ch}) < (8u64)) {{ acc = (acc + 1u64) }}"),
        // `as bytes` on a may-Nil operand → CastFail(71) guard
        "cbytes" => format!("acc = (acc + (size(({ch}) as bytes) as u64))"),
        _ => unreachable!(),
    };
    b.push_str(&line);
    b.push_str("\n}\nreturn (acc) as bytes");
    b
}

fn inp15(b15: u8) -> Vec<u8> {
    let mut v = vec![0u8; 16];
    v[15] = b15;
    v
}

/// BUG-020 (FIXED 2026-11-15, P2-C3FIX-BUG020; found 2026-11-15 during the
/// P2-C3FIX BUG-019 quadrant construction — the IRIFR join had the same
/// stale-tag exposure the eager-CHOOSE select had): the value-position
/// `if` (IRIFR, rv64.rs `IRIFR =>` arm) compiles BOTH arms at the SAME
/// vstack depth and the join had NO tag materialization at all — the tag
/// byte at the join depth was whatever the TAKEN arm wrote. A
/// mechanism-less arm (proven non-Nil, e.g. a faulting-op computation
/// like `255u64 + t0`) writes NO tag byte, so when it was taken the
/// consumer's Nil guard read the stale byte (prologue zero = "Nil") and
/// invented a 71.
///
/// Pre-fix behavior (pinned evidence, deterministic, all three modes;
/// re-verified by the pre-fix scratch before the fix): with the may-Nil
/// bare-GET arm as the sibling, BOTH orderings diverged when the
/// mechanism-less arm was taken —
///   * mechanism-less arm = YES: input byte15=4 → interp Ok, rv64 Err(71);
///   * mechanism-less arm = ELSE: input byte15=0 → interp Ok, rv64 Err(71).
///
/// Fix (rv64.rs IRIFR arm): per-path non-Nil publication — each path
/// emits `nil_set_tag_one` at the join positions exactly when ITS arm is
/// mechanism-less AND the other arm makes the join may-Nil (yes path:
/// `mn && !my`; no path: `my && !mn`). Unlike the CHOOSE select there is
/// NO copy arm: both IRIFR arms share the SAME result cells (the join
/// re-push lands the value where the taken arm's own pushes sat — the
/// debug_assert pins a_locs == b_locs), so a may-Nil arm's own mechanism
/// already published the real tag exactly where the consumer reads it;
/// my=mn=false emits nothing (no guard can read the tag — all-non-Nil
/// census shapes stay byte-identical). The (my, mn) quadrant × consumer
/// matrix is pinned in `c3_pin_bug020_nil_tag_quadrants`.
#[test]
fn c3_pin_bug020_irifr_join_stale_tag() {
    let inputs = [inp15(4), inp15(0)];
    // shape A: mechanism-less arm = the ELSE arm → pre-fix input#1 trapped
    let a = "var t0 = 0u64\nvar t2 = 0u64\nvar acc = 0u64\nwhile t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\nvar t4 = 0u64\nwhile t4 < 3 { t4 += 1 }\n}\nacc = (acc + (if ((byte(data,15) as u64) == 4u64) { t4 } else { (255u64 + t0) }))\n}\nreturn (acc) as bytes";
    let it = Interp::try_build(a).expect("interp builds");
    let rv = Rv64::try_build(a).expect("rv64 lowers");
    for (k, input) in inputs.iter().enumerate() {
        let want = it.run(input).unwrap();
        for (m, r) in rv.run(input) {
            // FIX FLIP: the mechanism-less path now publishes non-Nil
            assert_eq!(
                r.as_ref(),
                Ok(&want),
                "BUG-020 A fixed: input#{k} (taken={}) must agree, {m:?}: got {r:?}",
                if k == 0 { "yes" } else { "no" }
            );
        }
    }
    // shape B: mechanism-less arm = the YES arm → pre-fix input#0 trapped
    let b = "var t0 = 0u64\nvar t2 = 0u64\nvar acc = 0u64\nwhile t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\nvar t4 = 0u64\nwhile t4 < 3 { t4 += 1 }\n}\nacc = (acc + (if ((byte(data,15) as u64) == 4u64) { (255u64 + t0) } else { t4 }))\n}\nreturn (acc) as bytes";
    let it2 = Interp::try_build(b).expect("interp builds");
    let rv2 = Rv64::try_build(b).expect("rv64 lowers");
    for (k, input) in inputs.iter().enumerate() {
        let want = it2.run(input).unwrap();
        for (m, r) in rv2.run(input) {
            assert_eq!(
                r.as_ref(),
                Ok(&want),
                "BUG-020 B fixed: input#{k} (taken={}) must agree, {m:?}: got {r:?}",
                if k == 0 { "yes" } else { "no" }
            );
        }
    }
    // agreeing control: BOTH arms carry the may-Nil GET (each taken path
    // writes a real tag at the join depth) — must stay agreeing
    let ctrl = "var t2 = 0u64\nvar acc = 0u64\nwhile t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\nvar t4 = 0u64\nwhile t4 < 3 { t4 += 1 }\n}\nacc = (acc + (if ((byte(data,15) as u64) == 4u64) { (t4 * 2u64) } else { t4 }))\n}\nreturn (acc) as bytes";
    let it3 = Interp::try_build(ctrl).expect("interp builds");
    let rv3 = Rv64::try_build(ctrl).expect("rv64 lowers");
    for input in inputs.iter() {
        let want = it3.run(input).unwrap();
        for (m, r) in rv3.run(input) {
            assert_eq!(r.as_ref(), Ok(&want), "BUG-020 control must agree, {m:?}");
        }
    }
}

/// BUG-020 boundary matrix: the (my, mn) quadrant × Nil-consumer forms on
/// the value-position `if` (IRIFR), all four engines agree per input
/// (value-by-value cross-check). Arms are BARE GETs of the base slots so the may-Nil
/// verdicts are exactly (my, mn); the cond reads data byte 15 so both
/// arms are taken across the two inputs (byte 15 == 4 → yes arm, else no
/// arm); a quadrant's "may-Nil" base is declared INSIDE the nested loop
/// with the value-if AFTER it (the loop-header fixpoint state joins into
/// the after-state), a "proven non-Nil" base is declared before the loop.
///
/// Pre-fix divergences (measured by the pre-fix scratch, all three
/// modes, before P2-C3FIX-BUG020): the mechanism-less arm's taken path
/// faulted — my-only taken=no and mn-only taken=yes → Err(71)/Err(90)/
/// Err(71) for arith/eqneq/cbytes; both-true and neither-true agreed
/// (real tags both sides / no guards at all) — kept as regression
/// watches.
#[test]
fn c3_pin_bug020_nil_tag_quadrants() {
    let inputs = [inp15(4), inp15(0)];
    for (my, mn) in [(true, false), (false, true), (true, true), (false, false)] {
        for consumer in ["arith", "eqneq", "cbytes"] {
            let body = quad20_body(my, mn, consumer);
            let it = Interp::try_build(&body)
                .unwrap_or_else(|e| panic!("q20 my={my} mn={mn} {consumer}: interp build: {e}"));
            let rv = Rv64::try_build(&body)
                .unwrap_or_else(|e| panic!("q20 my={my} mn={mn} {consumer}: rv64 build: {e}"));
            for (k, input) in inputs.iter().enumerate() {
                let want = it.run(input).unwrap_or_else(|f| {
                    panic!("q20 my={my} mn={mn} {consumer} input#{k}: interp must be Ok: {f:?}")
                });
                for (m, r) in rv.run(input) {
                    assert_eq!(
                        r.as_ref(),
                        Ok(&want),
                        "q20 my={my} mn={mn} {consumer} input#{k} (taken={}) must agree, {m:?}: got {r:?}",
                        if k == 0 { "yes" } else { "no" }
                    );
                }
            }
        }
    }
}

/// Build a BUG-020 quadrant body: `my_mid`/`mn_mid` place the yes/no
/// arm's base slot declaration inside the nested loop (may-Nil) or
/// before it (proven non-Nil); `consumer` selects how the value-if
/// result is consumed (arith/cbytes → 71 guard, eqneq → 90 guard).
fn quad20_body(my_mid: bool, mn_mid: bool, consumer: &str) -> String {
    let mut b = String::from("var t2 = 0u64\nvar acc = 0u64\n");
    if !my_mid {
        b.push_str("var t4 = 0u64\n");
    }
    if !mn_mid {
        b.push_str("var t0 = 0u64\n");
    }
    b.push_str("while t2 < 2 { t2 += 1\nvar t3 = 0u64\nwhile t3 < 1 { t3 += 1\n");
    if my_mid {
        b.push_str("var t4 = 0u64\n");
    }
    if mn_mid {
        b.push_str("var t0 = 0u64\n");
    }
    b.push_str("while t4 < 3 { t4 += 1 }\n}\n");
    let iff = "if ((byte(data,15) as u64) == 4u64) { t4 } else { t0 }";
    let line = match consumer {
        "arith" => format!("acc = (acc + ({iff}))"),
        "eqneq" => format!("if (({iff}) == (0u64)) {{ acc = (acc + 1u64) }}"),
        "cbytes" => format!("acc = (acc + (size(({iff}) as bytes) as u64))"),
        _ => unreachable!(),
    };
    b.push_str(&line);
    b.push_str("\n}\nreturn (acc) as bytes");
    b
}

/// P2-C3 deliverable C: the FEEDBACK saturation run (env knob
/// `GEN_C3_FEEDBACK=1`, default OFF so the default tier is unchanged).
/// Iterates deterministic corpus rounds with plateau detection, then the
/// targeted probes close the rest; prints the FULL provenance table
/// (denominator item × hitting case) and asserts
/// hits ∪ unreachable == denominator.
#[test]
#[ignore]
fn c3_saturation_feedback() {
    if std::env::var("GEN_C3_FEEDBACK").ok().as_deref() != Some("1") {
        println!("GEN_C3_FEEDBACK != 1: feedback mode disabled (default off)");
        return;
    }
    let rounds = std::env::var("GEN_C3_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(6u64);
    let mut hits = Hits::new();
    // phase 1: corpus rounds (deterministic seed walk, plateau stop)
    let bad = measure_corpus(&mut hits, 0xC3FB_0000_0000_0001, rounds, "fb");
    assert!(bad.is_empty(), "feedback corpus found unexpected: {bad:?}");
    // phase 2: targeted probes for whatever remains (the loop equivalent
    // is c3_targeted_probes_cover_denominator, always-on; here we verify
    // it closes this run's gaps too and print the union table)
    {
        let mut probe_hits = Hits::new();
        let before = hits.map.len();
        // P2-C3 fix: run the FULL shared probe table (the previous replica
        // here was compile-only on several dimensions and skipped the
        // negative tables + feat-keyed attributions — its final equality
        // failed on 52 omitted items). Single source of truth =
        // run_targeted_probes (the always-on test's body).
        run_targeted_probes(&mut probe_hits);
        // merge probe hits into the corpus hits
        for (k, v) in probe_hits.map {
            hits.map.entry(k).or_insert(v);
        }
        println!(
            "[feedback] corpus {before} items; probes added {} more",
            hits.map.len() - before
        );
    }
    let items = denom_items();
    let missing: Vec<&str> = items
        .iter()
        .copied()
        .filter(|i| !hits.contains(i) && !UNREACHABLE.iter().any(|(u, _)| u == i))
        .collect();
    println!(
        "SATURATION TABLE: {}/{} hit (+{} unreachable) = {}/{} covered",
        hits.map.len(),
        DENOM_LEN,
        UNREACHABLE.len(),
        hits.map.len() + UNREACHABLE.len(),
        DENOM_LEN
    );
    for (k, v) in hits.map.iter() {
        println!("HIT {k} <- {v}");
    }
    for (u, arg) in UNREACHABLE.iter() {
        println!("UNREACHABLE {u}: {arg}");
    }
    assert!(missing.is_empty(), "feedback saturation left unhit: {missing:?}");
    assert_eq!(hits.map.len() + UNREACHABLE.len(), DENOM_LEN);
}

/// BUG-021 (FIXED 2026-11-15, P2-C4-MASS S6; found by the S1 10^4-case
/// bombardment run at case ~1400+, hunt seeds 0xB0DB…+1461/+1877): the
/// lowerer's 8-value register pool can be exhausted mid-CHOOSE-arm, and
/// `push_vreg`'s depth_err deferral then leaves the vstack SHORT by design
/// (emission continues so the final reject carries one clean diagnostic).
/// The CHOOSE cond read was the lowerer's ONLY unchecked vstack access —
/// `vstack.len() - 1 - 2*vals` — so on a shrunk stack it underflowed and
/// the release-profile wrapped-index OOB PANICKED the whole run instead of
/// producing the depth_err reject. Pre-fix evidence (pinned, deterministic):
/// "index out of bounds: the len is 2 but the index is
/// 18446744073709551615" at the CHOOSE cond read (rv64.rs:5855), release
/// profile, S1 bombard window. Trigger shape (delta-minimized, 20 lines):
/// a CHOOSE whose ELSE arm nests a big literal (16777216 → CU64(CU32
/// PBUF), pool-hungry) over a computed-index byte() whose constant-MOD
/// index (`c1 % 40u64`) takes the arith_k_handles absorption path — the
/// absorption also pools, the depth runs out mid-arm, and the cond read
/// indexes the shrunk stack.
///
/// Fix (rv64.rs CHOOSE arm): the cond index is now `checked_sub` + `.get`
/// guarded; on a shrunk stack it returns the depth_err reject text
/// ("…nesting exceeds the 8-value register pool (kernel subset limit)") —
/// exactly the text the bombardment's "nesting exceeds" downsizing retry
/// keys on, so the shape downgrades deterministically instead of killing
/// the run. This is a clean kernel-subset boundary reject (the 8-value
/// pool is a documented lowerer limit), NOT a miscompile: the interpreter
/// accepts and runs the same body, and the near-miss control (the same
/// body with ONE nesting level dropped) lowers and agrees on all three
/// modes. Measured boundary fact (probe round): literal size is NOT the
/// discriminator — a small-literal clone of the minimal body still
/// exhausts the pool; depth is. The control keeps the big literal and the
/// constant-MOD computed index and drops only the inner min(2, …) wrapper.
#[test]
fn c3_pin_bug021_choose_pool_exhaustion() {
    // delta-minimized S1 panicking body (hunt seed 0xB0DB…+1461 window)
    let body = "var c1 = 0u64\nvar k1 = 0u64\nwhile k1 < 8 { c1 = c1 * 256 + (byte(data, k1 + 8) as u64)\nk1 += 1 }\nvar c2 = 0u64\nvar k2 = 0u64\nwhile k2 < 8 { c2 = c2 * 256 + (byte(data, k2 + 16) as u64)\nk2 += 1 }\nvar acc = 0u64\nvar t0 = 0u64\nwhile t0 < 7 { t0 += 1\nacc = ((!(((acc) == (c2)))) ? (acc) : (16777216u64) + (min(((9u64 + c2)), ((min((2u64), ((byte(data, (c1 % 40u64)) as u64))))))))\n}\nvar t2 = 0u64\nwhile t2 < 11 { t2 += 1\n}\nreturn (acc) as bytes";
    // interpreter: accepts and runs (the shape is NOT invalid fitsh)
    let input = vec![0u8; 40]; // c1 = 0 → c1%40 = 0 < 40: every byte() in range, no overflow
    let it = Interp::try_build(body).expect("BUG-021: interp must build");
    let want = it
        .run(&input)
        .expect("BUG-021: interp must run the exhausted-pool body");
    // rv64: clean depth_err reject in EVERY profile — never a panic
    let rv = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Rv64::try_build(body)
    }))
    .expect("BUG-021: rv64 try_build must NOT panic (pre-fix: wrapped-index OOB at the CHOOSE cond read)");
    let err = match rv {
        Err(e) => e,
        Ok(_) => panic!("BUG-021: 8-value pool exhaustion must reject"),
    };
    assert!(
        err.contains("nesting exceeds the 8-value register pool"),
        "BUG-021: reject must be the depth_err text the downsizing retry keys on, got: {err}"
    );
    assert!(
        err.contains("kernel subset limit"),
        "BUG-021: reject must name the kernel-subset boundary, got: {err}"
    );
    // near-miss control: same body with ONE nesting level dropped (the
    // inner min(2, …) wrapper — the pool-hungry big literal AND the
    // constant-MOD computed index stay) — the pool is NOT exhausted, rv64
    // lowers and agrees with the interpreter (the reject sits exactly at
    // the documented 8-value depth boundary, one level above an accepted
    // shape; literal size is NOT the discriminator, depth is — a small-
    // literal clone of the minimal body still rejects)
    let ctrl = body.replace(
        "(min(((9u64 + c2)), ((min((2u64), ((byte(data, (c1 % 40u64)) as u64)))))))",
        "(byte(data, (c1 % 40u64) as u64))",
    );
    assert_ne!(ctrl, body, "BUG-021 control: replacement must apply");
    let it2 = Interp::try_build(&ctrl).expect("BUG-021 control: interp builds");
    let rv2 = Rv64::try_build(&ctrl).expect("BUG-021 control: rv64 lowers");
    let want2 = it2
        .run(&input)
        .expect("BUG-021 control: interp runs");
    for (m, r) in rv2.run(&input) {
        assert_eq!(
            r.as_ref(),
            Ok(&want2),
            "BUG-021 control (16u64 literal, same skeleton) must agree, {m:?}"
        );
    }
    let _ = want; // interp acceptance evidence (kept for debuggability)
}

/// BUG-022 (FIXED 2026-11-15, P2-C4-MASS S6; found by the S1 10^4-case
/// bombardment rerun, hunt seed 0xB0DB…+6249 family, delta-min 6 lines):
/// a loop-body assignment whose slot's ONLY read is its own statement
/// (`c9 *= X` / `c9 = c9 * X` last in a while body) had its store elided
/// by `store_is_dead` in Im/Imc: the backedge-liveness window was
/// half-open [span_a, store_idx) and missed the self-read AT the store's
/// own index — but on the next iteration that read re-executes AFTER the
/// store, so the elided store fed it. The multiply then recomputed from
/// the stale slot value every pass (overflow never fired) and Im/Imc
/// returned Ok where the interpreter trapped 90 at iteration 2 and
/// ImcRemap (banked slots never reach the DSE) also trapped 90 — a
/// three-way mode split. Pre-fix evidence (pinned): Im/Imc Ok(8B) on the
/// m6/m7 probes, ImcRemap Err(90), interp mul-check-90; the Im code dump
/// shows Mulhu+Branch-Ne+Mul with NO store of the product before the
/// backedge.
///
/// Fix (rv64.rs store_is_dead): the backedge window is now [span_a,
/// store_idx] — a read at the store's own statement index keeps the store
/// alive. Pure Im/Imc memory-path change (banked ImcRemap and the census
/// pins are untouched: 2922/3072 + causes 113/446 re-verified byte-exact).
#[test]
fn c3_pin_bug022_loop_body_selfread_store() {
    for (tag, body) in [
        // plain-assign form (m7): the product must land in the slot so the
        // second iteration's multiply overflows like the interpreter's
        ("plain", "var c9 = 255u64\nvar t0 = 0u64\nwhile t0 < 3 { t0 += 1\nc9 = c9 * 87960930222080u64\n}\nreturn (0u64) as bytes"),
        // compound XOP form (m6): same self-read shape via the old-value read
        ("xop", "var c9 = 255u64\nvar t0 = 0u64\nwhile t0 < 3 { t0 += 1\nc9 *= 87960930222080u64\n}\nreturn (0u64) as bytes"),
    ] {
        let it = Interp::try_build(body).expect("BUG-022 {tag}: interp builds");
        let fault = it
            .run(&[])
            .err()
            .expect("BUG-022 {tag}: interp must trap 90 at iteration 2");
        assert_eq!(
            fault.code, 90,
            "BUG-022 {tag}: interp fault must be the mul overflow"
        );
        let rv = Rv64::try_build(body).expect("BUG-022 {tag}: rv64 lowers");
        for (m, r) in rv.run(&[]) {
            assert_eq!(
                r.as_ref(),
                Err(&90u64),
                "BUG-022 {tag}: {m:?} must trap 90 like the interpreter (pre-fix Im/Imc returned Ok — elided loop-body store)"
            );
        }
    }
    // near-miss control: the same statement OUTSIDE a loop stores normally
    // (the first multiply's store is kept by the linear read that follows)
    let ctl = "var c9 = 255u64\nc9 *= 87960930222080u64\nreturn (c9) as bytes";
    let it = Interp::try_build(ctl).expect("BUG-022 ctl: interp builds");
    let want = it.run(&[]).expect("BUG-022 ctl: interp runs");
    let rv = Rv64::try_build(ctl).expect("BUG-022 ctl: rv64 lowers");
    for (m, r) in rv.run(&[]) {
        assert_eq!(
            r.as_ref(),
            Ok(&want),
            "BUG-022 ctl: standalone self-read store must keep the value, {m:?}"
        );
    }
}

/// BUG-023 (FIXED 2026-11-15, P2-C4-MASS S6; found by the S1 10^4-case
/// bombardment rerun, hunt seeds 0xB0DB…+1667/+4203/+4797/+7058 family,
/// delta-min 4 lines): the ImcRemap bank allocator's conflict test used
/// STRICT inequalities (`a.start < u.end && u.start < a.end`) while
/// `Seg.end` is INCLUSIVE — the cfg fixpoint extends a loop counter's
/// segment across the backedge, so the counter is live AT its segment's
/// end statement. Two units touching at that boundary statement were
/// co-located into one register:
///   * `while t0 < 1 { t0 += 1; var t1 = 0u64 }` — t1's init unit [7,7]
///     touched t0's unit [3,7] and took t0's home; the counter reset every
///     iteration and ImcRemap spun to OutOfGas while Im/Imc (frame homes,
///     no sharing) returned Ok — a mode split with interp Ok;
///   * the symmetric shape (a later-STARTING unit placed first by
///     priority ends where a current unit starts — the 14-line 7058
///     minimal: its boundary write clobbered the inner counter's
///     backedge-live value the same way).
/// Pre-fix evidence (pinned): ImcRemap Err(OutOfGas) at the cycle cap on
/// both bodies; Im/Imc Ok. The strict test is DELIBERATE for read→write
/// chains (`t1 = t0 + 1` coalescing): reads evaluate before same-
/// statement writes, so a touch is safe exactly when the ENDING unit's
/// slot reads at the boundary statement.
///
/// Fix (rv64.rs allocate_bank): the conflict test is closed-interval and
/// exempts only same-slot touches and ending-unit self-read touches (read
/// sets snapshotted before the segment build consumes the events).
#[test]
fn c3_pin_bug023_bank_touch_counter_clobber() {
    for (tag, body) in [
        // the 4-line delta-min: t1's init unit touched t0's backedge-extended
        // unit — pre-fix ImcRemap spun (t0 reset every iteration)
        ("decl_last", "var t0 = 0u64\nwhile t0 < 1 { t0 += 1\nvar t1 = 0u64\n}\nreturn (t0) as bytes"),
        // the counter must SURVIVE the touch and produce the interp value
        ("nested", "var c0 = 0u64\nvar c1 = 0u64\nvar c2 = 0u64\nvar c9 = (byte(data, 0) as u64)\nvar acc = 0u64\nvar t0 = 0u64\nwhile t0 < 2 { t0 += 1\nvar t1 = 0u64\nwhile t1 < 3 { t1 += 1\nvar t3 = ((c2 as u8) as u64)\n}\n}\nt3 = ((((((((c0) >= (c9)) as u64)) < (((c1) + (t3)))) as u64)) / ((((((byte(data, 0) * byte(data, 1)) + 1u8) as u64)) | 1u64)))\nt1 = if (((((!(((t3) == ((byte(data, (acc % 40u64)) as u64))))) as u64)) <= (((7u64) * (c1))))) { c2 } else { min((t3), (18446744073709551615u64)) }\nreturn (t0) as bytes"),
    ] {
        let it = Interp::try_build(body).expect("BUG-023 {tag}: interp builds");
        let input = (0..256u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        let want = it.run(&input).expect("BUG-023 {tag}: interp runs");
        let rv = Rv64::try_build(body).expect("BUG-023 {tag}: rv64 lowers");
        for (m, r) in rv.run(&input) {
            assert_eq!(
                r.as_ref(),
                Ok(&want),
                "BUG-023 {tag}: {m:?} must terminate with the interp value (pre-fix ImcRemap spun to OutOfGas — bank touch clobbered the loop counter)"
            );
        }
    }
    // the read->write chain the strict test was written for must STILL
    // coalesce (the exemption's purpose): t0's value flows into t1 through
    // the shared register — correctness unchanged, just watching agreement
    let chain = "var t0 = 5u64\nvar t1 = 0u64\nvar t2 = 0u64\nwhile t2 < 3 { t2 += 1\nt1 = t0 + 1u64\nvar t3 = 0u64\n}\nreturn (t1) as bytes";
    let it = Interp::try_build(chain).expect("BUG-023 chain: interp builds");
    let want = it.run(&[]).expect("BUG-023 chain: interp runs");
    let rv = Rv64::try_build(chain).expect("BUG-023 chain: rv64 lowers");
    for (m, r) in rv.run(&[]) {
        assert_eq!(
            r.as_ref(),
            Ok(&want),
            "BUG-023 chain: read->write coalescing must stay correct, {m:?}"
        );
    }
}

/// BUG-024 (FIXED 2026-11-15, P2-C4-MASS S2; found by the first 10^5-case
/// round, hunt seeds 0xB0DB…+79426/+79912/+60251): the compressed
/// assembler's scratch pair SC1C/SC2C (x30/x31) collided with the Imc
/// value pool — Imc shares Im's VSTACK_REGS = [x5, x6, x7, x28, x29, x30,
/// x31, x14], so x30/x31 ARE pool positions 5/6 there, and every
/// sc1-scoped helper clobbered a live vstack value at depth 6+. Shapes:
///   * `byte(data, (t1 % 40))` with the index at pool depth 5: the GET's
///     nil_wtag_from_flag loaded the slot's Nil FLAG (1) over the index
///     (4) — Imc returned 18|1=19 where interp/Im/ImcRemap returned
///     18|4=22 (content divergence, same length);
///   * an overflow-check operand at that depth read the flag instead of
///     the operand — Imc returned Ok where interp/Im/ImcRemap trapped 90
///     (seed …+60251), and where the D2 pairing says 71 (seed …+79426:
///     interp StackError(42), Im/ImcRemap 71, Imc Ok).
/// Im is untouched (SC1=x15 lies outside VSTACK_REGS); ImcRemap is
/// untouched (its pool VSTACK_ORDER_RVC excludes x30/x31).
///
/// Fix (rv64.rs): the RVC assembler takes an explicit scratch pair — Imc
/// now uses SC1I/SC2I = x12/x13 (outside VSTACK_REGS, outside its
/// (a0,a1)=(x10,x11) pair, saved to x18/x19 by the prologue before the
/// body runs); ImcRemap keeps SC1C/SC2C.
#[test]
fn c3_pin_bug024_imc_scratch_collision() {
    // the delta-min (14 lines): the byte index lands at pool depth 5 and
    // its GET publishes a Nil tag via sc1 — pre-fix Imc computed
    // 18|flag=19, everyone else 18|t1=22
    let body = "var c0 = 0u64\nvar c9 = (byte(data, 0) as u64)\nvar acc = 0u64\nvar t0 = 0u64\nwhile t0 < 2 { t0 += 1\nvar t1 = 0u64\nwhile t1 < 3 { t1 += 1\nif ((c0) >= (acc)) { t1 += 1\ncontinue }\n}\n}\nacc = (acc ^ ((((c9) ** 10u64) + (((byte(data, 18) as u64)) | ((byte(data, (t1 % 40u64)) as u64))))))\nreturn acc as bytes";
    let input = (0..256u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
    let it = Interp::try_build(body).expect("BUG-024: interp builds");
    let want = it.run(&input).expect("BUG-024: interp runs");
    assert_eq!(want.len(), 8);
    let rv = Rv64::try_build(body).expect("BUG-024: rv64 lowers");
    for (m, r) in rv.run(&input) {
        assert_eq!(
            r.as_ref(),
            Ok(&want),
            "BUG-024: {m:?} must produce the interp bytes (pre-fix Imc read the Nil flag over the index — 19 vs 22)"
        );
    }
    // and the trap-side shape: the overflow check's operand at the same
    // depth (seed …+60251's delta-min) must trap 90 in Imc too
    let trap = "var c0 = 0u64\nvar k0 = 0u64\nwhile k0 < 8 { c0 = c0 * 256 + (byte(data, k0 + 0) as u64)\nk0 += 1 }\nvar c1 = 0u64\nvar c2 = 0u64\nvar s0 = buf_right_drop(0, buf_left(6, data))\nvar acc = 0u64\nacc = (acc ^ (((((4u64) * (c2))) + (clamp((c2), (4u64), (c0))))))\nacc = (acc ^ (((((acc) == (c1))) ? (((c1) + ((byte(data, 2) as u64)))) : (max((acc), (acc))) + choose((((size(s0) as u64)) == (225u64)), 8u64, 18446744073709551615u64))))";
    let it2 = Interp::try_build(trap).expect("BUG-024 trap: interp builds");
    let f = it2
        .run(&input)
        .err()
        .expect("BUG-024 trap: interp must fault 90");
    assert_eq!(f.code, 90);
    let rv2 = Rv64::try_build(trap).expect("BUG-024 trap: rv64 lowers");
    for (m, r) in rv2.run(&input) {
        assert_eq!(
            r.as_ref(),
            Err(&90u64),
            "BUG-024 trap: {m:?} must trap 90 like the interpreter (pre-fix Imc returned Ok)"
        );
    }
}

/// BUG-025/BUG-026 (FIXED 2026-11-15, P2-C4-MASS S2; found by the first
/// 10^5-case round, hunt seeds 0xB0DB…+94042 (spin) and the 4-line
/// hand-min of the same family): BUG-023's read-at-boundary exemption for
/// bank-unit touches was sound only in STRAIGHT-LINE code — inside a
/// while span the next iteration re-executes the ending slot's reads
/// AFTER the surviving slot's boundary write to the shared home:
///   * BUG-025 (spin): `while t1 < 3 { t1 += 1; var t2 … while t2 < 1 {
///     t2 += 1; var t3 = (byte ** 61 + clamp) / ((size(s0) - c1 + t0*c1) |
///     1) } }` (11 lines) — a touch inside the nested loop let a sharer
///     take a counter's home; ImcRemap spun to OutOfGas while Im/Imc and
///     the interpreter ran clean (interp Ok(0));
///   * BUG-026 (invented 71): `var s0 = buf_left_drop(2, buf_left(8,
///     data)); while t0 < 16 { var s1 = buf_left_drop(1, s0) }` (4 lines)
///     — s1's init unit touched s0's backedge-extended unit and took its
///     home (x15/x3); s0's backedge re-read then saw s1's handle and the
///     slice count check faulted 71 in ImcRemap while Im/Imc and the
///     interpreter ran clean.
///
/// Fix (rv64.rs allocate_bank): the exemption now additionally refuses a
/// touch when the ENDING unit is LOOP-CARRIED into a while span containing
/// the boundary (its def precedes that span — the surviving slot's
/// boundary write would clobber a home whose next-iteration re-reads have
/// no intervening re-def). Touches whose ending unit's def lies inside
/// every enclosing span stay governed by the straight-line read-at-
/// boundary rule (the hacd200 anchor's 8 in-loop boundary chains are all
/// of that sound shape — 3072 B pinned, bytes unchanged).
#[test]
fn c3_pin_bug025_bug026_bank_touch_in_loop() {
    for (tag, body, input) in [
        ("bug025_spin", "var c0 = 0u64\nvar c1 = 0u64\nvar s0 = if ((((18446744073709551614u64) == (c0))) || (((256u64) == (c0)))) { buf_left(3, data) } else { buf_left(4, data) }\nvar t0 = 0u64\nvar t1 = 0u64\nwhile t1 < 3 { t1 += 1\nvar t2 = 0u64\nwhile t2 < 1 { t2 += 1\nvar t3 = ((((((byte(data, (t1 % 40u64)) as u64)) ** 61u64) + clamp((t2), (t2), (6u64)))) / (((((((size(s0) as u64)) - (c1)) + ((t0) * (c1)))) | 1u64)))\n}\n}", vec![0u8; 16]),
        ("bug026_slice71", "var s0 = buf_left_drop(2, buf_left(8, data))\nvar t0 = 0u64\nwhile t0 < 16 { t0 += 1\nvar s1 = buf_left_drop(1, s0)\n}", vec![0xFFu8; 16]),
    ] {
        let it = Interp::try_build(body).expect("BUG-025/026 {tag}: interp builds");
        let want = it.run(&input).expect("BUG-025/026 {tag}: interp runs");
        let rv = Rv64::try_build(body).expect("BUG-025/026 {tag}: rv64 lowers");
        for (m, r) in rv.run(&input) {
            assert_eq!(
                r.as_ref(),
                Ok(&want),
                "BUG-025/026 {tag}: {m:?} must terminate with the interp output (pre-fix ImcRemap spun to OutOfGas / invented 71 — in-loop bank touch took a live counter/handle home)"
            );
        }
    }
}
