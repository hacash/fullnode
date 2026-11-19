//! C12/C13 (impl/cse-dse-bank): targeted differential tests.
//!
//! C13 — banked-slot dead-write auxiliary machine elimination (E7): when
//!        the web timeline proves a banked slot's store dead (no later
//!        read on any path; loop-carried + BUG-022 backedge exemptions
//!        live inside `store_is_dead`), the bank write's shadow/flag/mv
//!        attachments — and the fault-free literal materializations —
//!        die with it, while every fault-bearing expression evaluates in
//!        full (survey/encoding-regalloc.md §E7: flag 读面全经 GET/XOP/
//!        XLG 记入时间线, dyn_w shadow 读面前置 GET 同为 Read 事件).
//! C12 — bytes-literal descriptor CSE (survey/eval-stmt-list.md §C12): a
//!        repeat materialization of the same pool label into exactly the
//!        still-live register pair re-issues the descriptor as Refs
//!        instead of re-emitting lit_addr + li. The hit face is
//!        basic-block-local (see `Lower::desc_fwd` / `eval_lit_bytes_pair`
//!        docs for why the cross-compare shape must NOT fire); the tests
//!        pin both the hits and the two mandatory misses (cross-join,
//!        register re-hand).
//!
//! Differential pattern (the ctrl_dse_tests convention): every body runs
//! through the interpreter (expectations are the interpreter's documented
//! faces) and all three RV64 lowering modes. Structural assertions are
//! code-length equalities/inequalities per mode; the banked mechanisms
//! only exist in ImcRemap, but the frame paths already carry the same
//! DSE/fresh-materialization faces, so the length relations hold in
//! every mode unless stated otherwise.

use super::helpers::{compile_with_input, run_input_mode};
use super::super::*;

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

/// 15-byte plain bytes literals (NOT 21-byte address-shaped: address
/// slots are never banked and would silently drop the tests onto the
/// frame paths).
const LIT_X: &str = "k_0123456789abc";
const LIT_Y: &str = "m_0123456789abc";
const LIT_Z: &str = "k_9876543210xyz";

fn code_len_mode(body: &str, mode: Rv64Mode) -> usize {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, mode)
        .unwrap_or_else(|e| panic!("lower failed ({mode:?}): {e}"));
    lo.code.len()
}

fn pool_len_mode(body: &str, mode: Rv64Mode) -> usize {
    let (ir, _) = compile_with_input(body);
    let lo = lower_body_mode(&ir, true, mode)
        .unwrap_or_else(|e| panic!("lower failed ({mode:?}): {e}"));
    lo.pool.len()
}

fn assert_ok_bytes(body: &str, want: &[u8], tag: &str) {
    for mode in MODES {
        let out = run_input_mode(body, b"", mode);
        assert_eq!(out, want, "{tag} mode {mode:?}");
    }
}

/// u64-valued returns come out as 8-byte BE; `size(...)` views are u16
/// (2-byte BE) — the interpreter's documented faces.
fn assert_ok_u64(body: &str, want: u64, tag: &str) {
    assert_ok_bytes(body, &want.to_be_bytes(), tag);
}

fn assert_ok_u16(body: &str, want: u16, tag: &str) {
    assert_ok_bytes(body, &want.to_be_bytes(), tag);
}

/// Per-mode relation assertion: `rel` must hold for every mode.
fn assert_len_rel(a: &str, b: &str, rel: fn(usize, usize) -> bool, name: &str, tag: &str) {
    for mode in MODES {
        let (la, lb) = (code_len_mode(a, mode), code_len_mode(b, mode));
        assert!(
            rel(la, lb),
            "{tag} / {name} mode {mode:?}: {la} !{name} {lb}"
        );
    }
}

fn lt(a: usize, b: usize) -> bool {
    a < b
}
fn gt(a: usize, b: usize) -> bool {
    a > b
}
fn eq(a: usize, b: usize) -> bool {
    a == b
}

/* ==================================================================== */
/* C13 — banked dead-write auxiliary machine elimination                */
/* ==================================================================== */

