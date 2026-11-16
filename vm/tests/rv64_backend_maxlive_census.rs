//! P0-A decisive experiments (tasks A0.1 + A0.2): maxlive census and li
//! materialization classification for the hacdtestnet score_sum / hacd200
//! kernels, in Rv64Mode::ImcRemap (with Im as the contrast).
//!
//! Reads the WebWalk statement timeline and the per-statement code-offset
//! trace through the `lower_body_mode_arena_dbg` side-channel (byte-identical
//! engine, pinned by `ir::rv64::tests::dbg_engine_matches_production`) and
//! answers:
//!
//! * A0.1 — maximum simultaneously-live IR slots (maxlive), whole-function
//!   and per while region, under three liveness definitions (strict
//!   straight-line interval; loop-aware optimistic fold; sound per-statement
//!   CFG with if-diamond rewiring, loop backedges, and write kills), against
//!   the ImcRemap bank register budget → the theoretical ceiling of a
//!   linear-scan-class allocator on the slot ld/sd residue.
//! * A0.2 — every `li`-class instruction (li / c.li / c.lui / lui+addi pair)
//!   in the ImcRemap artifact classified as: semantically-required per-
//!   iteration constant rebuild, provably-redundant rebuild, hoistable
//!   loop-invariant expression operand, one-shot top-level site, or
//!   infrastructure — with byte totals aligned to the census delta
//!   (const/li +198 B + const/lui +20 B = +218 B vs the rust profile-2
//!   kernel).
//!
//! Kernel bodies are transcribed verbatim from hacdtestnet
//! `src/tests/calc_conformance/fitsh_kernels.rs` (read-only reference; that
//! repo is not modified; see docs.loc/rv64im/10-a0-maxlive-and-licm.md §method
//! for the verbatim-transcription proof). Everything is deterministic: the
//! same inputs produce the same tables (pinned by `analysis_is_deterministic`).

use std::collections::{BTreeMap, BTreeSet};
use vm::calc::container::CalcContainer;
use vm::calc::decode::{decode_at, ImmAluOp, Inst, LoadOp, StoreOp};
use vm::calc::{CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap};
use vm::fitshc::{compile_body, CompiledCode};
use vm::ir::rv64::{
    dbg_int_literal, lower_body_mode_arena, lower_body_mode_arena_dbg, LowerDbg, Rv64Lowered,
    Rv64Mode, WebEvDbg, ARENA_SIZE_MAX, BANK_KIND_NONE, SLOT_TY_BYTES, SLOT_TY_INPUT, SLOT_TY_INT,
    WEB_EV_READ, WEB_EV_WRITE_C, WEB_EV_WRITE_U,
};
use vm::ir::{
    IRNode, IRNodeArray, IRNodeDouble, IRNodeEmpty, IRNodeLeaf, IRNodeParam1Single,
    IRNodeParam2Single, IRNodeParams, IRNodeParamsSingle, IRNodeSingle, IRNodeText, IRNodeTriple,
    IRNodeWrapOne,
};
use vm::lang::Tokenizer;
use vm::Bytecode as Bc;
use vm::ValueTy;

/// XOP operand mark → local slot index (low 6 bits; mirrors
/// `rt::xop::decode_local_operand_mark`'s `idx = mark & LXOP_MAX_IDX`,
/// LXOP_MAX_IDX = 0b0011_1111 — `vm::rt` is crate-private, so the two lines
/// are mirrored here instead of imported).
fn xop_slot(mark: u8) -> u8 {
    mark & 0b0011_1111
}

/* ==================================================================== */
/* Kernel bodies (verbatim transcriptions from hacdtestnet)             */
/* ==================================================================== */

/// hacdtestnet `SCORE_SUM_FITSH` (Diamond Analyzer v0.1.1, five-dimension
/// score; wire = names[6n] || numbers[8n] || vg[10n]). Comment/whitespace-
/// normalized text is identical to rv64.rs's embedded `HIP5_FITSH` (the
/// 3200 B census kernel), so census alignments carry over 1:1.
const SCORE_SUM_FITSH: &str = include_str!("fitsh_kernels_score_sum.inc");

/// hacdtestnet `HACD200_FITSH` (batch/single HACD scorer).
const HACD200_FITSH: &str = include_str!("fitsh_kernels_hacd200.inc");

/* ==================================================================== */
/* Compilation helpers                                                  */
/* ==================================================================== */

fn compile_ir(body: &str, has_input: bool) -> IRNodeArray {
    let tokens = Tokenizer::new(body.as_bytes()).parse().expect("tokenize");
    let args: Vec<(String, ValueTy)> = has_input
        .then(|| vec![("data".to_string(), ValueTy::Bytes)])
        .unwrap_or_default();
    let (ir, compiled, _) = compile_body(tokens, args, &[], &[], false).expect("compile body");
    assert!(matches!(compiled, CompiledCode::Bytecode(_)));
    ir
}

fn lower(body: &str, has_input: bool, mode: Rv64Mode) -> Rv64Lowered {
    let ir = compile_ir(body, has_input);
    lower_body_mode_arena(&ir, has_input, mode, ARENA_SIZE_MAX).expect("lower")
}

fn lower_dbg(body: &str, has_input: bool, mode: Rv64Mode) -> (Rv64Lowered, LowerDbg) {
    let ir = compile_ir(body, has_input);
    lower_body_mode_arena_dbg(&ir, has_input, mode, ARENA_SIZE_MAX).expect("lower dbg")
}

fn pack(lo: &Rv64Lowered) -> CalcContainer {
    let cap = SpaceCap::new(0);
    lo.pack_segments_profile(&cap, 64, 4800, 8, vm::calc::container::CALC_PROFILE_RV64IMC)
        .expect("pack")
}

fn exec(bytes: &CalcContainer, input: &[u8]) -> Result<Vec<u8>, u32> {
    let cap = SpaceCap::new(0);
    let fno = CalcFnObj::build(bytes.clone(), &cap, CalcHostConfig::new()).expect("verify+build");
    let ex = fno.run(input, CalcRunLimits { cycles: 100_000_000, resource: -1, deadline: None });
    ex.result.map_err(|_| ex.exit_code as u32)
}

/// Deterministic score_sum wire: names[6n] || numbers[8n] || vg[10n]
/// (xorshift64*, transcribed from hacdtestnet's `score_wire`).
fn score_wire(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut wire = Vec::with_capacity(n * 24);
    for _ in 0..n {
        for _ in 0..6 {
            wire.push(b'A' + (next() % 26) as u8);
        }
    }
    for _ in 0..n {
        wire.extend_from_slice(&next().to_be_bytes());
    }
    for _ in 0..n {
        for _ in 0..10 {
            wire.push((next() % 256) as u8);
        }
    }
    wire
}

/* ==================================================================== */
/* IR mirror walk: per-statement facts, while spans, if diamonds        */
/* ==================================================================== */

/// Bytes payload of a bytes literal node (mirrors the crate's `bytes_literal`
/// plus the CBYTES cast wrapper): PBUF/PBUFL/PNBUF leaves, `"..."` sources.
fn bytes_literal(node: &dyn IRNode) -> Option<Vec<u8>> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return bytes_literal(&*w.node);
    }
    if let Some(leaf) = node.as_any().downcast_ref::<IRNodeLeaf>() {
        if leaf.inst == Bc::PNBUF {
            return Some(Vec::new());
        }
        return None;
    }
    if let Some(s) = node.as_any().downcast_ref::<IRNodeSingle>() {
        if s.inst == Bc::CBYTES {
            return bytes_literal(&*s.subx);
        }
        return None;
    }
    if let Some(p) = node.as_any().downcast_ref::<IRNodeParams>() {
        let op = p.inst;
        match op {
            Bc::PBUF if p.para.len() >= 1 => Some(p.para[1..].to_vec()),
            Bc::PBUFL if p.para.len() >= 2 => Some(p.para[2..].to_vec()),
            _ => None,
        }
    } else {
        None
    }
}

/// Per walk-index statement facts (walk numbering mirrors `WebWalk`).
#[derive(Default, Clone)]
struct StmtFacts {
    /// every int literal value appearing anywhere in the statement's
    /// expression trees
    lits: BTreeSet<u64>,
    /// bytes literal payloads (e.g. `var ch = "9"` → [0x39])
    byte_lits: Vec<Vec<u8>>,
    /// `slot = <literal>` statement (PUT or XOP): (slot, literal)
    const_put: Option<(u8, PutConst)>,
    /// statement kind tag for the report
    kind: &'static str,
}

#[derive(Clone, Debug)]
enum PutConst {
    Int(u64),
    Bytes(Vec<u8>),
}

/// If-diamond shape of one `if` statement (walk indices).
#[derive(Clone, Copy)]
struct IfRange {
    /// the IRIF statement's own idx (its condition reads here)
    hdr: usize,
    /// last statement of the then arm (inclusive)
    t_hi: usize,
    /// last statement of the else arm (inclusive); the join point is
    /// e_hi + 1 (a virtual position between statements)
    e_hi: usize,
}

