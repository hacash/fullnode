//! P2-A19 batch 2: dominated constant-mul overflow-check merging — the
//! three mechanical release proofs, pinned on the anchor kernels
//! (main-agent ruling 2026-11-14; docs.loc/rv64im/15-p2-a19-guard-compression.md).
//!
//! The merge itself (`vm::ir::rv64::merge_dominated_mul_checks`, wired into
//! the relaxation loop) proves, per deletion:
//!   (a) constant domination — k_dominated < k_retained on the SAME base
//!       register; k1 < k2 implies k1*x overflows => k2*x overflows
//!       (x >= ceil(2^64/k1) >= ceil(2^64/k2));
//!   (b) same-base def-use — no instruction between the sites writes the
//!       base register (both checks read the same value);
//!   (c) interval scan — only pure compute, frame `sd` (non-observable),
//!       and arith-thunk guard branches (exit 90) sit between; arena/out
//!       stores, ecall, and foreign control flow veto.
//!
//! These tests re-verify (a) arithmetically from the recorded table, pin
//! the merge-site set (any change fails), pin the interval classes of
//! every record (b+c as observed facts), and pin the census bytes. The
//! behavioral trap-timing boundary pins (interpreter oracle vs all three
//! rv64 modes) live in `rv64_oracle_ground.rs` (`a19_b2_*`).

use vm::fitshc::compile_body;
use vm::ir::rv64::{
    lower_body_mode_arena_dbg, MulMergeIvKind, Rv64Mode, ARENA_SIZE_MAX,
};
use vm::lang::Tokenizer;
use vm::ValueTy;

const SCORE_SUM_FITSH: &str = include_str!("fitsh_kernels_score_sum.inc");
const HACD200_FITSH: &str = include_str!("fitsh_kernels_hacd200.inc");

fn lower_dbg(body: &str) -> (usize, Vec<vm::ir::rv64::MulMergeRecord>) {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let (ir, _, _) = compile_body(
        tokens,
        vec![("data".to_string(), ValueTy::Bytes)],
        &[],
        &[],
        false,
    )
    .expect("compile body");
    let (lo, dbg) =
        lower_body_mode_arena_dbg(&ir, true, Rv64Mode::ImcRemap, ARENA_SIZE_MAX).expect("lower");
    (lo.stats.code_bytes, dbg.mul_merges)
}

/// ceil(2^64 / k) — the smallest x with k*x >= 2^64.
fn ceil_div_2p64(k: u64) -> u64 {
    // 2^64 does not fit u64: compute ceil((2^64)/k) = (2^64 - 1)/k + 1
    // when k does not divide 2^64 exactly; k a power of two divides exactly.
    if k.is_power_of_two() {
        1u64 << (64 - k.trailing_zeros())
    } else {
        u64::MAX / k + 1
    }
}

/* ---------------- proof pins on the anchor kernels ---------------- */