/// The loop-tail dead counter bind (`i = 9` after the loop, `i` never
/// read again) emits ZERO bytes on the bank path: the plain-literal bank
/// bind carries no fault face, so the C13 arm kills the bookkeeping and
/// returns. Pinned with the per-statement debug ranges (removing the
/// statement shifts the bank allocation, so whole-body length equality
/// is not a clean pin). The frame arms (Im/Imc) keep their pre-existing
/// face — the rhs literal still evaluates into a pool register before
/// the store decision — so only the bank mode pins the zero.
#[test]
fn c13_loop_tail_dead_bind_emits_zero_bytes() {
    let body =
        "var i = 0u64\nvar n = 3u64\nwhile n > 0u64 { i = i + 1u64; n = n - 1u64 }\ni = 9\nreturn 7u64 as bytes";
    let (ir, _) = compile_with_input(body);
    let (_, dbg) = lower_body_mode_arena_dbg(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX)
        .unwrap_or_else(|e| panic!("lower failed: {e}"));
    // premise: the counter slot is banked (slot 0 is the frame input, 1 = i)
    assert_eq!(dbg.banked[1], BANK_KIND_INT, "premise: i is banked");
    // the tail bind is walk index 9 (10 statements total, 1-based)
    let rng = dbg
        .stmt_ranges
        .iter()
        .find(|r| r.w_lo == 9 && r.w_hi == 9)
        .unwrap_or_else(|| panic!("tail bind range missing in {:?}", dbg.stmt_ranges));
    assert_eq!(
        rng.off_start, rng.off_end,
        "dead bank bind must emit nothing (C13)"
    );
    assert_ok_u64(body, 7, "dead tail runs");
}

/// The live twin (`i` read after the tail bind) keeps the write: the
/// dead-tail body must be strictly shorter than the live-tail one.
#[test]
fn c13_live_tail_costs_more_than_dead_tail() {
    let dead = "var i = 0u64\nvar n = 3u64\nwhile n > 0u64 { i = i + 1u64; n = n - 1u64 }\ni = 9\nreturn 7u64 as bytes";
    let live = "var i = 0u64\nvar n = 3u64\nwhile n > 0u64 { i = i + 1u64; n = n - 1u64 }\ni = 9\nreturn (i + 7u64) as bytes";
    assert_len_rel(dead, live, lt, "<", "dead tail cheaper than live tail");
    assert_ok_u64(dead, 7, "dead tail result");
    assert_ok_u64(live, 16, "live tail result (9 + 7)");
}

/// Backedge exemption (BUG-022 discipline): `c`'s only reader is the loop
/// condition — linearly BEFORE its body bind — and the bind must stay
/// (the backedge re-reads it; without it the loop never terminates). The
/// no-cond twin, whose bind has no reader at all, must be strictly
/// shorter (bind fully elided).
#[test]
fn c13_backedge_cond_read_keeps_body_bind() {
    let exempt =
        "var c = 0u64\nvar n = 3u64\nwhile c == 0u64 && n > 0u64 { c = 1u64; n = n - 1u64 }\nreturn 7u64 as bytes";
    let dead =
        "var c = 0u64\nvar n = 3u64\nwhile n > 0u64 { c = 1u64; n = n - 1u64 }\nreturn 7u64 as bytes";
    assert_len_rel(exempt, dead, gt, ">", "cond-read bind kept, unread bind elided");
    // Termination IS the behavioral face: had the exempt body's bind been
    // elided, `c` would stay 0 and the loop would never exit.
    assert_ok_u64(exempt, 7, "exempt body terminates (store fired)");
    assert_ok_u64(dead, 7, "dead-bind twin terminates");
}

/// Banked XOP dead write (`a *= 3u32` on a mixed-width slot, `a` never
/// read after): the checked arithmetic stays in full (fault face), but
/// the shadow_grow auxiliary machine dies with the store. The frame XOP
/// arm already had the face, so the relation holds in every mode.
#[test]
fn c13_xop_dead_write_sheds_shadow_grow() {
    let dead = "var a = 2u8\na *= 3u32\nreturn 7u64 as bytes";
    let live = "var a = 2u8\na *= 3u32\nreturn (a + 7u64) as bytes";
    assert_len_rel(dead, live, lt, "<", "dead XOP cheaper than live XOP");
    assert_ok_u64(dead, 7, "dead XOP result");
    assert_ok_u64(live, 13, "live XOP result (2 * 3 + 7)");
}