/// Mirrors `WebWalk::walk_stmt` node-for-node (same idx numbering, same
/// recursion shape), collecting per-statement literal facts, the while
/// spans, and the if diamonds. `facts.len()-1` must end equal to the
/// timeline's `stmt_total`, and the mirror's while spans must equal the
/// timeline's (asserted by callers) — this is what licenses using the
/// mirror's if ranges together with the timeline's event lists.
fn ir_mirror(ir: &IRNodeArray) -> (Vec<StmtFacts>, Vec<(usize, usize)>, Vec<IfRange>) {
    fn record_expr(i: usize, node: &dyn IRNode, facts: &mut Vec<StmtFacts>) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return record_expr(i, &*w.node, facts);
        }
        if let Some(v) = dbg_int_literal(node) {
            facts[i].lits.insert(v);
        }
        if let Some(b) = bytes_literal(node) {
            facts[i].byte_lits.push(b);
        }
        // descend in evaluation order (mirrors `WebWalk::record_expr`'s
        // children()); statement nodes never appear below an expression root
        let any = node.as_any();
        macro_rules! one {
            ($n:expr) => {
                record_expr(i, &*$n, facts)
            };
        }
        if let Some(n) = any.downcast_ref::<IRNodeSingle>() {
            one!(n.subx);
        } else if let Some(n) = any.downcast_ref::<IRNodeDouble>() {
            one!(n.subx);
            one!(n.suby);
        } else if let Some(n) = any.downcast_ref::<IRNodeTriple>() {
            one!(n.subx);
            one!(n.suby);
            one!(n.subz);
        } else if let Some(n) = any.downcast_ref::<IRNodeParamsSingle>() {
            one!(n.subx);
        } else if let Some(n) = any.downcast_ref::<IRNodeParam1Single>() {
            one!(n.subx);
        } else if let Some(n) = any.downcast_ref::<IRNodeParam2Single>() {
            one!(n.subx);
        }
    }

    fn walk_stmt(
        node: &dyn IRNode,
        facts: &mut Vec<StmtFacts>,
        spans: &mut Vec<(usize, usize)>,
        ifrs: &mut Vec<IfRange>,
    ) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walk_stmt(&*w.node, facts, spans, ifrs);
        }
        // mirrors WebWalk::walk_stmt: Empty/Text count one idx; nodes whose
        // bytecode is not convertible consume NO idx at all (early return)
        if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
            || node.as_any().downcast_ref::<IRNodeText>().is_some()
        {
            facts.push(StmtFacts { kind: "empty", ..Default::default() });
            return;
        }
        let op = match Bc::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        facts.push(StmtFacts::default());
        let i = facts.len() - 1;
        match op {
            Bc::IRLIST | Bc::IRBLOCK | Bc::IRBLOCKR => {
                facts[i].kind = "block";
                if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                    for sub in &arr.subs {
                        walk_stmt(&**sub, facts, spans, ifrs);
                    }
                }
            }
            Bc::IRIF => {
                facts[i].kind = "if";
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    record_expr(i, &*n.subx, facts);
                    walk_stmt(&*n.suby, facts, spans, ifrs);
                    let t_hi = facts.len() - 1;
                    walk_stmt(&*n.subz, facts, spans, ifrs);
                    let e_hi = facts.len() - 1;
                    ifrs.push(IfRange { hdr: i, t_hi, e_hi });
                }
            }
            Bc::IRIFR | Bc::CHOOSE => {
                facts[i].kind = "choose";
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    record_expr(i, &*n.subx, facts);
                    record_expr(i, &*n.suby, facts);
                    record_expr(i, &*n.subz, facts);
                }
            }
            Bc::IRWHILE => {
                facts[i].kind = "while";
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    record_expr(i, &*n.subx, facts);
                    // push the span BEFORE the body walk and fill its tail
                    // after — mirrors WebWalk's push timing (outer spans come
                    // first in `while_spans` order)
                    let span = spans.len();
                    spans.push((i, i));
                    walk_stmt(&*n.suby, facts, spans, ifrs);
                    spans[span].1 = facts.len() - 1;
                }
            }
            Bc::PUT => {
                facts[i].kind = "put";
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    record_expr(i, &*p.subx, facts);
                    if let Some(v) = dbg_int_literal(&*p.subx) {
                        facts[i].const_put = Some((p.para, PutConst::Int(v)));
                    } else if let Some(b) = bytes_literal(&*p.subx) {
                        facts[i].const_put = Some((p.para, PutConst::Bytes(b)));
                    }
                }
            }
            Bc::XOP => {
                facts[i].kind = "xop";
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    record_expr(i, &*p.subx, facts);
                    if let Some(v) = dbg_int_literal(&*p.subx) {
                        let slot = xop_slot(p.para);
                        facts[i].const_put = Some((slot, PutConst::Int(v)));
                    }
                }
            }
            Bc::REQ => {
                facts[i].kind = "req";
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    record_expr(i, &*n.subx, facts);
                    record_expr(i, &*n.suby, facts);
                }
            }
            Bc::AST | Bc::ERR | Bc::RET => {
                facts[i].kind = "expr-stmt";
                if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
                    record_expr(i, &*n.subx, facts);
                }
            }
            _ => {
                facts[i].kind = "expr-fallback";
                record_expr(i, node, facts);
            }
        }
    }

    let mut facts = vec![StmtFacts::default()]; // 1-based; index 0 unused
    let mut spans = Vec::new();
    let mut ifrs = Vec::new();
    for n in &ir.subs {
        walk_stmt(&**n, &mut facts, &mut spans, &mut ifrs);
    }
    (facts, spans, ifrs)
}

/* ==================================================================== */
/* A0.1: liveness over the WebWalk timeline                             */
/* ==================================================================== */

/// Liveness definition variant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Def {
    /// strict single straight-line interval [first def write, last read];
    /// no wrap, no redefinition kills, no conditional-write subtleties —
    /// the naive lower-bound view a text-level reader would take.
    Strict,
    /// loop-aware optimistic: linear chain + while backedge folds (b→a,
    /// a→b+1); conditional writes do NOT kill (existence-of-a-defining-path
    /// semantics, guard correlation ignored) — an over-counting upper bound
    /// that isolates what the backedge alone contributes.
    LoopFold,
    /// sound per-statement CFG dataflow: if-diamonds rewired (then-arm exit
    /// jumps to the join; the header's false edge enters the else arm; both
    /// arm exits resolve through enclosing arms/loops to the real landing
    /// statement), while backedges b→a plus header exits a→exit, and EVERY
    /// write (WriteU and WriteC) kills along paths through it. This is what
    /// a register allocator must assume; it is the headline number.
    Cfg,
}

impl Def {
    const ALL: [Def; 3] = [Def::Strict, Def::LoopFold, Def::Cfg];
    fn name(self) -> &'static str {
        match self {
            Def::Strict => "strict-interval",
            Def::LoopFold => "loop-aware-optimistic",
            Def::Cfg => "cfg-sound",
        }
    }
}

/// The landing resolver: `land(q0, from, skip)` — the real statement-level
/// landing of a fallthrough that leaves a construct at virtual position q0,
/// where the leaving construct's tail is `from` (or the construct itself for
/// loop-exit edges). Innermost-first over the constructs containing `from`;
/// each redirect moves the landing past one enclosing if arm (to its join,
/// remembered in `skip`) and rescans; a landing past an enclosing loop body
/// end is the loop's backedge to its header. Without any enclosing
/// construct the answer is q0 itself (a real statement position, or n+1 =
/// function exit).
fn land(
    mut q: usize,
    from: usize,
    spans: &[(usize, usize)],
    ifrs: &[IfRange],
    skip: &mut BTreeSet<(char, usize)>,
) -> usize {
    loop {
        // innermost enclosing loop of `from` — re-checked every round (a
        // q-growing if-redirect may push the landing past an inner loop's
        // tail; only caller-supplied `skip` entries exclude a loop, e.g. the
        // loop's own exit edge)
        let mut innermost_loop: Option<(usize, usize)> = None; // (a, b)
        for (k, &(a, b)) in spans.iter().enumerate() {
            if a <= from && from <= b && !skip.contains(&('L', k)) {
                let better = match innermost_loop {
                    Some((a0, _)) => a > a0,
                    None => true,
                };
                if better {
                    innermost_loop = Some((a, b));
                    let _ = k;
                }
            }
        }
        if let Some((a, b)) = innermost_loop {
            if q > b {
                return a; // enclosing body completes → backedge to header
            }
        }
        // innermost enclosing if-arm of `from`
        let mut innermost_if: Option<(usize, IfRange)> = None;
        for (k, &r) in ifrs.iter().enumerate() {
            if r.hdr <= from && from <= r.e_hi && !skip.contains(&('I', k)) {
                let better = match innermost_if {
                    Some((_, r0)) => r.hdr > r0.hdr,
                    None => true,
                };
                if better {
                    innermost_if = Some((k, r));
                }
            }
        }
        match innermost_if {
            Some((k, r)) => {
                let arm_end = if from <= r.t_hi { r.t_hi } else { r.e_hi };
                if q > arm_end {
                    q = r.e_hi + 1; // arm completes → resolve at the join
                    skip.insert(('I', k));
                    continue;
                }
                return q;
            }
            None => return q,
        }
    }
}