/// (a)+(b)+(c) pinned on score_sum: the doc-69 E6(i) byte outcome minus the
/// n-chain — the i-chain (6i deleted under 8i, 8i deleted under 10i — the
/// pairwise cascade deletes the same bytes as the direct 6i,8i<10i
/// domination) survives on its u64 slots. The n-chain record (`6n` under
/// `8n`) DISSOLVED with the task-1 A3 width closure: `n` is u16 in the
/// interpreter (size() is U16), so `n*6`/`n*8` are u16 range-checked muls
/// with no mulhu pair for the merge to act on.
#[test]
fn a19_merge_sites_pinned_score_sum() {
    let (bytes, merges) = lower_dbg(SCORE_SUM_FITSH);
    // Size History: 2946 (P2-MEASURE-V3, doc 69) → 2922 (P2-A19-B2 dominated
    // mul-check merges, doc 15) → 2918 (P3-GUARDTAIL GT-a: one 5-site guard
    // copy cluster @0x8e8..0xa2c, −10 site bytes +6 c.j copy placed @0x9ca
    // (final coords; predecessor = unconditional jal @0x9c8 = the safe
    // boundary; the first seal wrote @0x9d0 from a pre-final draft, corrected
    // against LowerDbg.guard_copies[0].copy_at = 2506 = 0x9ca); sibling
    // cluster @0x634..0x7aa has no safe
    // boundary in reach and is not forced; records in LowerDbg.guard_copies,
    // docs.loc 96) → 2930 (task-1 A1 shift closure: the 3 variable-amount
    // u64 shift sites gained the checked-amount guard, +4 B each — the
    // `andi 63` mask became `srli 6` + `bne -> fault_arith`) → 2950
    // (task-1 A3 narrow-width closure: the u16 n-chain swaps u64 checked
    // guards for u16 range checks — which also dissolves the (6,8) n-chain
    // merge — and the u8 shift/pow2-mul sites re-shape; bucket attribution
    // alongside the census pin in rv64_backend_maxlive_census.rs)
    // → 2952 x17-bank fix (audit 3103669): x17 removed from the ImcRemap
    // bank candidate list (the shared divmod/mulmod routines clobber it
    // unsaved), so one web segment that used to home in x17 spills to the
    // frame (+2 B). Im/Imc unaffected.
    // → 2950 E3 (impl/fwd-regalloc): a banked-left commutative site (narrow
    // MUL / bitwise class) skips pop2_dest's materializing c.mv (−2 B); the
    // w == 64 MUL sites keep the mv so the A19 mul-check merge below keeps
    // its copy-feeding-mulhu shape. Im/Imc unaffected.
    assert_eq!(bytes, 2950, "census pin lives in rv64_backend_maxlive_census");
    assert_eq!(merges.len(), 2, "merge site set changed: {merges:#?}");
    let mut table: Vec<(u8, u64, u64)> =
        merges.iter().map(|m| (m.base_reg, m.k_dominated, m.k_retained)).collect();
    table.sort_unstable();
    // the i home carries the (6,8)+(8,10) cascade
    let i_base = table.iter().find(|t| t.1 == 8 && t.2 == 10).expect("8->10 record").0;
    assert_eq!(
        table,
        vec![(i_base, 6, 8), (i_base, 8, 10)],
        "merge site set changed"
    );
    // proof (a), recomputed arithmetically from the recorded table: the
    // dominated trap-input set is contained in the retained one
    for m in &merges {
        assert!(m.k_dominated >= 2 && m.k_retained > m.k_dominated);
        assert!(
            ceil_div_2p64(m.k_dominated) >= ceil_div_2p64(m.k_retained),
            "trap-set containment fails for k1={} k2={}",
            m.k_dominated,
            m.k_retained
        );
    }
    // proofs (b)+(c) as observed facts: every interval instruction is
    // whitelisted (Pure / frame-sd / arith guard) — the classifier vetoed
    // anything else. The (8i -> 10i) interval spans the retained xb
    // add-overflow guard.
    for m in &merges {
        assert!(
            m.interval
                .iter()
                .all(|k| matches!(k, MulMergeIvKind::Pure | MulMergeIvKind::FrameSd | MulMergeIvKind::GuardArith)),
            "interval whitelist violated: {:?}",
            m
        );
    }
    let i8_rec = merges
        .iter()
        .find(|m| m.base_reg == i_base && m.k_dominated == 8 && m.k_retained == 10)
        .unwrap();
    assert!(
        i8_rec.interval.iter().filter(|k| **k == MulMergeIvKind::GuardArith).count() >= 1,
        "the 8i->10i interval must span the retained xb add-overflow guard"
    );
}

/// hacd200 has no same-base dominating pair (doc-69 E6(i)): zero merges.
/// Size history: 3072 (P2-HACD200-COLOR, doc 68) → 3052 (P3-IDIOMS M3:
/// the :629 `byte(data,0)*256 + byte(data,1)` BE-2 group fused to one
/// span-guarded lhu — single contiguous code region, attribution table in
/// docs.loc/rv64im/94-a25-m3-byte-idioms.md E2) → 3052 (P3-GUARDTAIL GT-a
/// re-measured to zero: the 6-site guard cluster @0x7a..0x272 is net-positive
/// on paper but no safe boundary (unconditional-jump/trap tail) lies in any
/// profitable copy-reach window — not forced; record in
/// LowerDbg.guard_copies, docs.loc 96) → 3060 (task-1 A1 shift closure:
/// 2 variable-amount u64 shift sites gained the checked-amount guard,
/// +4 B each) → 3068 (task-1 A3 narrow-width closure: the u8 `c1x * 16`
/// pow2-mul guards in the pos-chain re-shape; same attribution as the
/// census pin) → 3082 (narrow-PUT width-shadow closure: the `total` slot
/// is mixed-width written (`if total < 1 { total = 1 }`) and viewed via
/// `total as bytes`, so it gains the 1-byte runtime width shadow — prologue
/// init, the two assignment stores, and the dynamic-length view loop; the
/// view becomes width-exact to the interpreter on the total < 1 inputs)
/// → 3090 (x17-bank fix, audit 3103669: x17 removed from the ImcRemap
/// bank candidate list — the shared divmod/mulmod routines clobber it
/// unsaved — so hacd200's hottest slots re-plan into one fewer bank home,
/// +8 B; Im/Imc unaffected).
#[test]
fn a19_merge_sites_pinned_hacd200_empty() {
    let (bytes, merges) = lower_dbg(HACD200_FITSH);
    assert_eq!(bytes, 3090);
    assert!(merges.is_empty(), "hacd200 gained merge sites: {merges:#?}");
}

/* ---------------- veto pins: the proofs must bite ---------------- */