/// Banked max/min dead write (the fused `if a < K { a = K }` shape, `a`
/// never read after): the fused comparison stays (its Nil/90 face and
/// the E7 arithmetic-retention rule), the shadow grow dies with the
/// proven-dead store.
#[test]
fn c13_maxmin_dead_write_sheds_shadow_grow() {
    let dead = "var a = 2u8\nif a < 5u32 { a = 5u32 }\nreturn 7u64 as bytes";
    let live = "var a = 2u8\nif a < 5u32 { a = 5u32 }\nreturn (a + 7u64) as bytes";
    assert_len_rel(dead, live, lt, "<", "dead max/min cheaper than live max/min");
    assert_ok_u64(dead, 7, "dead max/min result");
    assert_ok_u64(live, 12, "live max/min result (max(2, 5) + 7)");
}

/* ==================================================================== */
/* C12 — bytes-literal descriptor CSE                                   */
/* ==================================================================== */

/// Same-slot literal rebind (`a = X` twice in a row, both binds live via
/// the trailing read): the second bind re-issues the still-live
/// descriptor instead of re-emitting lit_addr + li. Cheaper than the
/// different-literal twin in every mode; the pool dedup (one entry per
/// distinct literal) is the pre-existing face C12 builds on.
#[test]
fn c12_rebind_same_label_hits() {
    let hit = format!(
        "var a = \"{LIT_X}\"\na = \"{LIT_X}\"\nreturn size(a) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\na = \"{LIT_Z}\"\nreturn size(a) as bytes"
    );
    assert_len_rel(&hit, &diff, lt, "<", "rebind hit cheaper than fresh literal");
    assert_ok_u16(&hit, 15, "rebind hit result");
    assert_ok_u16(&diff, 15, "rebind diff result");
    // pool: the hit body dedups both binds onto ONE entry.
    for mode in MODES {
        assert!(
            pool_len_mode(&hit, mode) < pool_len_mode(&diff, mode),
            "rebind pool dedup mode {mode:?}"
        );
    }
}

/// The stmt.rs BANK-path hit (ImcRemap): a loop-carried bytes slot keeps
/// ONE segment (same bank homes) across the loop, so two consecutive
/// same-label binds inside the body hit the desc_fwd entry recorded by
/// the first bind. The frame modes hit through the same-depth eval path.
#[test]
fn c12_bank_path_rebind_hits_under_loop_carried() {
    let hit = format!(
        "var a = \"{LIT_X}\"\nvar n = 3u64\nwhile size(a) == 15u64 && n > 0u64 {{ a = \"{LIT_X}\"; a = \"{LIT_X}\"; n = n - 1u64 }}\nreturn size(a) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\nvar n = 3u64\nwhile size(a) == 15u64 && n > 0u64 {{ a = \"{LIT_X}\"; a = \"{LIT_Z}\"; n = n - 1u64 }}\nreturn size(a) as bytes"
    );
    assert_len_rel(&hit, &diff, lt, "<", "bank rebind hit cheaper than fresh literal");
    assert_ok_u16(&hit, 15, "bank rebind hit result");
    assert_ok_u16(&diff, 15, "bank rebind diff result");
}

/// Consecutive distinct-slot binds (`a = X; b = X`, both read after):
/// the second site runs at the same stack depth as the first, so the
/// frame-path eval hit re-issues the pair (Im/Imc). In ImcRemap both
/// slots get their own bank homes — a different pair by construction —
/// so the second site must MISS there (equal to the different-literal
/// twin); pinning that asymmetry pins the hit rule (same pair only).
#[test]
fn c12_multi_site_frame_hit_bank_miss() {
    let hit = format!(
        "var a = \"{LIT_X}\"\nvar b = \"{LIT_X}\"\nreturn (size(a) + size(b)) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\nvar b = \"{LIT_Z}\"\nreturn (size(a) + size(b)) as bytes"
    );
    // Im / Imc: frame eval path, same depth 0 → hit.
    for mode in [Rv64Mode::Im, Rv64Mode::Imc] {
        let (lh, ld) = (code_len_mode(&hit, mode), code_len_mode(&diff, mode));
        assert!(lh < ld, "frame multi-site hit mode {mode:?}: {lh} !< {ld}");
    }
    // ImcRemap: distinct bank homes → mandatory miss.
    let (lh, ld) = (
        code_len_mode(&hit, Rv64Mode::ImcRemap),
        code_len_mode(&diff, Rv64Mode::ImcRemap),
    );
    assert_eq!(
        lh, ld,
        "bank multi-site must miss (distinct homes): {lh} != {ld}"
    );
    assert_ok_u16(&hit, 30, "multi-site hit result");
    assert_ok_u16(&diff, 30, "multi-site diff result");
}