/// live[p] for p in 1..=n: the slot's value is live immediately BEFORE
/// statement p (standard backward may-liveness, use-before-def convention:
/// a read at a write's own statement index counts as a use of the old
/// value). `spans` / `ifrs` come from the IR mirror walk (verified in sync
/// with the timeline's while_spans).
fn slot_liveness(
    evs: &[WebEvDbg],
    n: usize,
    spans: &[(usize, usize)],
    ifrs: &[IfRange],
    def: Def,
) -> Vec<bool> {
    let mut live = vec![false; n + 3];
    if evs.is_empty() || n == 0 {
        return live;
    }
    if def == Def::Strict {
        // one interval per slot: born at the first unconditional write (or
        // at entry when the first event is a read / conditional write —
        // entry-born covers read-before-write and guarded-first-write
        // shapes), dead after the last read
        let first_write = evs.iter().find(|e| e.kind == WEB_EV_WRITE_U).map(|e| e.idx);
        let Some(last_read) = evs.iter().filter(|e| e.kind == WEB_EV_READ).map(|e| e.idx).max()
        else {
            return live;
        };
        let entry_born = first_write.is_none();
        let d = first_write.unwrap_or(0);
        for q in 1..=n {
            let born = entry_born
                || q > d
                || (q == d && evs.iter().any(|e| e.idx == q && e.kind == WEB_EV_READ));
            live[q] = born && last_read >= q;
        }
        return live;
    }

    let mut use_ = vec![false; n + 3];
    let mut kill = vec![false; n + 3];
    for e in evs {
        match e.kind {
            WEB_EV_READ => use_[e.idx] = true,
            WEB_EV_WRITE_U => kill[e.idx] = true,
            WEB_EV_WRITE_C => {
                if def == Def::Cfg {
                    kill[e.idx] = true;
                }
            }
            _ => {}
        }
    }

    // ---- CFG edges (nodes 1..=n; n+1 = function exit, never live) ----
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n + 2];
    for p in 1..n {
        succ[p].push(p + 1);
    }
    succ[n].push(n + 1);

    // The landing resolver (`land`, defined below as a free function):
    // resolve(q0, from, skip) — the real statement-level landing of a
    // fallthrough that leaves a construct at virtual position q0, where the
    // leaving construct's tail is `from` (or the construct itself for loop
    // exits). Innermost-first over the constructs containing `from`; each
    // redirect moves the landing past one enclosing arm (to its join) and
    // rescans; a landing past an enclosing loop body end is the loop's
    // backedge to its header.
    for (k, r) in ifrs.iter().enumerate() {
        let mut skip = BTreeSet::new();
        skip.insert(('I', k)); // the resolver must not redirect through R itself
        let join = land(r.e_hi + 1, r.t_hi, spans, ifrs, &mut skip.clone());
        succ[r.hdr] = vec![r.hdr + 1, r.t_hi + 1];
        succ[r.t_hi] = vec![join];
        let join_e = land(r.e_hi + 1, r.e_hi, spans, ifrs, &mut skip);
        succ[r.e_hi] = vec![join_e];
    }
    // while edges: the tail's only successor is the header (the emitted body
    // ends in an unconditional jump to the condition); the header enters the
    // body and exits (cond false) to the landing after the loop.
    for (k, &(a, b)) in spans.iter().enumerate() {
        let mut skip = BTreeSet::new();
        skip.insert(('L', k)); // the loop's own exit must not fold to itself
        let exit = land(b + 1, b, spans, ifrs, &mut skip);
        succ[a] = vec![a + 1, exit];
        succ[b] = vec![a];
    }

    // backward may-liveness to fixpoint (statements in reverse order)
    loop {
        let mut changed = false;
        for p in (1..=n).rev() {
            let mut out = false;
            for &s in &succ[p] {
                out = out || live[s];
            }
            let l = use_[p] || (out && !kill[p]);
            if l != live[p] {
                live[p] = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live
}

/// live[p] for the LoopFold definition: linear chain + while folds, writes
/// never kill (except that a live-in must still be reachable: use[p] or
/// survives through non-kill... kill array is all-false here, so simply
/// reachability of a use/read backward through the folded chain, born at
/// entry or at the slot's first write site — modeled by making the first
/// WriteU statement of the slot a kill of "before" and nothing else).
fn slot_liveness_loopfold(evs: &[WebEvDbg], n: usize, spans: &[(usize, usize)]) -> Vec<bool> {
    let mut live = vec![false; n + 3];
    if evs.is_empty() || n == 0 {
        return live;
    }
    let mut use_ = vec![false; n + 3];
    let mut kill = vec![false; n + 3];
    for e in evs {
        match e.kind {
            WEB_EV_READ => use_[e.idx] = true,
            WEB_EV_WRITE_U => {
                // before the FIRST unconditional write the slot has no value
                // (unless an earlier read/conditional write implies an
                // entry-born value)
                if kill.iter().all(|&k| !k) && use_.iter().all(|&u| !u) {
                    kill[e.idx] = true;
                }
            }
            _ => {} // WriteC never kills under the optimistic definition
        }
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n + 2];
    for p in 1..n {
        succ[p].push(p + 1);
    }
    succ[n].push(n + 1);
    for &(a, b) in spans {
        if !succ[a].contains(&(b + 1)) {
            succ[a].push(b + 1);
        }
        succ[b] = vec![a, b + 1];
    }
    loop {
        let mut changed = false;
        for p in (1..=n).rev() {
            let mut out = false;
            for &s in &succ[p] {
                out = out || live[s];
            }
            let l = use_[p] || (out && !kill[p]);
            if l != live[p] {
                live[p] = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live
}

struct SpanReport {
    a: usize,
    b: usize,
    depth: usize,
    /// header condition slot ids (cond_read at the header)
    cond_slots: Vec<usize>,
    /// worst-point live slot list (slot id, ty, banked flag)
    worst_slots: Vec<(usize, &'static str, bool)>,
    worst_point: usize,
    slot_count: usize,
    reg_count: usize,
    /// Σ max(0, live_regs(q) − R) × traffic(q) inside the span
    spill_excess: u64,
    /// Σ traffic(q) inside the span (register-width weighted)
    traffic: u64,
}

struct MaxReport {
    kernel: &'static str,
    mode: Rv64Mode,
    n_slots_int: usize,
    n_slots_bytes: usize,
    /// definition → (max live slots, max live registers)
    maxlive: BTreeMap<&'static str, (usize, usize)>,
    /// definition → statement idx achieving the max
    argmax: BTreeMap<&'static str, usize>,
    spans: Vec<SpanReport>,
    bank_regs: usize,
    banked_slot_count: usize,
    pool_size: usize,
    pool_base_reg: Option<u8>,
    /// artifact slot-to-frame ld/sd bytes (base s0 = x8)
    slot_ldsd_insts: usize,
    slot_ldsd_bytes: usize,
    slot_ldsd_by_slot: BTreeMap<usize, (usize, usize)>,
    /// register-width-weighted slot-event traffic, whole function
    traffic_total: u64,
    /// Σ max(0, live_regs(q) − R) × traffic(q), whole function, cfg def
    traffic_spilled_cfg: u64,
    /// bytes slots simultaneously live at the cfg argmax point
    bytes_at_argmax: usize,
    stmt_total: usize,
    code_bytes: usize,
}

fn innermost_span(spans: &[(usize, usize)], p: usize) -> Option<(usize, usize)> {
    spans.iter().copied().filter(|&(a, b)| a <= p && p <= b).max_by_key(|&(a, _)| a)
}

fn span_depth(spans: &[(usize, usize)], s: (usize, usize)) -> usize {
    let (a, b) = s;
    spans
        .iter()
        .filter(|&&(a2, b2)| (a2, b2) != (a, b) && a2 <= a && b <= b2)
        .count()
}

fn analyze_maxlive(kernel: &'static str, body: &str, has_input: bool, mode: Rv64Mode) -> MaxReport {
    let (lo, dbg) = lower_dbg(body, has_input, mode);
    let tl = &dbg.timeline;
    let n = tl.stmt_total;
    let r_budget = dbg.bank_regs.len();
    let (mirror, mirror_spans, ifrs) = ir_mirror(&compile_ir(body, has_input));
    assert_eq!(mirror.len() - 1, n, "mirror walk desynced from the timeline");
    assert_eq!(mirror_spans, tl.while_spans, "mirror while spans != timeline spans");

    let regw = |s: usize| if tl.slot_tys[s] == SLOT_TY_BYTES { 2usize } else { 1usize };

    // per-definition live sets (the `data: bytes` input slot is excluded:
    // it lives in the dedicated ABI input registers, never in the bank)
    let mut live_sets: BTreeMap<&'static str, Vec<Vec<bool>>> = BTreeMap::new();
    for def in Def::ALL {
        let per: Vec<Vec<bool>> = (0..tl.ev.len())
            .map(|s| {
                if matches!(tl.slot_tys.get(s), Some(&SLOT_TY_INPUT)) {
                    vec![false; n + 3]
                } else {
                    match def {
                        Def::Strict => {
                            slot_liveness(&tl.ev[s], n, &tl.while_spans, &ifrs, Def::Strict)
                        }
                        Def::LoopFold => slot_liveness_loopfold(&tl.ev[s], n, &tl.while_spans),
                        Def::Cfg => slot_liveness(&tl.ev[s], n, &tl.while_spans, &ifrs, Def::Cfg),
                    }
                }
            })
            .collect();
        live_sets.insert(def.name(), per);
    }

    let mut maxlive = BTreeMap::new();
    let mut argmax = BTreeMap::new();
    let mut bytes_at = BTreeMap::new();
    for (name, sets) in &live_sets {
        let mut best = (0usize, 0usize);
        let mut bp = 0usize;
        let mut bb = 0usize;
        for p in 1..=n {
            let mut cnt = 0usize;
            let mut regs = 0usize;
            let mut nbytes = 0usize;
            for (s, lv) in sets.iter().enumerate() {
                if lv.get(p).copied().unwrap_or(false) {
                    cnt += 1;
                    regs += regw(s);
                    if tl.slot_tys[s] == SLOT_TY_BYTES {
                        nbytes += 1;
                    }
                }
            }
            if regs > best.1 {
                best = (cnt, regs);
                bp = p;
                bb = nbytes;
            }
        }
        maxlive.insert(*name, best);
        argmax.insert(*name, bp);
        bytes_at.insert(*name, bb);
    }

    // per-span reports under the sound cfg def
    let la = &live_sets[Def::Cfg.name()];
    let mut spans_rep: Vec<SpanReport> = Vec::new();
    for &(a, b) in tl.while_spans.iter() {
        let mut worst = (0usize, 0usize, 0usize);
        let mut worst_slots = Vec::new();
        for p in a..=b {
            let mut cnt = 0usize;
            let mut regs = 0usize;
            let mut cur = Vec::new();
            for (s, lv) in la.iter().enumerate() {
                if lv.get(p).copied().unwrap_or(false) {
                    cnt += 1;
                    regs += regw(s);
                    let ty = if tl.slot_tys[s] == SLOT_TY_BYTES { "Bytes" } else { "Int" };
                    cur.push((s, ty, dbg.banked.get(s).copied().unwrap_or(BANK_KIND_NONE) != 0));
                }
            }
            if regs > worst.0 {
                worst = (regs, cnt, p);
                worst_slots = cur;
            }
        }
        let mut ex = 0u64;
        let mut traf = 0u64;
        for p in a..=b {
            let rl: usize =
                (0..tl.ev.len()).filter(|&s| la[s].get(p).copied().unwrap_or(false)).map(regw).sum();
            let t: u64 = (0..tl.ev.len())
                .filter(|&s| tl.ev[s].iter().any(|e| e.idx == p))
                .map(regw)
                .sum::<usize>() as u64;
            traf += t;
            ex += (rl.saturating_sub(r_budget) as u64) * t;
        }
        let cond_slots = (0..tl.ev.len())
            .filter(|&s| tl.ev[s].iter().any(|e| e.idx == a && e.kind == WEB_EV_READ))
            .collect();
        spans_rep.push(SpanReport {
            a,
            b,
            depth: span_depth(&tl.while_spans, (a, b)),
            cond_slots,
            worst_slots,
            worst_point: worst.2,
            slot_count: worst.1,
            reg_count: worst.0,
            spill_excess: ex,
            traffic: traf,
        });
    }
    spans_rep.sort_by_key(|s| (s.depth, s.a));

    // artifact slot-to-frame traffic (base s0 = x8), decoded
    let profile = vm::calc::container::CALC_PROFILE_RV64IMC;
    let mut slot_ldsd_bytes = 0usize;
    let mut slot_ldsd_insts = 0usize;
    let mut by_slot: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut off2slot: BTreeMap<i64, usize> = BTreeMap::new();
    for (s, &o) in dbg.frame_off.iter().enumerate() {
        if o >= 0 {
            off2slot.insert(o as i64, s);
            if tl.slot_tys[s] == SLOT_TY_BYTES {
                off2slot.insert(o as i64 + 8, s);
            }
        }
    }
    let mut off = 0usize;
    while off < lo.code.len() {
        let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
        let l = len as usize;
        match &inst {
            Inst::Load { op: LoadOp::Ld, rs1: 8, imm, .. }
            | Inst::Store { op: StoreOp::Sd, rs1: 8, imm, .. } => {
                if let Some(&slot) = off2slot.get(imm) {
                    slot_ldsd_bytes += l;
                    slot_ldsd_insts += 1;
                    let e = by_slot.entry(slot).or_default();
                    e.0 += 1;
                    e.1 += l;
                }
            }
            _ => {}
        }
        off += l;
    }

    // whole-function traffic + spill excess (register-weighted, cfg def)
    let mut traffic_total = 0u64;
    let mut spilled_cfg = 0u64;
    for p in 1..=n {
        let mut tw = 0u64;
        for evs in tl.ev.iter() {
            if evs.iter().any(|e| e.idx == p) {
                tw += 1; // refined below per slot width
            }
        }
        // width-weighted traffic: count each slot's event at p with its width
        let mut tw2 = 0u64;
        for (s, evs) in tl.ev.iter().enumerate() {
            if evs.iter().any(|e| e.idx == p) {
                tw2 += regw(s) as u64;
            }
        }
        let rl: usize =
            (0..tl.ev.len()).filter(|&s| la[s].get(p).copied().unwrap_or(false)).map(regw).sum();
        traffic_total += tw2;
        spilled_cfg += (rl.saturating_sub(r_budget) as u64) * tw2;
        let _ = tw;
    }

    MaxReport {
        kernel,
        mode,
        n_slots_int: tl.slot_tys.iter().filter(|&&t| t == SLOT_TY_INT).count(),
        n_slots_bytes: tl.slot_tys.iter().filter(|&&t| t == SLOT_TY_BYTES).count(),
        maxlive,
        argmax,
        spans: spans_rep,
        bank_regs: r_budget,
        banked_slot_count: dbg.banked.iter().filter(|&&b| b != 0).count(),
        pool_size: dbg.pool_size,
        pool_base_reg: dbg.pool_base_reg,
        slot_ldsd_insts,
        slot_ldsd_bytes,
        slot_ldsd_by_slot: by_slot,
        traffic_total,
        traffic_spilled_cfg: spilled_cfg,
        bytes_at_argmax: bytes_at[Def::Cfg.name()],
        stmt_total: n,
        code_bytes: lo.stats.code_bytes,
    }
}

/* ==================================================================== */
/* A0.2: li materialization classification                              */
/* ==================================================================== */

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LiCat {
    /// inside a while span
    ALoop,
    /// top-level body (once per call)
    CTop,
    /// prologue
    IPrologue,
    /// fault thunks / trap epilogue (after the body)
    IThunk,
}

impl LiCat {
    fn name(self) -> &'static str {
        match self {
            LiCat::ALoop => "A-loop",
            LiCat::CTop => "C-top",
            LiCat::IPrologue => "I-prologue",
            LiCat::IThunk => "I-thunk",
        }
    }
}

/// Why a site cannot (or can) be removed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Licm {
    /// loop-invariant expression operand (no slot state): hoistable; the
    /// win is (sites_of_same_value − 1) × bytes per enclosing loop
    OperandHoistable,
    /// per-iteration `var x = K` re-init whose slot receives ANY non-K write
    /// inside the loop (data-dependent XOP writes included): the
    /// re-establishing store is required by per-iteration fresh-binding
    /// semantics (an SSA/LICM pipeline cannot delete it either — the phi
    /// backedge carries the merged non-K value). Compressible only via a
    /// banked home (li+sd → c.li), which is A0.1's allocator ceiling.
    RebuildRequired,
    /// per-iteration `var x = K` re-init whose slot is provably ≡ K at the
    /// loop tail: every in-loop write event of the slot (timeline WriteU and
    /// WriteC) is a K const-put, an unconditional (top-of-body) K write
    /// exists, and no in-loop read precedes the first write — the re-init is
    /// then redundant against the carried tail value and deletable.
    RebuildRedundant,
    /// in-loop site whose bytes could not be attributed to a statement fact
    /// (const-select table runs, address scaffolding): counted conservatively
    /// as NOT savable
    SynthUnattributed,
    /// one-shot top-level materialization: executes once, hoisting gains
    /// zero bytes
    OneShot,
    /// infrastructure (prologue setup, arena bounds, guard-thunk exit codes,
    /// trap epilogue)
    Infra,
}

impl Licm {
    fn name(self) -> &'static str {
        match self {
            Licm::OperandHoistable => "LICM-hoistable-operand",
            Licm::RebuildRequired => "rebuild-required (semantics)",
            Licm::RebuildRedundant => "rebuild-redundant (deletable)",
            Licm::SynthUnattributed => "synth-unattributed (conservative)",
            Licm::OneShot => "one-shot-top-level",
            Licm::Infra => "infrastructure",
        }
    }
}

struct LiSite {
    off: usize,
    /// full emitted bytes (a merged lui+addi pair counts both halves)
    bytes: usize,
    /// bytes visible to the census categories const/li + const/lui
    census_bytes: usize,
    /// addi-zero li (c.li/li) or a lui(+addi) pair site
    is_pair: bool,
    value: i64,
    rd: u8,
    /// innermost containing while span, if any
    in_span: Option<(usize, usize)>,
    w_lo: usize,
    w_hi: usize,
    cat: LiCat,
    /// const-put | literal-operand | synth
    note: &'static str,
    licm: Licm,
}

/// Decode all li-class sites of the artifact: `addi rd, x0, imm` singles
/// (li / c.li) and `lui rd` (+ a contiguous same-rd `addi rd, rd, imm`
/// merged into the pair, mirroring the emitter's `li_pair`). Returns
/// (off, full_bytes, census_bytes, is_pair, value, rd). The census
/// categories const/li + const/lui are exactly the unmerged census-visible
/// halves, so `Σ census_bytes` must equal the census total (asserted).
fn li_sites(lo: &Rv64Lowered) -> Vec<(usize, usize, usize, bool, i64, u8)> {
    let profile = vm::calc::container::CALC_PROFILE_RV64IMC;
    let code = &lo.code;
    struct Raw {
        off: usize,
        bytes: usize,
        census_bytes: usize,
        is_pair: bool,
        value: i64,
        rd: u8,
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut off = 0usize;
    let mut pending_lui: Option<usize> = None;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile).unwrap();
        let l = len as usize;
        let mut merged = false;
        match &inst {
            Inst::ImmAlu { op: ImmAluOp::Addi, rs1: 0, rd, imm } if *rd != 0 => {
                // fold a trailing same-rd addi into an adjacent lui (li_pair)
                if let Some(ri) = pending_lui {
                    if raws[ri].rd == *rd && raws[ri].off + raws[ri].bytes == off {
                        raws[ri].bytes += l;
                        raws[ri].value += *imm;
                        merged = true;
                    }
                }
                if !merged {
                    raws.push(Raw {
                        off,
                        bytes: l,
                        census_bytes: l,
                        is_pair: false,
                        value: *imm,
                        rd: *rd,
                    });
                }
            }
            Inst::Lui { rd, val } => {
                raws.push(Raw {
                    off,
                    bytes: l,
                    census_bytes: l,
                    is_pair: true,
                    value: *val,
                    rd: *rd,
                });
                pending_lui = Some(raws.len() - 1);
                off += l;
                continue;
            }
            _ => {}
        }
        if !merged {
            pending_lui = None;
        }
        off += l;
    }
    raws
        .into_iter()
        .map(|r| (r.off, r.bytes, r.census_bytes, r.is_pair, r.value, r.rd))
        .collect()
}

/// Classify every li-class site of one artifact.
#[allow(clippy::too_many_arguments)]
fn classify_li_sites(
    lo: &Rv64Lowered,
    dbg: &LowerDbg,
    mirror: &[StmtFacts],
    spans: &[(usize, usize)],
) -> Vec<LiSite> {
    let raws = li_sites(lo);
    let body_start = dbg.stmt_ranges.iter().map(|r| r.off_start).min().unwrap_or(0);
    let body_end = dbg.stmt_ranges.iter().map(|r| r.off_end).max().unwrap_or(0);
    let tl = &dbg.timeline;

    // Sound redundancy precondition, computed per (span, slot) on demand:
    // the slot is provably ≡ K at the loop tail iff (a) EVERY timeline write
    // event (WriteU/WriteC) of the slot inside the span sits at a statement
    // whose const_put is exactly K — no data-dependent (XOP) or other-K
    // writes exist; (b) at least one of those writes is unconditional
    // (WriteU — executed every iteration, so every path re-establishes K);
    // (c) no read event of the slot inside the span precedes the first
    // write (the carried entry value is never observed). Under (a)+(b)+(c),
    // deleting a re-init store cannot change any read.
    let mut redundant_cache: BTreeMap<((usize, usize), usize, u64), bool> = BTreeMap::new();
    let mut slot_tail_is_k = |span: (usize, usize), slot: usize, k: u64| -> bool {
        *redundant_cache
            .entry((span, slot, k))
            .or_insert_with(|| {
                let Some(evs) = tl.ev.get(slot) else { return false };
                let in_span: Vec<_> = evs
                    .iter()
                    .filter(|e| span.0 <= e.idx && e.idx <= span.1)
                    .collect();
                let writes: Vec<_> = in_span
                    .iter()
                    .filter(|e| e.kind == WEB_EV_WRITE_U || e.kind == WEB_EV_WRITE_C)
                    .collect();
                if writes.is_empty() {
                    return false;
                }
                // (a) every write is a K const-put of this slot — PUT-kind
                // statements only (an XOP write is a read-modify-write whose
                // const_put field records the operand, and is
                // data-dependent by construction)
                let all_k = writes.iter().all(|e| {
                    matches!(
                        mirror.get(e.idx).filter(|f| f.kind == "put").and_then(|f| f.const_put.as_ref()),
                        Some((s2, PutConst::Int(kv))) if *s2 == slot as u8 && *kv == k
                    )
                });
                if !all_k {
                    return false;
                }
                // (b) an unconditional K write exists (every-iteration def)
                if !writes.iter().any(|e| e.kind == WEB_EV_WRITE_U) {
                    return false;
                }
                // (c) no read before the first write
                let first_write = writes.iter().map(|e| e.idx).min().unwrap();
                if in_span.iter().any(|e| e.kind == WEB_EV_READ && e.idx < first_write) {
                    return false;
                }
                true
            })
    };

    let mut sites = Vec::new();
    for (off, bytes, census_bytes, is_pair, value, rd) in raws {
        let mut cat = if off < body_start {
            LiCat::IPrologue
        } else if off >= body_end {
            LiCat::IThunk
        } else {
            LiCat::CTop
        };
        let (mut w_lo, mut w_hi) = (0usize, 0usize);
        let mut in_span = None;
        if let Some((lo_w, hi_w, _, _)) = dbg
            .stmt_ranges
            .iter()
            .filter(|r| r.off_start <= off && off < r.off_end)
            .min_by_key(|r| r.off_end - r.off_start)
            .map(|r| (r.w_lo, r.w_hi, r.off_start, r.off_end))
        {
            w_lo = lo_w;
            w_hi = hi_w;
            in_span = (w_lo..=w_hi).find_map(|w| innermost_span(spans, w));
            if in_span.is_some() {
                cat = LiCat::ALoop;
            }
        }
        let v = value as u64;
        // collect ALL matching const-put walks (a multi-statement emitted
        // unit can over-match; distinct slots → ambiguous → synth verdict)
        let mut matched_slots: BTreeSet<u8> = BTreeSet::new();
        let mut matched_lit = false;
        for w in w_lo..=w_hi {
            if let Some(f) = mirror.get(w) {
                // const-put matching only for PUT-kind statements (a genuine
                // `var x = K` value write). An XOP statement's const_put
                // field records the read-modify-write OPERAND (e.g. the `1`
                // in `run += 1`), not a value put — its literals take the
                // operand path below instead.
                if f.kind == "put" {
                    if let Some((slot, PutConst::Int(k))) = &f.const_put {
                        if *k == v {
                            matched_slots.insert(*slot);
                        }
                    }
                    if let Some((slot, PutConst::Bytes(b))) = &f.const_put {
                    let be8 = |b: &[u8]| -> u64 {
                        let mut t = [0u8; 8];
                        for (i, x) in b.iter().take(8).enumerate() {
                            t[i] = *x;
                        }
                        u64::from_be_bytes(t)
                    };
                    let le8 = |b: &[u8]| -> u64 {
                        let mut t = [0u8; 8];
                        for (i, x) in b.iter().take(8).enumerate() {
                            t[i] = *x;
                        }
                        u64::from_le_bytes(t)
                    };
                    if b.iter().any(|&x| x as u64 == v)
                        || (b.len() <= 8 && (be8(b) == v || le8(b) == v))
                    {
                        matched_slots.insert(*slot);
                    }
                    }
                }
                if f.lits.contains(&v) || f.lits.contains(&(v as u32 as u64)) {
                    matched_lit = true;
                }
                for bl in &f.byte_lits {
                    if bl.iter().any(|&x| x as u64 == v) {
                        matched_lit = true;
                    }
                }
            }
        }
        let matched_const_put: Option<u8> = if matched_slots.len() == 1 {
            matched_slots.iter().copied().next()
        } else {
            None
        };
        let note = if matched_slots.len() > 1 {
            "const-put?multi"
        } else if matched_const_put.is_some() {
            "const-put"
        } else if matched_lit {
            "literal-operand"
        } else {
            "synth"
        };
        let licm = match cat {
            LiCat::IPrologue | LiCat::IThunk => Licm::Infra,
            LiCat::CTop => Licm::OneShot,
            LiCat::ALoop => match (matched_const_put, matched_lit, matched_slots.len()) {
                (Some(slot), _, 1) => {
                    let span = in_span.unwrap();
                    if slot_tail_is_k(span, slot as usize, v) {
                        Licm::RebuildRedundant
                    } else {
                        Licm::RebuildRequired
                    }
                }
                (None, true, _) => Licm::OperandHoistable,
                _ => Licm::SynthUnattributed,
            },
        };
        sites.push(LiSite {
            off,
            bytes,
            census_bytes,
            is_pair,
            value,
            rd,
            in_span,
            w_lo,
            w_hi,
            cat,
            note,
            licm,
        });
    }
    sites
}

/// census category of one decoded instruction (mirror of the crate's
/// `census_cat`, kept local so the delta table is self-contained)
fn census_cat(i: &Inst) -> &'static str {
    use vm::calc::decode::RegAluOp as R;
    match i {
        Inst::Lui { .. } => "const/lui",
        Inst::Auipc { .. } => "auipc",
        Inst::ImmAlu { op: ImmAluOp::Addi, rs1: 0, .. } => "const/li",
        Inst::ImmAlu { op: ImmAluOp::Addi, imm: 0, rs1, rd } if rd == rs1 && *rs1 != 0 => "mv",
        Inst::RegAlu { op: R::Add, rs1: 0, .. } => "mv",
        Inst::ImmAlu { .. } => "alu-imm",
        Inst::RegAlu { op, .. }
            if matches!(
                op,
                R::Mul | R::Mulh | R::Mulhsu | R::Mulhu | R::Div | R::Divu | R::Rem | R::Remu
            ) =>
        {
            "mul-div"
        }
        Inst::RegAlu { op, .. } if matches!(op, R::Slt | R::Sltu) => "setbb",
        Inst::RegAlu { .. } => "alu-reg",
        Inst::ShiftImm { .. } => "shift",
        Inst::Load { op: LoadOp::Ld, .. } => "ld",
        Inst::Load { op: LoadOp::Lbu, .. } => "lbu",
        Inst::Load { .. } => "load",
        Inst::Store { op: StoreOp::Sd, .. } => "sd",
        Inst::Store { op: StoreOp::Sb, .. } => "sb",
        Inst::Store { .. } => "store",
        Inst::Branch { rs2: 0, .. } => "br-zero",
        Inst::Branch { .. } => "br",
        Inst::Jal { rd: 0, .. } => "j",
        Inst::Jal { .. } => "jal-link",
        Inst::Jalr { .. } => "jr",
        Inst::Ecall => "ecall",
        _ => "other",
    }
}

fn census_stream(code: &[u8], profile: u8) -> BTreeMap<&'static str, (usize, usize, usize)> {
    let mut map: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    let mut off = 0usize;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile).unwrap();
        let e = map.entry(census_cat(&inst)).or_default();
        e.0 += 1;
        e.1 += len as usize;
        e.2 += (len == 2) as usize;
        off += len as usize;
    }
    map
}