/// An arena write (a RUNTIME bytes concatenation — literal ++ literal
/// folds at compile time) between the two checks is consensus-observable —
/// the merge must be refused (interval scan veto).
#[test]
fn a19_merge_refused_by_arena_write() {
    let body = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
                var a = x * 6\nvar s = (a as bytes) ++ \"cd\"\nvar b = a + x * 8\nreturn (b + size(s)) as bytes\n";
    let (_, merges) = lower_dbg(body);
    assert!(
        merges.iter().all(|m| m.k_retained != 8 || m.k_dominated != 6),
        "arena store between the checks must veto the 6->8 merge: {merges:#?}"
    );
}

/// A cast guard (exit 71) between the two checks can intercept inputs the
/// deleted 90-check would have caught — exit-code red line, merge refused.
#[test]
fn a19_merge_refused_by_cast_guard_between() {
    let body = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
                var a = x * 6\nvar c = byte(data, 0)\nvar b = a + x * 8\nreturn (b + c) as bytes\n";
    let (_, merges) = lower_dbg(body);
    assert!(
        merges.iter().all(|m| m.k_retained != 8 || m.k_dominated != 6),
        "byte() bounds guard between the checks must veto the merge: {merges:#?}"
    );
}

/// A loop backedge between the checks is foreign control flow — refused
/// (the later check is not straight-line-reachable from the earlier one).
#[test]
fn a19_merge_refused_by_control_flow() {
    let body = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
                var a = x * 6\nvar k = 0u64\nwhile k < 2 { k += 1 }\nvar b = a + x * 8\nreturn (b + k) as bytes\n";
    let (_, merges) = lower_dbg(body);
    assert!(
        merges.iter().all(|m| m.k_retained != 8 || m.k_dominated != 6),
        "control flow between the checks must veto the merge: {merges:#?}"
    );
}

/// The trap-timing boundary bodies used by `rv64_oracle_ground.rs`
/// (`a19_b2_*`) must actually exercise the merge — otherwise those pins
/// would pass vacuously.
#[test]
fn a19_boundary_bodies_do_merge() {
    let n_chain = a19_bodies::N_CHAIN;
    let (_, merges) = lower_dbg(n_chain);
    assert!(
        merges.iter().any(|m| m.k_dominated == 6 && m.k_retained == 8),
        "n-chain boundary body lost its merge: {merges:#?}"
    );
    let i_chain = a19_bodies::I_CHAIN;
    let (_, merges) = lower_dbg(i_chain);
    // the 6i and 8i checks must both be deleted (directly under 10i, or
    // via the 6<8<10 pairwise cascade — same final bytes)
    assert!(
        merges.iter().any(|m| m.k_dominated == 8 && m.k_retained == 10)
            && merges.iter().any(|m| m.k_dominated == 6),
        "i-chain boundary body lost its merges: {merges:#?}"
    );
}

/// Shared bodies for the oracle-ground behavioral pins (duplicated there
/// because integration test binaries do not share code; keep in sync).
mod a19_bodies {
    /// n-chain: `ne = n*6; nue = ne + n*8` (score_sum statements 3-4).
    pub const N_CHAIN: &str = "var n = 0u64\nvar i = 0u64\nwhile i < 8 { n = n * 256 + byte(data, i)\ni += 1 }\n\
        var ne = n * 6\nvar nue = ne + n * 8\nreturn nue as bytes\n";
    /// i-chain: `nb = x*6; xb = nb + x*8; gb = xb + x*10` (score_sum
    /// statements 8-10, unrolled off the loop).
    pub const I_CHAIN: &str = "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
        var a = x * 6\nvar b = a + x * 8\nvar c = b + x * 10\nreturn c as bytes\n";
}

/// Adversarial: a >4 KiB straight-line body between the 8-check and the
/// 10-check forces the relaxation ladder into wide/trampoline forms while
/// the merge spans the whole bulk (interval = pure compute only). The
/// merge must still apply, converge, and stay behaviorally identical to
/// the interpreter oracle (checked model) on the trap boundaries.
#[test]
fn a19_merge_across_relaxation_bulk() {
    let mut body = String::from(
        "var x = 0u64\nvar i = 0u64\nwhile i < 8 { x = x * 256 + byte(data, i)\ni += 1 }\n\
         var a = x * 6\nvar b = a + x * 8\nvar t = b\n",
    );
    // ~2200 add-stements of pure frame compute: pushes the retained 10x
    // check and the guard branches past the ±4 KiB B-type window
    for _ in 0..2200 {
        body.push_str("t = t + 1\n");
    }
    body.push_str("var c = t + x * 10\nreturn c as bytes\n");
    let (bytes, merges) = lower_dbg(&body);
    assert!(
        merges.iter().any(|m| m.k_dominated == 6 && m.k_retained == 8)
            && merges.iter().any(|m| m.k_dominated == 8 && m.k_retained == 10),
        "bulk-relaxation body lost its merges: {merges:#?}"
    );
    assert!(bytes > 4096, "body must actually exceed the B-type window");
}
