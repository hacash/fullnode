use super::*;


/* ------------------------------------------------------------------ */
/* P0 debug exports. Pure additions supporting the compiler experiments */
/* (docs.loc/rv64im/10-a0-maxlive-and-licm.md): the WebWalk statement    */
/* timeline, the per-statement code-offset trace, and a mirror of the    */
/* ImcRemap promotion plan. Nothing in the lowering reads these; the     */
/* production entry point discards the side-channel, and equality of    */
/* the emitted code is pinned by the `dbg_engine_matches_production`    */
/* test.                                                                */
/* ------------------------------------------------------------------ */

/// `WebEvDbg::kind`: the slot's value is read.
pub const WEB_EV_READ: u8 = 0;
/// `WebEvDbg::kind`: unconditional write (dominates later reads on its path).
pub const WEB_EV_WRITE_U: u8 = 1;
/// `WebEvDbg::kind`: write under if/choose arms (conditional).
pub const WEB_EV_WRITE_C: u8 = 2;

/// `LowerDbg::slot_tys` element mirrors (`SlotTy`).
pub const SLOT_TY_NONE: u8 = 0;
pub const SLOT_TY_INT: u8 = 1;
pub const SLOT_TY_BYTES: u8 = 2;
/// The kernel's `data: bytes` parameter (lives in the ABI input registers).
pub const SLOT_TY_INPUT: u8 = 3;
/// P3-V2IMPL: compo-list slot (the admitted v2 shape set).
pub const SLOT_TY_LIST: u8 = 4;
/// T16: an arena map descriptor slot.
pub const SLOT_TY_MAP: u8 = 5;
/// T13 stain fix: an Address-stained local slot (runtime shape == Bytes).
pub const SLOT_TY_ADDRESS: u8 = 6;

/// `LowerDbg::banked` element mirrors (`BankKind`).
pub const BANK_KIND_NONE: u8 = 0;
pub const BANK_KIND_INT: u8 = 1;
pub const BANK_KIND_BYTES: u8 = 2;

/// One timeline event of a slot (mirror of `WebWalk::Ev`).
#[derive(Clone, Copy, Debug)]
pub struct WebEvDbg {
    /// statement index (mirrors `WebWalk::walk_stmt` numbering)
    pub idx: usize,
    /// one of `WEB_EV_*`
    pub kind: u8,
    /// region id (while bodies get their own; see `region_parent`)
    pub region: usize,
}

/// The WebWalk statement timeline (the def-use / liveness infrastructure the
/// dead-store pass and the bank plan are built on), exported verbatim.
#[derive(Clone, Debug)]
pub struct WebTimelineDbg {
    /// per-slot type (one of `SLOT_TY_*`); index = slot id
    pub slot_tys: Vec<u8>,
    /// per-slot event list in statement order
    pub ev: Vec<Vec<WebEvDbg>>,
    /// per while statement: (loop-head idx — the condition's reads —, last
    /// body idx); the backedge folds [a, b] circularly
    pub while_spans: Vec<(usize, usize)>,
    /// per slot: read by some while condition (loop-carried)
    pub cond_read: Vec<bool>,
    /// region id → parent region id (`usize::MAX` for the root)
    pub region_parent: Vec<usize>,
    /// final statement index = number of statement nodes walked
    pub stmt_total: usize,
}

/// One lowered statement (or const-select table run): the emitted bytes
/// `[off_start, off_end)` belong to walk-index range `[w_lo, w_hi]`.
#[derive(Clone, Copy, Debug)]
pub struct StmtRangeDbg {
    pub w_lo: usize,
    pub w_hi: usize,
    pub off_start: usize,
    pub off_end: usize,
}

/// Debug side-channel of one lowering.
#[derive(Clone, Debug)]
pub struct LowerDbg {
    pub timeline: WebTimelineDbg,
    /// per emitted unit, in emission order (nested statement ranges overlap)
    pub stmt_ranges: Vec<StmtRangeDbg>,
    /// per-slot bank kind (one of `BANK_KIND_*`)
    pub banked: Vec<u8>,
    /// the register vector handed to the web plan (pool-carved; the literal
    /// pool base is popped off first when active)
    pub bank_regs: Vec<u8>,
    /// value-pool register count (peak expression depth, min 1 cap 8)
    pub pool_size: usize,
    /// literal-pool base register, when the pool-base scheme is active
    pub pool_base_reg: Option<u8>,
    /// frame byte offset per slot (-1 = not in the frame: banked/input)
    pub frame_off: Vec<i32>,
    /// literal sites found by `count_literal_sites` (drives the pool base)
    pub lit_sites: usize,
    /// A-19 batch 2 applied dominated constant-mul check merges (the
    /// mechanical-proof pin set: base register × constant pair × interval)
    pub mul_merges: Vec<MulMergeRecord>,
    /// P3-GUARDTAIL GT-a applied guard-copy clusters (thunk code × copy
    /// position × form × converted site offsets)
    pub guard_copies: Vec<GuardCopyRecord>,
}

/// Literal value of an int IR node (debug wrapper over the crate-internal
/// `int_literal` helper — same extraction the const-select/max-min fusion
/// matches use).
pub fn dbg_int_literal(node: &dyn IRNode) -> Option<u64> {
    int_literal(node)
}

/* ------------------------------------------------------------------ */
/* Tests: encoder round-trips, subset pinning, e2e execution, and the  */
/* HVM-vs-RV64 per-iteration comparison report.                        */
/* ------------------------------------------------------------------ */


#[test]
pub(crate) fn ctr_probe_plan_kinds() {
    let body = "var l = [1u64, 2u64]\nvar m = l\nappend(m, 3u64)\nreturn length(l) as bytes";
    let tokens = crate::lang::Tokenizer::new(body.as_bytes()).parse().unwrap();
    let (ir, _, _) = crate::fitshc::compile_body(
        tokens,
        vec![("data".to_string(), crate::value::ValueTy::Bytes)],
        &[], &[], false,
    ).unwrap();
    let fp = plan(&ir.subs, true, crate::rt::HEAP_GROW_SEGMENT_DEFAULT, true).unwrap();
    println!("slots: {:?}", fp.slot);
    println!("kinds: {:?}", fp.list.kind);
    println!("ew:    {:?}", fp.list.ew);
    // slot 0 is the `data` input parameter; the list literal is slot 1
    // and its alias slot 2 — both take the descriptor class (this holds on
    // the default entry, which now IS the container model).
    assert!(matches!(fp.list.kind[1], ListKind::Arena { .. }), "slot l must be Arena");
    assert!(matches!(fp.list.kind[2], ListKind::Arena { .. }), "slot m must be Arena");
}