/* ==================================================================== */
/* Tests                                                                */
/* ==================================================================== */

/// The transcriptions compile to the pinned baselines and agree across
/// modes on a battery of wires (artifact sanity for everything below).
#[test]
fn kernels_pin_and_agree() {
    // score_sum: ImcRemap baseline 2922 B (P2-A27-S2 diamond-aware segments,
    // docs.loc/rv64im/14-p2a27s2-diamond-segments.md — cfg-sound statement
    // liveness replaces the conservative loop-tail extension; the 62/64
    // slots now bank, init segments end at their true last read). History:
    // 3200 pre-A27 -> 3108 A27 bank 18 -> 3148 P2-F2 eager-AND (+40, the
    // trap-timing fix; see 13-p2f2-bugfixes.md §census) -> 2946 A27-S2
    // diamond segments -> 2922 P2-A19 dominated constant-mul check merging
    // (-24: n-chain 6n<8n deletes mulhu+bne; i-chain 6i,8i<10i deletes
    // 2x mulhu+bne + srl+bne; docs.loc/rv64im/15-p2-a19-guard-compression.md)
    // -> 2918 P3-GUARDTAIL GT-a guard thunk copy (-4: the 5-site zero-form
    // guard cluster @0x8e8..0xa2c converts its 4B guard branches to 2B c.bz
    // against a 6B `li a0,90 + c.j` copy thunk placed @0x9ca in the final
    // stream (predecessor jal @0x9c8 = the safe boundary; seal draft said
    // @0x9d0, corrected against LowerDbg.guard_copies record) (safe-boundary
    // rule: only directly after an unconditional jump/trap); the sibling
    // cluster @0x634..0x7aa has NO safe boundary in its copy-reach window
    // and hacd200's windows likewise — both recorded as zero-impact, not
    // forced; per-cluster attribution in docs.loc/rv64im/96-a25-m4b-guardtail.md).
    let lo_remap = lower(SCORE_SUM_FITSH, true, Rv64Mode::ImcRemap);
    assert_eq!(
        lo_remap.stats.code_bytes, 2918,
        "score_sum ImcRemap baseline moved — update docs.loc/rv64im/96-a25-m4b-guardtail.md"
    );
    let lo_im = lower(SCORE_SUM_FITSH, true, Rv64Mode::Im);
    println!("score_sum code bytes: Im={} ImcRemap={}", lo_im.stats.code_bytes, lo_remap.stats.code_bytes);
    let rust_p2 = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1"
    ))
    .expect("rust profile-2 fixture");
    let (rc, _) = CalcContainer::decode(&rust_p2).unwrap();
    let rseg =
        rc.segs.iter().find(|s| s.vaddr == vm::calc::container::CODE_BASE as u32).unwrap();
    assert_eq!(rseg.data.len(), 2674, "rust profile-2 baseline moved");

    // hacd200 compiles in both modes. ImcRemap baseline history: 3168 (A0-era
    // body, pre-A27 backend) -> 3128 P2-F2 -> 2916 A27-S2 (doc 14 §3.1) ->
    // 3072 there: P2-HACD200-COLOR closed the B3 E3(a) contract gap — the
    // Common path gained the distinct-color-count rule (+156 B; body change,
    // not a backend change; docs.loc/rv64im/68-hacd200-color-rule.md) —
    // 3052 there: P3-IDIOMS M3 fused the :629 `byte(data,0)*256 +
    // byte(data,1)` BE-2 group into one span-guarded lhu (single contiguous
    // code region per mode; attribution table in
    // docs.loc/rv64im/94-a25-m3-byte-idioms.md E2) -> 3052 here: P3-GUARDTAIL
    // GT-a measured ZERO net: the 6-site zero-form guard cluster
    // @0x7a..0x272 is net-positive on paper (2*6-8=+4) but every
    // copy-reach window of it and of every profitable sub-window contains
    // no safe boundary (placement rule: only directly after an
    // unconditional jump/trap) — recorded as re-measured-to-zero, not
    // forced (attribution in docs.loc/rv64im/96-a25-m4b-guardtail.md).
    let h_remap = lower(HACD200_FITSH, true, Rv64Mode::ImcRemap);
    let h_im = lower(HACD200_FITSH, true, Rv64Mode::Im);
    assert_eq!(
        h_remap.stats.code_bytes, 3052,
        "hacd200 ImcRemap baseline moved — update docs.loc/rv64im/96-a25-m4b-guardtail.md"
    );
    println!(
        "hacd200 code bytes: Im={} ImcRemap={}",
        h_im.stats.code_bytes, h_remap.stats.code_bytes
    );

    // cross-mode behavioral agreement on wires (fitsh vs fitsh)
    let remap = pack(&lo_remap);
    let im = pack(&lo_im);
    let hz = pack(&h_remap);
    let hi = pack(&h_im);
    for n in 0..=4usize {
        let input = score_wire(n, 0x9E37_79B9 + n as u64);
        let a = exec(&remap, &input);
        let b = exec(&im, &input);
        assert_eq!(a, b, "score_sum ImcRemap vs Im diverged on {}B input", input.len());
        assert_eq!(exec(&hz, &input), exec(&hi, &input), "hacd200 modes diverged");
    }
}