/// Cross-join miss (mandatory): a join label between the establishing
/// and the repeat bind must drop the entry — the arm's establishment
/// does not dominate the join, so keeping the entry would be unsound.
/// The post-if rebind must cost exactly what a fresh different literal
/// costs (equal lengths), i.e. no hit fires across the join.
#[test]
fn c12_cross_join_miss() {
    let hit = format!(
        "var a = \"{LIT_X}\"\nvar s = size(a)\nvar n = 2u64\nif n > 0u64 {{ n = n - 1u64 }}\na = \"{LIT_X}\"\nreturn (size(a) + s) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\nvar s = size(a)\nvar n = 2u64\nif n > 0u64 {{ n = n - 1u64 }}\na = \"{LIT_Z}\"\nreturn (size(a) + s) as bytes"
    );
    assert_len_rel(&hit, &diff, eq, "==", "no hit across a join");
    assert_ok_u16(&hit, 30, "cross-join hit body result");
    assert_ok_u16(&diff, 30, "cross-join diff body result");
}

/// Re-hand miss (mandatory): an intervening materialization (here `t =
/// Y`) pushes fresh pool registers, re-handing — and killing the entry
/// via `kill_fwd_reg` — exactly the registers the X descriptor watched.
/// On the frame paths the final X bind must therefore re-materialize
/// (equal to the different-literal twin). In ImcRemap the slots live in
/// distinct bank homes, nothing is re-handed, and the rebind still hits
/// (strictly cheaper) — both faces in one body.
#[test]
fn c12_rehand_miss_frame_hit_bank() {
    let hit = format!(
        "var a = \"{LIT_X}\"\nvar t = \"{LIT_Y}\"\na = \"{LIT_X}\"\nreturn (size(a) + size(t)) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\nvar t = \"{LIT_Y}\"\na = \"{LIT_Z}\"\nreturn (size(a) + size(t)) as bytes"
    );
    // Im / Imc: the Y bind re-handed the watched pool pair → miss.
    for mode in [Rv64Mode::Im, Rv64Mode::Imc] {
        let (lh, ld) = (code_len_mode(&hit, mode), code_len_mode(&diff, mode));
        assert_eq!(lh, ld, "frame re-hand miss mode {mode:?}: {lh} != {ld}");
    }
    // ImcRemap: distinct bank homes survive the Y bind → hit.
    let (lh, ld) = (
        code_len_mode(&hit, Rv64Mode::ImcRemap),
        code_len_mode(&diff, Rv64Mode::ImcRemap),
    );
    assert!(lh < ld, "bank rebind unaffected by Y bind: {lh} !< {ld}");
    assert_ok_u16(&hit, 30, "re-hand hit body result");
    assert_ok_u16(&diff, 30, "re-hand diff body result");
}

/// Three-site same-label chain on the frame path: the second AND third
/// binds hit (the re-issue as Refs neither kills the entry nor moves the
/// depth), so the triple must be strictly cheaper per saved site than
/// the all-distinct twin. ImcRemap misses (distinct homes) — equal.
#[test]
fn c12_three_site_chain_frame_hit() {
    let hit = format!(
        "var a = \"{LIT_X}\"\nvar b = \"{LIT_X}\"\nvar c = \"{LIT_X}\"\nreturn (size(a) + size(b) + size(c)) as bytes"
    );
    let diff = format!(
        "var a = \"{LIT_X}\"\nvar b = \"{LIT_Y}\"\nvar c = \"{LIT_Z}\"\nreturn (size(a) + size(b) + size(c)) as bytes"
    );
    for mode in [Rv64Mode::Im, Rv64Mode::Imc] {
        let (lh, ld) = (code_len_mode(&hit, mode), code_len_mode(&diff, mode));
        assert!(lh < ld, "three-site frame hit mode {mode:?}: {lh} !< {ld}");
    }
    let (lh, ld) = (
        code_len_mode(&hit, Rv64Mode::ImcRemap),
        code_len_mode(&diff, Rv64Mode::ImcRemap),
    );
    assert_eq!(lh, ld, "three-site bank miss: {lh} != {ld}");
    assert_ok_u16(&hit, 45, "three-site hit result");
    assert_ok_u16(&diff, 45, "three-site diff result");
}