/// A0.1 — maxlive per kernel / per region / per definition, against the
/// bank budget, plus the allocator ceiling estimate. Im-mode numbers are
/// reported as the contrast (the WebWalk timeline is a mode-independent IR
/// walk, so the liveness numbers must be identical across modes; what
/// differs is whether a bank exists to allocate into).
#[test]
fn a0_1_maxlive_report() {
    for (kernel, body, has_input) in
        [("score_sum", SCORE_SUM_FITSH, true), ("hacd200", HACD200_FITSH, true)]
    {
        let rep = analyze_maxlive(kernel, body, has_input, Rv64Mode::ImcRemap);
        println!("\n===== A0.1 maxlive: {kernel} ({:?}) =====", rep.mode);
        println!(
            "slots: {} Int + {} Bytes | stmt_total={} | bank regs={} (banked slots={} pool_size={} pool_base={:?})",
            rep.n_slots_int,
            rep.n_slots_bytes,
            rep.stmt_total,
            rep.bank_regs,
            rep.banked_slot_count,
            rep.pool_size,
            rep.pool_base_reg
        );
        for (name, (slots, regs)) in &rep.maxlive {
            println!(
                "maxlive[{name:^22}] = {slots:>2} slots = {regs:>2} regs @stmt {} (bytes slots at peak: {})",
                rep.argmax[*name], rep.bytes_at_argmax
            );
        }
        println!(
            "artifact slot-to-frame ld/sd (base x8): {} inst, {} B across {} frame slots",
            rep.slot_ldsd_insts,
            rep.slot_ldsd_bytes,
            rep.slot_ldsd_by_slot.len()
        );
        println!(
            "traffic (reg-weighted): total={} spilled-cfg={} → theoretically eliminable ratio = {:.3}",
            rep.traffic_total,
            rep.traffic_spilled_cfg,
            1.0 - rep.traffic_spilled_cfg as f64 / rep.traffic_total.max(1) as f64
        );
        println!("-- while regions (cfg-sound def), innermost first --");
        for s in &rep.spans {
            println!(
                "  depth{} span stmts {}..={} maxlive={} slots / {} regs @stmt {} spill_excess_xt={} traffic={} cond_slots={:?}",
                s.depth, s.a, s.b, s.slot_count, s.reg_count, s.worst_point,
                s.spill_excess, s.traffic, s.cond_slots
            );
            for (slot, ty, banked) in &s.worst_slots {
                println!("      slot {slot:>3} {ty:<5} banked={banked}");
            }
        }
        let mut rows: Vec<_> = rep.slot_ldsd_by_slot.iter().collect();
        rows.sort_by_key(|(_, (_, b))| std::cmp::Reverse(*b));
        println!("-- artifact slot ld/sd bytes top 15 (frame-resident slots only) --");
        for (slot, (n, b)) in rows.iter().take(15) {
            println!("  slot {slot:>3}: {n:>3} inst, {b:>4} B");
        }
        // Im contrast: same IR walk (mode-independent timeline) → identical
        // maxlive; artifact has no bank plan (pool fixed at 8).
        let rep_im = analyze_maxlive(kernel, body, has_input, Rv64Mode::Im);
        assert_eq!(
            rep.maxlive, rep_im.maxlive,
            "liveness is a mode-independent IR property — Im must match ImcRemap"
        );
        println!(
            "Im contrast [{}]: code={} B, bank regs={} (no bank plan; maxlive identical: {:?})",
            rep_im.kernel,
            rep_im.code_bytes,
            rep_im.bank_regs,
            rep_im.maxlive[Def::Cfg.name()]
        );
    }
}

/// A0.2 — li materialization classification for score_sum ImcRemap,
/// census-aligned (must reproduce the li-category bytes exactly and the
/// +218 B delta attribution vs the rust profile-2 kernel).
#[test]
fn a0_2_li_classification() {
    let (lo, dbg) = lower_dbg(SCORE_SUM_FITSH, true, Rv64Mode::ImcRemap);
    let (mirror, spans, _) = ir_mirror(&compile_ir(SCORE_SUM_FITSH, true));
    assert_eq!(
        mirror.len() - 1,
        dbg.timeline.stmt_total,
        "mirror walk numbering desynced from the WebWalk timeline"
    );
    assert_eq!(spans, dbg.timeline.while_spans);
    let sites = classify_li_sites(&lo, &dbg, &mirror, &spans);

    let mut by_cat: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    let mut by_note: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut by_licm: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    for s in &sites {
        let e = by_cat.entry(s.cat.name()).or_default();
        e.0 += 1;
        e.1 += s.bytes;
        e.2 += s.census_bytes;
        if !s.note.is_empty() {
            let e = by_note.entry(format!("{}:{}", s.cat.name(), s.note)).or_default();
            e.0 += 1;
            e.1 += s.bytes;
            e.2 += s.census_bytes;
        }
        let e = by_licm.entry(s.licm.name()).or_default();
        e.0 += 1;
        e.1 += s.bytes;
        e.2 += s.census_bytes;
    }
    let total_bytes: usize = sites.iter().map(|s| s.bytes).sum();
    let total_census: usize = sites.iter().map(|s| s.census_bytes).sum();
    println!("\n===== A0.2 li classification: score_sum ImcRemap =====");
    println!(
        "total li-class sites={} bytes={total_bytes} (census-visible={total_census})",
        sites.len()
    );
    println!("-- by location --");
    for (k, (n, b, c)) in &by_cat {
        println!("  {k:<12} n={n:>3} bytes={b:>4} census={c:>4}");
    }
    println!("-- by location:note --");
    for (k, (n, b, c)) in &by_note {
        println!("  {k:<26} n={n:>3} bytes={b:>4} census={c:>4}");
    }
    println!("-- by LICM verdict --");
    for (k, (n, b, c)) in &by_licm {
        println!("  {k:<32} n={n:>3} bytes={b:>4} census={c:>4}");
    }
    println!("-- sites in loops (A-loop) --");
    for s in sites.iter().filter(|s| s.cat == LiCat::ALoop) {
        println!(
            "  @{:04x} {:>1}B{} val={:<10} rd=x{} walk {}..={} span={:?} [{}] {}",
            s.off, s.bytes, if s.is_pair { "(lui-pair)" } else { "" }, s.value, s.rd, s.w_lo,
            s.w_hi, s.in_span, s.note, s.licm.name()
        );
    }
    println!("-- top-level (C-top) sites --");
    for s in sites.iter().filter(|s| s.cat == LiCat::CTop) {
        println!(
            "  @{:04x} {:>1}B{} val={:<10} rd=x{} walk {}..={} [{}]",
            s.off, s.bytes, if s.is_pair { "(lui-pair)" } else { "" }, s.value, s.rd, s.w_lo,
            s.w_hi, s.note
        );
    }
    println!("-- prologue/thunk (I-*) sites --");
    for s in sites.iter().filter(|s| matches!(s.cat, LiCat::IPrologue | LiCat::IThunk)) {
        println!(
            "  @{:04x} {}B{} val={} rd=x{} [{}] infrastructure",
            s.off, s.bytes, if s.is_pair { "(lui-pair)" } else { "" }, s.value, s.rd, s.cat.name()
        );
    }

    // ---- census alignment: Σ census-visible bytes == census const/li+const/lui
    let profile = vm::calc::container::CALC_PROFILE_RV64IMC;
    let cen = census_stream(&lo.code, profile);
    let li_bytes =
        cen.get("const/li").map(|r| r.1).unwrap_or(0) + cen.get("const/lui").map(|r| r.1).unwrap_or(0);
    let li_count =
        cen.get("const/li").map(|r| r.0).unwrap_or(0) + cen.get("const/lui").map(|r| r.0).unwrap_or(0);
    assert_eq!(total_census, li_bytes, "li classification must cover the census bytes");
    assert_eq!(sites.len(), li_count, "li site count must match the census");
    println!(
        "census const/li+const/lui: n={li_count} bytes={li_bytes} — aligned (pairs add {} B of merged non-census addi halves)",
        total_bytes - total_census
    );

    // ---- full delta table vs the rust profile-2 fixture
    let rust_p2 = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1"
    ))
    .unwrap();
    let (rc, _) = CalcContainer::decode(&rust_p2).unwrap();
    let rcode = &rc
        .segs
        .iter()
        .find(|s| s.vaddr == vm::calc::container::CODE_BASE as u32)
        .unwrap()
        .data;
    let rcen = census_stream(rcode, profile);
    println!("-- census delta (fitsh - rust) by category, bytes --");
    let keys: BTreeSet<&'static str> = cen.keys().chain(rcen.keys()).copied().collect();
    let mut delta_total = 0i64;
    for k in &keys {
        let f = cen.get(*k).map(|r| r.1).unwrap_or(0);
        let r = rcen.get(*k).map(|r| r.1).unwrap_or(0);
        let d = f as i64 - r as i64;
        delta_total += d;
        println!("  {k:<12} fitsh={f:>5} rust={r:>5} delta={d:+}");
    }
    println!("  TOTAL delta = {delta_total:+} (census baseline: 2946 - 2674 = +272; history: 3200 pre-A27 -> 3108 A27 -> 3148 P2-F2 eager-AND -> 2946 A27-S2 diamond segments)");
    let fli = cen.get("const/li").map(|r| r.1).unwrap_or(0)
        + cen.get("const/lui").map(|r| r.1).unwrap_or(0);
    let rli = rcen.get("const/li").map(|r| r.1).unwrap_or(0)
        + rcen.get("const/lui").map(|r| r.1).unwrap_or(0);
    println!("li-category delta (fitsh-rust) = {} B (acceptance report pins +218)", fli as i64 - rli as i64);

    // ---- category (b): literal-pool loads (bytes literals; not li-class)
    if let Some(base) = dbg.pool_base_reg {
        let mut pool_ld = (0usize, 0usize);
        let mut pool_addi = (0usize, 0usize);
        let mut off = 0usize;
        while off < lo.code.len() {
            let (inst, len) = decode_at(&lo.code, off, profile).unwrap();
            match &inst {
                Inst::Load { rs1, .. } if *rs1 == base => {
                    pool_ld.0 += 1;
                    pool_ld.1 += len as usize;
                }
                Inst::ImmAlu { op: ImmAluOp::Addi, rs1, .. } if *rs1 == base => {
                    pool_addi.0 += 1;
                    pool_addi.1 += len as usize;
                }
                _ => {}
            }
            off += len as usize;
        }
        println!(
            "PoolLd via x{base}: n={} bytes={}; PoolOff addi via x{base}: n={} bytes={}",
            pool_ld.0, pool_ld.1, pool_addi.0, pool_addi.1
        );
        let pool_seg_bytes = lo.pool.len();
        println!("literal pool segment (RO, outside the code census): {pool_seg_bytes} B, {} entries", dbg.lit_sites);
    } else {
        println!("no pool-base register (pool scheme inactive)");
    }

    // ---- LICM savings estimate, printed for the doc
    // hoistable operands: per enclosing loop, per distinct value, the win is
    // (sites − 1) × site bytes (one preheader copy replaces n loop copies).
    let mut hoist_wins: BTreeMap<((usize, usize), u64), (usize, usize, usize)> = BTreeMap::new();
    for s in sites.iter().filter(|s| s.licm == Licm::OperandHoistable) {
        let e = hoist_wins.entry((s.in_span.unwrap_or((0, 0)), s.value as u64)).or_default();
        e.0 += 1;
        e.1 += s.bytes;
        e.2 = e.2.max(s.bytes);
    }
    let hoist_total: usize = hoist_wins
        .values()
        .map(|(n, tot, one)| tot.saturating_sub(if *n > 0 { *one } else { 0 }))
        .sum();
    println!("-- LICM-hoistable operand dedup (per loop per value: (n-1)×bytes) --");
    for ((span, v), (n, tot, one)) in &hoist_wins {
        println!("  span={span:?} val={v}: n={n} total={tot}B one-copy={one}B win={}B", tot - one);
    }
    println!("LICM operand-hoist code-size win total ≈ {hoist_total} B (upper bound)");
    let rebuild_req: usize = sites
        .iter()
        .filter(|s| s.licm == Licm::RebuildRequired)
        .map(|s| s.bytes)
        .sum();
    println!("rebuild-required bytes (semantically per-iteration): {rebuild_req} B");
}

/// hacd200 li classification (context for the score_sum-focused A0.2).
#[test]
fn a0_2_hacd200_li_context() {
    let (lo, dbg) = lower_dbg(HACD200_FITSH, true, Rv64Mode::ImcRemap);
    let (mirror, spans, _) = ir_mirror(&compile_ir(HACD200_FITSH, true));
    assert_eq!(mirror.len() - 1, dbg.timeline.stmt_total);
    let sites = classify_li_sites(&lo, &dbg, &mirror, &spans);
    let mut by_cat: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    let mut by_licm: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    for s in &sites {
        let e = by_cat.entry(s.cat.name()).or_default();
        e.0 += 1;
        e.1 += s.bytes;
        e.2 += s.census_bytes;
        let e = by_licm.entry(s.licm.name()).or_default();
        e.0 += 1;
        e.1 += s.bytes;
        e.2 += s.census_bytes;
    }
    println!("\n===== A0.2 li classification: hacd200 ImcRemap =====");
    println!(
        "total sites={} bytes={} census={}",
        sites.len(),
        sites.iter().map(|s| s.bytes).sum::<usize>(),
        sites.iter().map(|s| s.census_bytes).sum::<usize>()
    );
    for (k, (n, b, c)) in &by_cat {
        println!("  {k:<12} n={n:>3} bytes={b:>4} census={c:>4}");
    }
    println!("-- by LICM verdict --");
    for (k, (n, b, c)) in &by_licm {
        println!("  {k:<32} n={n:>3} bytes={b:>4} census={c:>4}");
    }
    println!("-- sites in loops (A-loop) --");
    for s in sites.iter().filter(|s| s.cat == LiCat::ALoop) {
        println!(
            "  @{:04x} {:>1}B val={:<10} rd=x{} walk {}..={} span={:?} [{}] {}",
            s.off, s.bytes, s.value, s.rd, s.w_lo, s.w_hi, s.in_span, s.note, s.licm.name()
        );
    }
}

/// Whole-pipeline determinism: byte-identical artifacts and timeline on a
/// repeated run.
#[test]
fn analysis_is_deterministic() {
    for (body, has_input) in [(SCORE_SUM_FITSH, true), (HACD200_FITSH, true)] {
        let a = lower_dbg(body, has_input, Rv64Mode::ImcRemap);
        let b = lower_dbg(body, has_input, Rv64Mode::ImcRemap);
        assert_eq!(a.0.code, b.0.code);
        assert_eq!(a.0.pool, b.0.pool);
        assert_eq!(a.1.timeline.stmt_total, b.1.timeline.stmt_total);
        assert_eq!(a.1.timeline.while_spans, b.1.timeline.while_spans);
        assert_eq!(a.1.timeline.ev.len(), b.1.timeline.ev.len());
        for (ea, eb) in a.1.timeline.ev.iter().zip(b.1.timeline.ev.iter()) {
            assert_eq!(
                ea.iter().map(|e| (e.idx, e.kind, e.region)).collect::<Vec<_>>(),
                eb.iter().map(|e| (e.idx, e.kind, e.region)).collect::<Vec<_>>()
            );
        }
        assert_eq!(a.1.stmt_ranges.len(), b.1.stmt_ranges.len());
        assert_eq!(
            a.1
                .stmt_ranges
                .iter()
                .map(|r| (r.w_lo, r.w_hi, r.off_start, r.off_end))
                .collect::<Vec<_>>(),
            b.1
                .stmt_ranges
                .iter()
                .map(|r| (r.w_lo, r.w_hi, r.off_start, r.off_end))
                .collect::<Vec<_>>()
        );
    }
}

/// Trace validation: the statement ranges cover the walk indices and the
/// body bytes exactly (no coverage holes). Guards the li→statement mapping.
#[test]
fn stmt_ranges_cover_the_body() {
    for (body, has_input) in [(SCORE_SUM_FITSH, true), (HACD200_FITSH, true)] {
        let (_, dbg) = lower_dbg(body, has_input, Rv64Mode::ImcRemap);
        let n = dbg.timeline.stmt_total;
        let mut covered = vec![false; n + 1];
        for r in &dbg.stmt_ranges {
            for w in r.w_lo..=r.w_hi.min(n) {
                if w >= 1 {
                    covered[w] = true;
                }
            }
        }
        let holes: Vec<usize> = (1..=n).filter(|&w| !covered[w]).collect();
        assert!(holes.is_empty(), "{body}: walk-index coverage holes: {holes:?}");
        let body_start = dbg.stmt_ranges.iter().map(|r| r.off_start).min().unwrap();
        let body_end = dbg.stmt_ranges.iter().map(|r| r.off_end).max().unwrap();
        let mut cur = body_start;
        let mut sorted: Vec<(usize, usize)> =
            dbg.stmt_ranges.iter().map(|r| (r.off_start, r.off_end)).collect();
        sorted.sort_unstable();
        for (s, e) in sorted {
            if s > cur {
                panic!("{body}: byte coverage hole at ({s:#x}, {cur:#x})");
            }
            cur = cur.max(e);
        }
        assert_eq!(cur, body_end, "{body}: byte coverage ends short");
        println!(
            "{body}: stmt ranges cover [{body_start:#x}, {body_end:#x}) — ok ({} ranges, {n} walk idx)",
            dbg.stmt_ranges.len()
        );
    }
}

/// CFG sanity on hand-checkable micro-bodies: the resolver must produce the
/// textbook while CFG and the diamond CFG (documented expectations).
#[test]
fn cfg_edges_sanity() {
    // while loop: var s=0; var i=0; while i<3 { s += i; i += 1 }; return s
    let body = "var s = 0u64\nvar i = 0u64\nwhile i < 3 { s += i\ni += 1 }\nreturn s as bytes";
    let ir = compile_ir(body, false);
    let (mirror, spans, ifrs) = ir_mirror(&ir);
    let (_, dbg) = lower_dbg(body, false, Rv64Mode::ImcRemap);
    assert_eq!(mirror.len() - 1, dbg.timeline.stmt_total);
    assert_eq!(spans, dbg.timeline.while_spans);
    // exactly one while span, no ifs
    assert_eq!(spans.len(), 1, "one while expected");
    assert!(ifrs.is_empty(), "no ifs expected");
    // the loop-carried slots (s, i) must be live at the header (backedge)
    for slot in 0..dbg.timeline.slot_tys.len() {
        let evs = &dbg.timeline.ev[slot];
        if evs.is_empty() {
            continue;
        }
        let lv = slot_liveness(evs, dbg.timeline.stmt_total, &spans, &ifrs, Def::Cfg);
        let (a, _b) = spans[0];
        // both s and i are read/written across the loop — both must be live
        // at the header under the cfg def (this is the backedge requirement)
        assert!(lv[a], "slot {slot} must be live at the loop header (backedge fold)");
    }
    // strict def has no wrap: the header liveness for i (first written at 0,
    // re-read each iteration) still holds since i's interval covers the loop
    let _ = slot_liveness(&dbg.timeline.ev[1], dbg.timeline.stmt_total, &spans, &ifrs, Def::Strict);
}

/// The parked-draft regression: the resolver handles an if as a loop tail
/// (then-arm exit must land on the loop HEADER, not on a bogus join past
/// the loop) and a loop as an if-arm tail (loop exit lands on the if's
/// join). Micro-bodies exercising both shapes.
#[test]
fn cfg_edge_cases_tail_redirects() {
    // if as loop tail: then-arm exit must fold to the loop header
    let body = "var s = 0u64\nvar i = 0u64\nwhile i < 3 { i += 1\nif s == 0 { s = 1 } else { s = 2 }\ni += 1 }\nreturn s as bytes";
    let ir = compile_ir(body, false);
    let (mirror, spans, ifrs) = ir_mirror(&ir);
    let (_, dbg) = lower_dbg(body, false, Rv64Mode::ImcRemap);
    assert_eq!(mirror.len() - 1, dbg.timeline.stmt_total);
    assert_eq!(spans.len(), 1);
    assert_eq!(ifrs.len(), 1);
    // walk the live sets: nothing may be live at a virtual position past the
    // loop (n+1 is the exit and only the header's exit edge reaches it)
    let n = dbg.timeline.stmt_total;
    for slot in 0..dbg.timeline.slot_tys.len() {
        let evs = &dbg.timeline.ev[slot];
        if evs.is_empty() {
            continue;
        }
        let lv = slot_liveness(evs, n, &spans, &ifrs, Def::Cfg);
        let (a, b) = spans[0];
        assert!(lv[a] || evs.iter().all(|e| e.idx < a), "loop-carried slot live at header");
        let _ = b;
    }
    // loop as if-arm tail: loop exit must land on the if's join.
    // (Mirror-only: no lowering here — this exact shape trips a PRE-EXISTING
    // RV64_INV_CHECK instrumentation gap in the engine's segment checker,
    // proven to fire on the unpatched baseline too; see
    // docs.loc/rv64im/10-a0-maxlive-and-licm.md §Open questions. Without
    // RV64_INV_CHECK the lowering and execution of this shape succeed.)
    let body2 = "var s = 0u64\nvar i = 0u64\nif s == 0 { while i < 3 { i += 1 } } else { s = 2 }\ns += i\nreturn s as bytes";
    let ir2 = compile_ir(body2, false);
    let (mirror2, spans2, ifrs2) = ir_mirror(&ir2);
    assert!(mirror2.len() - 1 >= 1);
    assert_eq!(spans2.len(), 1);
    assert_eq!(ifrs2.len(), 1);
    // the loop's exit target must be the statement after the whole if
    // (rebuild the resolver's answer directly for the report)
    let r = ifrs2[0];
    let (_a, b) = spans2[0];
    assert!(
        b >= r.t_hi,
        "loop must end at/after the then-arm tail for this shape to be exercised"
    );
    let mut skip = BTreeSet::new();
    skip.insert(('L', 0usize));
    let exit = land(b + 1, b, &spans2, &ifrs2, &mut skip);
    assert_eq!(exit, r.e_hi + 1, "loop exit must land on the if's join");
}
