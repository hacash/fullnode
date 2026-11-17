use super::*;


/* ---- Pass B: web/segment analysis + interval-shared bank allocation ---- */

/// Per-slot banking kind: the register (pair) IS the slot's home for the
/// whole function; which register depends on the active web segment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BankKind {
    None,
    Int,
    Bytes,
}

/// Static plan: which slots are banked and which register (pair) each web
/// segment of each banked slot lives in, keyed by statement index.
#[derive(Clone)]
pub(crate) struct WebPlan {
    pub(crate) banked: Vec<BankKind>,
    /// per slot: sorted (segment-start stmt idx, regs); Int uses [0].
    pub(crate) homes: Vec<Vec<(usize, [u8; 2])>>,
    /// banked slots whose first segment is an init segment (first touch
    /// not an unconditional write): they need the entry zero
    pub(crate) init_zero: Vec<bool>,
    /// debug copies for the invariant checker (test-only)
    pub(crate) segs_debug: Vec<Vec<Seg>>,
    pub(crate) no_split_debug: Vec<bool>,
}

impl WebPlan {
    /// Register home of `slot` at statement `idx` (Int: [0]; Bytes: both).
    /// Segment boundaries are write statements: a WRITE at the boundary
    /// takes the new segment (inclusive), a READ at it evaluates before
    /// that write and takes the previous one (exclusive).
    pub(crate) fn home_at(&self, slot: usize, idx: usize, for_write: bool) -> Option<[u8; 2]> {
        if !matches!(self.banked.get(slot), Some(BankKind::Int) | Some(BankKind::Bytes)) {
            return None;
        }
        let hs = self.homes.get(slot)?;
        let mut cur = hs.first()?.1;
        let mut chosen_b = hs.first()?.0;
        for (b, regs) in hs {
            let hit = if for_write { *b <= idx } else { *b < idx };
            if hit {
                cur = *regs;
                chosen_b = *b;
            } else {
                break;
            }
        }
        if std::env::var("RV64_INV_CHECK").is_ok() {
            let segs = &self.segs_debug[slot];
            if let Some(sg) = segs.iter().find(|s| s.start == chosen_b) {
                let covers = if for_write {
                    idx >= sg.start
                } else {
                    idx >= sg.start && idx <= sg.end
                };
                if !covers {
                    panic!(
                        "INV slot={slot} idx={idx} for_write={for_write} chosen_b={chosen_b} seg=[{},{}] no-split={}",
                        sg.start, sg.end, self.no_split_debug[slot]
                    );
                }
            }
        }
        Some(cur)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum EvKind {
    Read,
    /// unconditional write: dominates later reads on its region path
    WriteU,
    /// write under if/choose arms: merges into the current segment
    WriteC,
}

#[derive(Clone, Copy)]
pub(crate) struct Ev {
    pub(crate) idx: usize,
    pub(crate) kind: EvKind,
    pub(crate) region: usize,
}

/// One live segment of one slot: `[start, end]` in statement indices; the
/// value is born at `start` (a write, or the zero from entry for init
/// segments) and dead after `end`.
#[derive(Clone)]
pub(crate) struct Seg {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// value comes from the entry zero: cannot share (a sharer's writes
    /// between the prologue and the segment's reads would clobber it)
    pub(crate) init: bool,
}

/// Statement-order walk mirroring `Lower`'s traversal exactly (the same
/// nodes increment the same counter), recording per-slot access events.
pub(crate) struct WebWalk<'a> {
    pub(crate) slot: &'a [SlotTy],
    /// Timeline mode: record XOP's read-modify-write as an explicit Read
    /// event ahead of its Write. The plan walk keeps the legacy pure-write
    /// events; the dead-store walk must see the old-value read or a store
    /// feeding only an `+=` looks dead and gets dropped.
    pub(crate) xop_reads: bool,
    pub(crate) idx: usize,
    /// writes under if/choose arms are conditional; while bodies are NOT
    /// (their top-of-body writes dominate the body's own reads; the split
    /// rule's region containment handles loop-skip paths)
    pub(crate) if_depth: usize,
    pub(crate) region: usize,
    pub(crate) next_region: usize,
    pub(crate) region_parent: Vec<usize>,
    pub(crate) ev: Vec<Vec<Ev>>,
    /// slots read by some while condition: loop-carried values — never
    /// split (the backedge re-reads the last body write invisibly)
    pub(crate) cond_read: Vec<bool>,
    /// (while idx, post-body idx) spans, filled on body completion
    pub(crate) while_spans: Vec<(usize, usize)>,
    /// if-diamond ranges (hdr, t_hi, e_hi): hdr = the IRIF's own idx,
    /// t_hi/e_hi = last idx of the then/else arm (P2-A27-S2: statement-CFG
    /// liveness for exact segment ends — see `StmtCfg`)
    pub(crate) ifrs: Vec<(usize, usize, usize)>,
    /// enclosing while spans (indices into `while_spans`), for break/continue
    pub(crate) loop_stack: Vec<usize>,
    /// break/continue sites: (stmt idx, enclosing `while_spans` index,
    /// is_break) — their CFG edge leaves the linear chain
    pub(crate) brk_cont: Vec<(usize, usize, bool)>,
}

impl<'a> WebWalk<'a> {
    pub(crate) fn new(slot: &'a [SlotTy]) -> Self {
        Self {
            slot,
            xop_reads: false,
            idx: 0,
            if_depth: 0,
            region: 0,
            next_region: 1,
            region_parent: vec![usize::MAX],
            ev: vec![Vec::new(); slot.len()],
            cond_read: vec![false; slot.len()],
            while_spans: Vec::new(),
            ifrs: Vec::new(),
            loop_stack: Vec::new(),
            brk_cont: Vec::new(),
        }
    }

    pub(crate) fn region_within(&self, r: usize, anc: usize) -> bool {
        let mut cur = r;
        while cur != usize::MAX {
            if cur == anc {
                return true;
            }
            cur = self.region_parent[cur];
        }
        false
    }

    pub(crate) fn record_expr(&mut self, node: &dyn IRNode) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.record_expr(&*w.node);
        }
        let op = match Bytecode::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        match op {
            Bytecode::GET | Bytecode::GET0 | Bytecode::GET1 | Bytecode::GET2 | Bytecode::GET3 => {
                if let Some(s) = slot_get_index(node) {
                    let s = s as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                    }
                }
            }
            // T11: a constant-index local_x(i) IS a slot read — without the
            // event a store feeding only a folded GETX looks dead and the
            // DSE pass drops it (and its init-flag write)
            Bytecode::GETX => {
                for c in children(node) {
                    self.record_expr(c);
                }
                if let Some(s) = slot_get_index(node) {
                    let s = s as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                    }
                }
            }
            // T9: XLG reads its lhs slot (the mark's low 5 bits) on top of
            // the children walk (the rhs subtree)
            Bytecode::XLG => {
                for c in children(node) {
                    self.record_expr(c);
                }
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    let s = decode_local_logic_mark(p.para).1 as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                    }
                }
            }
            Bytecode::PUT | Bytecode::XOP => {
                // nested inside an expression: always conditional
                for c in children(node) {
                    self.record_expr(c);
                }
                let s = match op {
                    Bytecode::PUT => node
                        .as_any()
                        .downcast_ref::<IRNodeParam1Single>()
                        .map(|p| p.para as usize),
                    _ => node
                        .as_any()
                        .downcast_ref::<IRNodeParam1Single>()
                        .map(|p| decode_local_operand_mark(p.para).1 as usize),
                };
                if let Some(s) = s {
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::WriteC, region: self.region });
                    }
                }
            }
            _ => {
                for c in children(node) {
                    self.record_expr(c);
                }
            }
        }
    }

    pub(crate) fn walk_stmt(&mut self, node: &dyn IRNode) {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.walk_stmt(&*w.node);
        }
        if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
            || node.as_any().downcast_ref::<IRNodeText>().is_some()
        {
            self.idx += 1;
            return;
        }
        let op = match Bytecode::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        self.idx += 1;
        match op {
            // T8: the high-slot local compound fragment — ONE statement
            // position (the lowering consumes the whole IRLIST natively,
            // XOP-shaped): the value's reads record here via record_expr
            // and the slot's read+write events mirror the XOP arm; the
            // subs are NOT walked (the raw fragment records nothing).
            Bytecode::IRLIST
                if node
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .map(|a| local_compound_assign_parts(&*a).is_some())
                    .unwrap_or(false) =>
            {
                let (value, idx, _) = local_compound_assign_parts(node).unwrap();
                self.record_expr(value);
                let s = idx as usize;
                if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                    if self.xop_reads {
                        // the old value is read before the write (same
                        // statement index: boundary reads evaluate first)
                        self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                    }
                    let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                    self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                }
            }
            Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
                if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                    for sub in &arr.subs {
                        self.walk_stmt(&**sub);
                    }
                }
            }
            Bytecode::IRIF => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    self.record_expr(&*n.subx);
                    let hdr = self.idx;
                    self.if_depth += 1;
                    self.walk_stmt(&*n.suby);
                    let t_hi = self.idx - 1;
                    self.walk_stmt(&*n.subz);
                    let e_hi = self.idx - 1;
                    self.if_depth -= 1;
                    self.ifrs.push((hdr, t_hi, e_hi));
                }
            }
            Bytecode::IRIFR | Bytecode::CHOOSE => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeTriple>() {
                    self.record_expr(&*n.subx);
                    self.if_depth += 1;
                    self.record_expr(&*n.suby);
                    self.record_expr(&*n.subz);
                    self.if_depth -= 1;
                }
            }
            Bytecode::IRWHILE => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    // loop-carried slots (read in the condition) keep ONE
                    // segment across the loop: the backedge re-reads the
                    // last body write in a way the linear event list cannot
                    // express
                    mark_cond_reads(&*n.subx, &mut self.cond_read);
                    self.record_expr(&*n.subx);
                    let r = self.next_region;
                    self.next_region += 1;
                    self.region_parent.push(self.region);
                    let saved = self.region;
                    self.region = r;
                    let span = self.while_spans.len();
                    self.while_spans.push((self.idx, self.idx));
                    self.loop_stack.push(span);
                    self.walk_stmt(&*n.suby);
                    self.loop_stack.pop();
                    self.while_spans[span].1 = self.idx;
                    self.region = saved;
                }
            }
            Bytecode::PUT => {
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    self.record_expr(&*p.subx);
                    let s = p.para as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                        self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                    }
                }
            }
            Bytecode::XOP => {
                if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
                    self.record_expr(&*p.subx);
                    let s = decode_local_operand_mark(p.para).1 as usize;
                    if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                        if self.xop_reads {
                            // the old value is read before the write (same
                            // statement index: boundary reads evaluate first)
                            self.ev[s].push(Ev { idx: self.idx, kind: EvKind::Read, region: self.region });
                        }
                        let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                        self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                    }
                }
            }
            // T11: constant-index local_x_put(i, v) — the PUT store face
            Bytecode::PUTX => {
                if let Some(p) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    self.record_expr(&*p.suby);
                    if let Ok(s) = local_x_slot_index(&*p.subx, "PUTX") {
                        let s = s as usize;
                        if matches!(self.slot.get(s), Some(SlotTy::Int) | Some(SlotTy::Bytes) | Some(SlotTy::Address)) {
                            let kind = if self.if_depth == 0 { EvKind::WriteU } else { EvKind::WriteC };
                            self.ev[s].push(Ev { idx: self.idx, kind, region: self.region });
                        }
                    }
                }
            }
            Bytecode::REQ => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeDouble>() {
                    self.record_expr(&*n.subx);
                    self.record_expr(&*n.suby);
                }
            }
            Bytecode::AST | Bytecode::ERR => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
                    self.record_expr(&*n.subx);
                }
            }
            Bytecode::RET => {
                if let Some(n) = node.as_any().downcast_ref::<IRNodeSingle>() {
                    self.record_expr(&*n.subx);
                }
            }
            _ => {
                // break/continue leave the linear chain (jump to the loop's
                // exit/header): record the edge target's span for the CFG
                if matches!(op, Bytecode::IRBREAK | Bytecode::IRCONTINUE) {
                    if let Some(&sp) = self.loop_stack.last() {
                        self.brk_cont.push((self.idx, sp, op == Bytecode::IRBREAK));
                    }
                }
                // expression-fallback statement: eval + drop
                self.record_expr(node);
            }
        }
    }
}

/// Statement-index count of `node` under `WebWalk::walk_stmt` — the Lower
/// cursor must advance by exactly this per statement, or `home_at` lookups
/// desync from the plan (a real bug class: const-select arms count their
/// inner block+PUT+else while the group consumed them as one sub).
pub(crate) fn stmt_walk_count(node: &dyn IRNode) -> usize {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return stmt_walk_count(&*w.node);
    }
    if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
        || node.as_any().downcast_ref::<IRNodeText>().is_some()
    {
        return 1;
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return 1,
    };
    1 + match op {
        // T8: the high-slot local compound fragment counts as ONE statement
        // (WebWalk::walk_stmt consumes it without walking the subs)
        Bytecode::IRLIST
            if node
                .as_any()
                .downcast_ref::<IRNodeArray>()
                .map(|a| local_compound_assign_parts(&*a).is_some())
                .unwrap_or(false) =>
        {
            0
        }
        Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
            node.as_any()
                .downcast_ref::<IRNodeArray>()
                .map(|arr| arr.subs.iter().map(|s| stmt_walk_count(&**s)).sum())
                .unwrap_or(0)
        }
        Bytecode::IRIF => match node.as_any().downcast_ref::<IRNodeTriple>() {
            Some(n) => stmt_walk_count(&*n.suby) + stmt_walk_count(&*n.subz),
            None => 0,
        },
        Bytecode::IRWHILE => match node.as_any().downcast_ref::<IRNodeDouble>() {
            Some(n) => stmt_walk_count(&*n.suby),
            None => 0,
        },
        _ => 0,
    }
}

/// Flag every slot read inside an expression tree as loop-carried.
pub(crate) fn mark_cond_reads(node: &dyn IRNode, cond_read: &mut Vec<bool>) {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return mark_cond_reads(&*w.node, cond_read);
    }
    if let Some(s) = slot_get_index(node) {
        let s = s as usize;
        if cond_read.len() > s {
            cond_read[s] = true;
        }
        return;
    }
    // T9: XLG's lhs slot read is not a child node
    if let Ok(Bytecode::XLG) = Bytecode::try_from_u8(node.bytecode()) {
        if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
            let s = decode_local_logic_mark(p.para).1 as usize;
            if cond_read.len() > s {
                cond_read[s] = true;
            }
        }
    }
    for c in children(node) {
        mark_cond_reads(c, cond_read);
    }
}

/* ---- P2-A27-S2: statement-level CFG for exact (cfg-sound) liveness ---- */

/// The statement CFG the segment ends are measured on: linear chain + if
/// diamond rewiring (the untaken arm is skipped by the header's false edge,
/// arm tails land at the join) + while backedges/loop exits + break /
/// continue edges. Production port of the census measurement CFG
/// (`rv64_backend_maxlive_census.rs`, `slot_liveness(Def::Cfg)` — doc 10),
/// which doc 12 §6.1 identified as the true lifetime of the init segments
/// the old `seg_end` backedge rule over-extended to the loop end.
pub(crate) struct StmtCfg {
    /// succ[p] = real successors of statement p (p in 1..=n; n+1 = exit)
    pub(crate) succ: Vec<Vec<usize>>,
}

impl StmtCfg {
    /// `spans` = `WebWalk::while_spans` (header idx, last body idx);
    /// `ifrs` = (hdr, t_hi, e_hi); `brk_cont` = (stmt, span idx, is_break).
    pub(crate) fn new(n: usize, spans: &[(usize, usize)], ifrs: &[(usize, usize, usize)], brk_cont: &[(usize, usize, bool)]) -> Self {
        // The landing resolver: the real statement-level landing of a
        // fallthrough that leaves a construct at virtual position q, where
        // the leaving construct's tail is `from`. Innermost-first over the
        // constructs containing `from`: each redirect moves the landing
        // past one enclosing if arm (to its join) and rescans; a landing
        // past an enclosing loop body end is the loop's backedge. `skip`
        // excludes one construct (the resolver must not redirect through
        // the very edge it is resolving).
        fn land(
            mut q: usize,
            from: usize,
            spans: &[(usize, usize)],
            ifrs: &[(usize, usize, usize)],
            skip: &mut Vec<(u8, usize)>,
        ) -> usize {
            loop {
                let mut innermost_loop: Option<(usize, usize)> = None;
                for (k, &(a, b)) in spans.iter().enumerate() {
                    if a <= from && from <= b && !skip.contains(&(b'L', k)) {
                        let better = match innermost_loop {
                            Some((a0, _)) => a > a0,
                            None => true,
                        };
                        if better {
                            innermost_loop = Some((a, b));
                        }
                    }
                }
                if let Some((a, b)) = innermost_loop {
                    if q > b {
                        return a; // enclosing body completes → backedge
                    }
                }
                let mut innermost_if: Option<(usize, usize, usize, usize)> = None;
                for (k, &(hdr, t_hi, e_hi)) in ifrs.iter().enumerate() {
                    if hdr <= from && from <= e_hi && !skip.contains(&(b'I', k)) {
                        let better = match innermost_if {
                            Some((_, h0, _, _)) => hdr > h0,
                            None => true,
                        };
                        if better {
                            innermost_if = Some((k, hdr, t_hi, e_hi));
                        }
                    }
                }
                match innermost_if {
                    Some((k, _hdr, t_hi, e_hi)) => {
                        let arm_end = if from <= t_hi { t_hi } else { e_hi };
                        if q > arm_end {
                            skip.push((b'I', k));
                            q = e_hi + 1; // arm completes → resolve at the join
                            continue;
                        }
                        return q;
                    }
                    None => return q,
                }
            }
        }
        let mut succ: Vec<Vec<usize>> = (0..=n + 1).map(|p| vec![p + 1]).collect();
        // if diamonds: the header's true/false edges enter the then/else
        // arm when it consumed statements, else jump straight to the join
        // (an arm that emitted nothing is skipped, not entered)
        for (k, &(hdr, t_hi, e_hi)) in ifrs.iter().enumerate() {
            let then_arm = t_hi >= hdr + 1;
            let else_arm = e_hi >= t_hi + 1;
            let join = land(e_hi + 1, e_hi, spans, ifrs, &mut vec![(b'I', k)]);
            let te = if then_arm { hdr + 1 } else { join };
            let fe = if else_arm { t_hi + 1 } else { join };
            succ[hdr] = if te == fe { vec![te] } else { vec![te, fe] };
            if then_arm {
                succ[t_hi] = vec![land(t_hi + 1, t_hi, spans, ifrs, &mut vec![(b'I', k)])];
            }
            if else_arm {
                succ[e_hi] = vec![join];
            }
        }
        // while loops: header → body / exit, tail → header. An empty body
        // keeps the exit edge on the header itself (succ[b] would be the
        // header's own self overwrite). The tail overwrite is consistent
        // with any if-rewiring there: an arm ending at the loop tail
        // resolves its join to this same backedge.
        for (k, &(a, b)) in spans.iter().enumerate() {
            let exit = land(b + 1, b, spans, ifrs, &mut vec![(b'L', k)]);
            if b == a {
                let mut s = vec![a];
                if exit != a {
                    s.push(exit);
                }
                succ[a] = s;
            } else {
                succ[a] = vec![a + 1, exit];
                succ[b] = vec![a];
            }
        }
        // break/continue: their fallthrough is dead code; the jump goes to
        // the enclosing loop's exit / header
        for &(idx, sp, is_break) in brk_cont {
            let (a, b) = spans[sp];
            succ[idx] = vec![if is_break {
                land(b + 1, b, spans, ifrs, &mut vec![(b'L', sp)])
            } else {
                a
            }];
        }
        StmtCfg { succ }
    }
}

/// Backward may-liveness per statement for one slot's event list
/// (use-before-def): `live[p]` = the slot's value is live immediately
/// before statement p. Reads are uses (a read at a write's own index uses
/// the OLD value — evaluation precedes the store); WriteU and WriteC both
/// kill, but only on paths through the statement (the if rewiring carries
/// that). Loop backedges are real successors, so a value that may be
/// re-read in a later iteration stays live around the whole loop — while a
/// value fully redefined before every re-read (both diamond arms writing,
/// dominating arm writes) dies at the join. This is exactly the register
/// exclusivity the bank allocator needs: the unit interval must cover
/// every live statement plus every event statement.
pub(crate) fn cfg_live(cfg: &StmtCfg, evs: &[Ev], n: usize) -> Vec<bool> {
    let mut use_ = vec![false; n + 2];
    let mut kill = vec![false; n + 2];
    for e in evs {
        match e.kind {
            EvKind::Read => use_[e.idx] = true,
            EvKind::WriteU | EvKind::WriteC => kill[e.idx] = true,
        }
    }
    let mut live = vec![false; n + 3];
    loop {
        let mut changed = false;
        for p in (1..=n).rev() {
            let mut out = false;
            for &s in &cfg.succ[p] {
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

/// Build live segments for one slot from its (in-order) events.
///
/// Splitting rule (soundness, unchanged): split the timeline at each
/// unconditional write only when all those writes share one region R and
/// every read after the first write is region-contained in R — then each
/// execution that reaches a read of segment i has passed write i.
/// Otherwise one long segment.
///
/// Segment END (P2-A27-S2): the last statement at which the segment's
/// register must stay exclusively owned = the last statement where any
/// event maps to the segment (reads evaluate before boundary writes;
/// writes clobber) or the segment's value is cfg-live (a sharer there
/// would overwrite it before its reader — backedge re-reads included,
/// which the fixpoint folds in). This replaces the old conservative
/// extension to the outermost loop containing any read: a slot fully
/// redefined on every path before its re-reads (if-diamond both arms
/// writing) now ends at its true last read instead of hogging the
/// register to the loop end. `init` (carries the entry zero from the
/// prologue) is likewise exact: `live[1]` — only when some path reaches a
/// read without passing any write does the entry zero matter.
pub(crate) fn build_segments(evs: &[Ev], within: &WebWalk, cfg: &StmtCfg, n: usize, cond_read: bool) -> Vec<Seg> {
    if evs.is_empty() {
        return Vec::new();
    }
    let live = cfg_live(cfg, evs, n);
    let wu: Vec<usize> = (0..evs.len())
        .filter(|&p| matches!(evs[p].kind, EvKind::WriteU))
        .collect();
    // reads at a write's own statement index happen BEFORE that write
    // (evaluation precedes the store), so a boundary read belongs to the
    // PREVIOUS segment: segment k spans writes w_k..w_{k+1} with reads in
    // (w_k, w_{k+1}]
    let can_split = !cond_read
        && wu.len() >= 2 && {
            let r0 = evs[wu[0]].region;
            wu.iter().all(|&p| evs[p].region == r0)
                && evs.iter().all(|e| {
                    !matches!(e.kind, EvKind::Read)
                        || e.idx <= evs[wu[0]].idx
                        || within.region_within(e.region, r0)
                })
    };
    let mut segs: Vec<Seg> = Vec::new();
    if can_split {
        let w1 = evs[wu[0]].idx;
        // init window: every event strictly before the first unconditional
        // write, plus reads AT it (they evaluate first and see the older
        // value). Its end additionally covers every cfg-live statement up
        // to w1 — the entry/earlier value flowing to its readers.
        let init_mapped =
            |e: &Ev| e.idx < w1 || (e.idx == w1 && matches!(e.kind, EvKind::Read));
        if evs.iter().any(|e| init_mapped(e)) {
            let mut end = 0usize;
            for e in evs.iter().filter(|e| init_mapped(e)) {
                end = end.max(e.idx);
            }
            for p in 1..=w1 {
                if live[p] {
                    end = end.max(p);
                }
            }
            segs.push(Seg { start: evs[0].idx, end, init: live[1] });
        }
        for (i, &p) in wu.iter().enumerate() {
            let start = evs[p].idx;
            let next_w = wu.get(i + 1).map(|&q| evs[q].idx);
            let mapped = |e: &Ev| match e.kind {
                EvKind::Read => e.idx > start && next_w.map_or(true, |nw| e.idx <= nw),
                _ => e.idx >= start && next_w.map_or(true, |nw| e.idx < nw),
            };
            let mut end = start;
            for e in evs.iter().filter(|e| mapped(e)) {
                end = end.max(e.idx);
            }
            let hi = next_w.unwrap_or(n);
            for q in start + 1..=hi {
                if live[q] {
                    end = end.max(q);
                }
            }
            segs.push(Seg { start, end, init: false });
        }
        return segs;
    }
    // one segment: every event maps to it (single home)
    let mut end = 0usize;
    for e in evs {
        end = end.max(e.idx);
    }
    for p in 1..=n {
        if live[p] {
            end = end.max(p);
        }
    }
    segs.push(Seg { start: evs[0].idx, end, init: live[1] });
    segs
}

/* ---- Pass A: constant-select chains → byte-table lookup ---- */

/// Integer literal under the pinned subset (u64 domain), or None.
/// T7: a u128-width error-value node that is a compile-time constant
/// fitting u64 (paren groups wrap literals in IRNodeWrapOne — unwrap
/// first). None = dynamic or too wide (the callers keep the reject).
pub(crate) fn u128_err_literal(node: &dyn IRNode) -> Option<u64> {
    let mut cur = node;
    while let Some(w) = cur.as_any().downcast_ref::<IRNodeWrapOne>() {
        cur = &*w.node;
    }
    crate::lang::ir_literal_value(cur)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
        .filter(|&v| v <= u64::MAX as u128)
        .map(|v| v as u64)
}

pub(crate) fn int_literal(node: &dyn IRNode) -> Option<u64> {
    crate::lang::ir_literal_value(node)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
        .filter(|&n| n <= u64::MAX as u128)
        .map(|n| n as u64)
}

/// A bare int literal node (P0..P3/PU8/PU16) with no cast wrapper — safe
/// to fold into any width context (the CU* arms keep their range checks).
pub(crate) fn is_plain_int_literal(node: &dyn IRNode) -> bool {
    if node.as_any().downcast_ref::<IRNodeWrapOne>().is_some() {
        return false;
    }
    matches!(
        Bytecode::try_from_u8(node.bytecode()),
        Ok(Bytecode::P0) | Ok(Bytecode::P1) | Ok(Bytecode::P2) | Ok(Bytecode::P3)
            | Ok(Bytecode::PU8) | Ok(Bytecode::PU16)
    )
}

/// True when lowering this statement emits zero bytes (it may still have
/// vstack effects — the caller keeps lowering it normally).
pub(crate) fn is_zero_byte_stmt(node: &dyn IRNode) -> bool {
    if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
        || node.as_any().downcast_ref::<IRNodeText>().is_some()
    {
        return true;
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return false,
    };
    if matches!(op, Bytecode::NOP | Bytecode::ALLOC) {
        return true;
    }
    if matches!(op, Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR) {
        if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
            return arr.subs.iter().all(|n| is_zero_byte_stmt(&**n));
        }
    }
    false
}

/// True when control cannot fall out of the end of this statement
/// (return/throw/abort/break/continue): a following jump is dead.
pub(crate) fn ends_in_unconditional_jump(node: &dyn IRNode) -> bool {
    let mut n = node;
    loop {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            n = &*w.node;
            continue;
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return false,
        };
        if matches!(
            op,
            Bytecode::RET | Bytecode::END | Bytecode::ABT | Bytecode::ERR | Bytecode::IRBREAK | Bytecode::IRCONTINUE
        ) {
            return true;
        }
        if matches!(op, Bytecode::IRLIST | Bytecode::IRBLOCK | Bytecode::IRBLOCKR) {
            if let Some(arr) = n.as_any().downcast_ref::<IRNodeArray>() {
                if let Some(last) = arr.subs.last() {
                    n = &**last;
                    continue;
                }
            }
        }
        return false;
    }
}

/// Allocate the bank: interval-shared register assignment over the web
/// segments. Greedy by slot access count; slots whose segments do not all
/// fit are un-banked entirely (frame homes) and the pass repeats.
/// Merge (and coalesce adjacent) closed statement intervals.
pub(crate) fn merge_spans(mut v: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    v.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            Some(last) if s <= last.1.saturating_add(1) => {
                last.1 = last.1.max(e);
            }
            _ => out.push((s, e)),
        }
    }
    out
}

/// A bank candidate register and its busy intervals: closed statement
/// ranges during which the expression machinery may write the register
/// (empty for always-free registers). A bank segment may claim the
/// register only when its live range avoids every busy interval.
pub(crate) type BankCand = (u8, Vec<(usize, usize)>);

pub(crate) fn allocate_bank(mut within: WebWalk, counts: &[usize], regs: Vec<BankCand>, skip: &[bool]) -> WebPlan {
    let nslots = within.slot.len();
    if diag_env_enabled("RV64_EV_DEBUG") {
        for s in 0..nslots {
            let evs = &within.ev[s];
            if !evs.is_empty() {
                let idxs: Vec<String> = evs
                    .iter()
                    .map(|e| format!("{}{}", e.idx, match e.kind { EvKind::Read => "r", EvKind::WriteU => "W", EvKind::WriteC => "w" }))
                    .collect();
                println!("  ev slot {s}: {}", idxs.join(" "));
            }
        }
    }
    let mut segs: Vec<Vec<Seg>> = Vec::with_capacity(nslots);
    let n = within.idx;
    let cfg = StmtCfg::new(n, &within.while_spans, &within.ifrs, &within.brk_cont);
    // BUG-023: per-slot read-statement sets, snapshotted before the events
    // are moved into build_segments — the placement conflict test needs
    // them to tell a safe read->write register chain from a clobbering
    // touch at a shared boundary statement.
    let read_at: Vec<std::collections::HashSet<usize>> = (0..nslots)
        .map(|s| {
            within.ev[s]
                .iter()
                .filter(|e| matches!(e.kind, EvKind::Read))
                .map(|e| e.idx)
                .collect()
        })
        .collect();
    for s in 0..nslots {
        let mut evs = std::mem::take(&mut within.ev[s]);
        evs.sort_by_key(|e| (e.idx, !matches!(e.kind, EvKind::Read)));
        segs.push(build_segments(&evs, &within, &cfg, n, within.cond_read[s]));
    }
    struct Unit {
        slot: usize,
        seg: usize,
        start: usize,
        end: usize,
        bytes: bool,
        prio: usize,
    }
    let mut units: Vec<Unit> = Vec::new();
    for s in 0..nslots {
        // T13 stain fix: an Address slot is NOT banked (it would need the
        // two-register bytes unit, but its static stain must survive even
        // a register-pressure eviction — the frame cell is its only home).
        // Its web events still exist (DSE/forwarding soundness).
        if !matches!(within.slot[s], SlotTy::Int | SlotTy::Bytes) {
            continue;
        }
        // task 3: u128 slots keep their 16-byte frame cell (no bank)
        if skip.get(s).copied().unwrap_or(false) {
            continue;
        }
        let bytes = matches!(within.slot[s], SlotTy::Bytes);
        for (i, seg) in segs[s].iter().enumerate() {
            // Init segments used to map to [0, usize::MAX] — a whole-program
            // register hog. Only [0, seg.end] is actually live: the entry
            // zero is read within [seg.start, seg.end] and no read of the
            // slot can precede seg.start (it is the slot's first event), so
            // any other unit granted the register strictly after seg.end
            // cannot clobber a value anyone reads. The closed-overlap (and
            // busy) conflict test blocks every sharer inside [0, seg.end],
            // which is exactly the prologue-to-last-read window the old
            // comment worried about.
            let start = if seg.init { 0 } else { seg.start };
            let end = seg.end;
            units.push(Unit {
                slot: s,
                seg: i,
                start,
                end,
                bytes,
                // slot-global count priority (per-unit traffic × loop-depth
                // lifts were tried in S2 and measured WORSE on both anchors:
                // fragmenting the allocation by per-unit traffic starves
                // whole-slot residency — see docs.loc/rv64im/12-* §6)
                prio: counts[s],
            });
        }
    }
    units.sort_by(|a, b| {
        b.prio
            .cmp(&a.prio)
            .then(a.start.cmp(&b.start))
            .then(a.end.cmp(&b.end))
            .then(a.slot.cmp(&b.slot))
            .then(a.seg.cmp(&b.seg))
    });

    let mut banked = vec![BankKind::None; nslots];
    let mut homes: Vec<Vec<(usize, [u8; 2])>> = vec![Vec::new(); nslots];
    let mut assigned: Vec<Vec<(usize, usize)>>; // (list pos, unit idx)

    loop {
        assigned = (0..regs.len()).map(|_| Vec::new()).collect();
        banked = vec![BankKind::None; nslots];
        homes = vec![Vec::new(); nslots];
        let mut failed: Vec<usize> = Vec::new();
        let mut slot_placed = vec![false; nslots];
        let mut evicted: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut fail_why: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        for (uidx, u) in units.iter().enumerate() {
            if evicted.contains(&uidx) {
                continue;
            }
            if !slot_placed[u.slot] && failed.contains(&u.slot) {
                continue;
            }
            let need = if u.bytes { 2 } else { 1 };
            let mut placed: Vec<usize> = Vec::new();
            for ri in 0..regs.len() {
                placed.clear();
                let mut blockers: Vec<usize> = Vec::new();
                let mut ok = true;
                for k in 0..need {
                    let r = ri + k;
                    if r >= regs.len() {
                        ok = false;
                        break;
                    }
                    for &(_, uu) in &assigned[r] {
                        let a = &units[uu];
                        // BUG-023 (P2-C4 S6, 10^4 bombardment): Seg.end is
                        // INCLUSIVE (the value is live AT its end statement
                        // — the cfg fixpoint extends it across the backedge
                        // for loop counters), so the old strict test
                        // (`u.start < a.end`) treated ends as exclusive and
                        // co-located units that merely TOUCH at a boundary
                        // statement. Two shapes, both observed:
                        //   * the placed unit starts where the current one
                        //     ends (`while t0 < 1 { t0 += 1; var t1 = 0u64 }`
                        //     banked t1's init into t0's home — the counter
                        //     reset every iteration and ImcRemap spun to
                        //     OutOfGas while Im/Imc (frame homes, no
                        //     sharing) terminated);
                        //   * the placed unit ENDS where the current one
                        //     starts (priority order places a later-starting
                        //     unit first; its boundary write then clobbers
                        //     the current unit's backedge-live value).
                        // A touch IS safe exactly when the ENDING unit's
                        // slot READS at the boundary statement: reads
                        // evaluate before same-statement writes, so the
                        // value flows through the shared register (`t1 =
                        // t0 + 1` chains — the coalescing the strict test
                        // was written for). Same-slot touches are always
                        // safe (one value, in-place update at the boundary
                        // write).
                        let strict = a.start < u.end && u.start < a.end;
                        let mut touch_unsafe = false;
                        if !strict && a.slot != u.slot {
                            // The read-at-boundary exemption has a second,
                            // loop-shaped hole (BUG-025/026, P2-C4 S2 10^5
                            // round; seeds 0xB0DB...+94042's 4-line delta-min
                            // `var s0 = buf_left_drop(2, buf_left(8, data));
                            // while t0 < 16 { var s1 = buf_left_drop(1, s0) }`
                            // and an 11-line nested-loop spin): if the
                            // ENDING unit is LOOP-CARRIED into a while span
                            // containing the boundary — its def precedes the
                            // span (s0's def is stmt 1, the boundary sits at
                            // stmt 3 inside span (2,..)) — its value must
                            // survive that span's iterations, because its
                            // in-span reads re-execute every iteration with
                            // NO intervening re-def. The surviving slot's
                            // boundary write then clobbers the shared home
                            // and the next iteration's re-reads see the
                            // wrong value (s0's backedge re-read saw s1's
                            // handle — slice count check faulted 71; or a
                            // sharer took a loop counter's home and
                            // ImcRemap spun to OutOfGas) while Im/Imc
                            // (frame homes, no sharing) and the interpreter
                            // ran clean. A unit whose def lies INSIDE every
                            // span containing the boundary is safe: the
                            // next iteration re-executes its def before any
                            // of its reads (the hacd200 anchor's 8 in-loop
                            // boundary chains are all of this shape and
                            // keep their homes — 3072 B pinned).
                            let carried_into = |unit_start: usize, k: usize| {
                                within
                                    .while_spans
                                    .iter()
                                    .any(|&(w0, w1)| w0 <= k && k <= w1 && unit_start < w0)
                            };
                            if a.end == u.start
                                && (carried_into(a.start, a.end)
                                    || !read_at[a.slot].contains(&a.end))
                            {
                                touch_unsafe = true;
                            }
                            if u.end == a.start
                                && (carried_into(u.start, u.end)
                                    || !read_at[u.slot].contains(&u.end))
                            {
                                touch_unsafe = true;
                            }
                        }
                        if (strict || touch_unsafe) && !blockers.contains(&uu) {
                            blockers.push(uu);
                        }
                    }
                    if regs[r].1.iter().any(|&(b0, b1)| b0 <= u.end && u.start <= b1) {
                        ok = false;
                    }
                    if !ok {
                        placed.clear();
                        break;
                    }
                    placed.push(r);
                }
                if !ok || placed.len() != need {
                    continue;
                }
                if blockers.iter().any(|&b| b <= uidx) {
                    // this window is unusable: clear `placed` so the last
                    // attempted window cannot leak past the failed check
                    // below (the BUG-012 double-assignment: with every
                    // window conflicted, a filled `placed` escaped the
                    // `placed.len() != need` guard and two slots shared a
                    // register — wrong values, no INV_CHECK hit)
                    placed.clear();
                    continue;
                }
                for b in blockers.drain(..) {
                    for a in assigned.iter_mut() {
                        a.retain(|&(_, uu)| uu != b);
                    }
                    evicted.insert(b);
                }
                break;
            }
            if placed.len() != need {
                failed.push(u.slot);
                continue;
            }
            slot_placed[u.slot] = true;
            for r in &placed {
                assigned[*r].push((*r, uidx));
            }
        }
        // derive banked flags + homes from what survived the round
        for (uidx, u) in units.iter().enumerate() {
            let mut pair = [0u8; 2];
            let mut got = 0usize;
            for (k, occ) in assigned.iter().enumerate() {
                if occ.iter().any(|&(_, uu)| uu == uidx) {
                    pair[got] = regs[k].0;
                    got += 1;
                }
            }
            if got == 0 {
                continue;
            }
            debug_assert_eq!(got, if u.bytes { 2 } else { 1 }, "split placement");
            homes[u.slot].push((u.start, pair));
        }
        for (s, h) in homes.iter_mut().enumerate() {
            h.sort_by_key(|(b, _)| *b);
            h.dedup();
            if !h.is_empty() {
                banked[s] = if matches!(within.slot[s], SlotTy::Bytes) {
                    BankKind::Bytes
                } else {
                    BankKind::Int
                };
            }
        }
        if failed.is_empty() {
            if diag_env_enabled("RV64_EV_DEBUG") {
                for (uidx, u) in units.iter().enumerate() {
                    let got = assigned
                        .iter()
                        .filter(|occ| occ.iter().any(|&(_, uu)| uu == uidx))
                        .count();
                    let want = if u.bytes { 2 } else { 1 };
                    if got != 0 && got != want {
                        println!(
                            "  bank-PARTIAL slot {} seg{} [{},{}] got {} want {}",
                            u.slot, u.seg, u.start, u.end, got, want
                        );
                    }
                    if got == 0 {
                        println!(
                            "  bank-EVICTED slot {} seg{} [{},{}]",
                            u.slot, u.seg, u.start, u.end
                        );
                    }
                }
            }
            break;
        }
        if diag_env_enabled("RV64_EV_DEBUG") {
            let mut keys: Vec<_> = fail_why.keys().copied().collect();
            keys.sort();
            for s in keys {
                println!("  bank-FAIL {}", fail_why[&s]);
            }
        }
        // un-bank failed slots entirely and retry (their frame homes return)
        let before = units.len();
        units.retain(|u| !failed.contains(&u.slot));
        if units.len() == before {
            if diag_env_enabled("RV64_EV_DEBUG") {
                let mut keys: Vec<_> = fail_why.keys().copied().collect();
                keys.sort();
                for s in keys {
                    println!("  bank-FAIL {}", fail_why[&s]);
                }
                for (ri, (r, busy)) in regs.iter().enumerate() {
                    let spans: Vec<String> =
                        busy.iter().map(|&(b0, b1)| format!("[{b0},{b1}]")).collect();
                    let occ: Vec<String> = assigned[ri]
                        .iter()
                        .map(|&(_, uu)| format!("s{}[{},{}]", units[uu].slot, units[uu].start, units[uu].end))
                        .collect();
                    println!(
                        "  bank-reg x{} busy={} occ={}",
                        r,
                        spans.join(""),
                        occ.join(",")
                    );
                }
            }
            break;
        }
    }
    for h in homes.iter_mut() {
        h.sort_by_key(|(b, _)| *b);
    }
    if diag_env_enabled("RV64_EV_DEBUG") {
        // P2-A27-S2: per-slot segment table + final homes (the structure the
        // census/probe diffing reads; no codegen effect)
        let banked_n = banked
            .iter()
            .filter(|&&k| matches!(k, BankKind::Int | BankKind::Bytes))
            .count();
        println!("  bank-final: {banked_n} banked slots, {n} stmts");
        for s in 0..nslots {
            let sg: Vec<String> = segs[s]
                .iter()
                .map(|g| {
                    format!(
                        "[{},{}]{}",
                        g.start,
                        g.end,
                        if g.init { "i" } else { "" }
                    )
                })
                .collect();
            let hm: Vec<String> = homes[s]
                .iter()
                .map(|(b, r)| {
                    format!(
                        "{}->x{}{}",
                        b,
                        r[0],
                        if matches!(banked.get(s), Some(BankKind::Bytes)) {
                            format!("x{}", r[1])
                        } else {
                            String::new()
                        }
                    )
                })
                .collect();
            let kind = match banked.get(s) {
                Some(BankKind::Int) => "INT",
                Some(BankKind::Bytes) => "BYTES",
                _ => "frame",
            };
            println!("  seg slot {s} {kind}: {} homes={}", sg.join(" "), hm.join(" "));
        }
    }
    let mut init_zero = vec![false; nslots];
    for (s, sg) in segs.iter().enumerate() {
        init_zero[s] = sg.first().map(|x| x.init).unwrap_or(false);
    }
    WebPlan { banked, homes, init_zero, segs_debug: segs, no_split_debug: within.cond_read.clone() }
}

/// Compute the whole promotion plan for a body: event walk + segments +
/// bank allocation. Each candidate register carries its busy intervals —
/// closed statement ranges during which the expression machinery may write
/// it (pool registers above the base depth, see `lower_body_mode_arena`);
/// a segment's live range must avoid all of them.
pub(crate) fn web_plan(ir: &IRNodeArray, fp: &FramePlan, regs_available: &[BankCand]) -> WebPlan {
    let mut w = WebWalk::new(&fp.slot);
    // XOP's old-value read MUST be visible to the segment builder: the
    // read-modify-write evaluates the old value (from the PREVIOUS segment's
    // register — `Lower`'s XOP arm reads `bank_int_read` at the write's own
    // statement index) before the new segment's write lands. With the legacy
    // pure-write plan events that boundary read was invisible, so a segment
    // ended early and the allocator freely shared its register with another
    // slot inside (seg.end, xop idx) — the XOP then read the sharer's value
    // (BUG-008: `a=7; b=a; c=9; a+=1` returned 10 in ImcRemap). Recording
    // the read makes the old segment's unit cover the boundary statement, so
    // the overlap check protects it and `home_at`'s read-side coverage
    // invariant holds for every emitted read.
    w.xop_reads = true;
    for n in &ir.subs {
        w.walk_stmt(&**n);
    }
    let mirror: usize = ir.subs.iter().map(|n| stmt_walk_count(&**n)).sum();
    debug_assert_eq!(mirror, w.idx, "stmt_walk_count mirror desync");
    let (reads, writes) = slot_use_counts(&ir.subs);
    let mut counts = vec![0usize; fp.slot.len()];
    for (i, c) in counts.iter_mut().enumerate() {
        *c = reads.get(i).copied().unwrap_or(0) + writes.get(i).copied().unwrap_or(0);
    }
    // task 3: a u128 slot's value is a (lo, hi) register pair — the single
    // register homes of the local bank cannot hold it, so the slot stays in
    // the frame (reads/writes go through the 16-byte cell).
    let skip: Vec<bool> = (0..fp.slot.len())
        .map(|i| fp.slot_w.get(i).copied().flatten() == Some(128))
        .collect();
    allocate_bank(w, &counts, regs_available.to_vec(), &skip)
}

/// Count the sites that will reference a pool literal (PBUF-family
/// evaluations, literal PUT/CBYTES/CAT folds) — drives the pool-base
/// register decision.
pub(crate) fn count_literal_sites(nodes: &[Box<dyn IRNode>]) -> usize {
    fn walkc(n: &dyn IRNode, count: &mut usize) {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walkc(&*w.node, count);
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        use Bytecode::*;
        match op {
            PBUF | PBUFL | PNBUF => *count += 1,
            PUT => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    if bytes_literal(&*p.subx).is_some() {
                        *count += 1;
                    }
                }
            }
            CBYTES => {
                if let Some(single) = n.as_any().downcast_ref::<IRNodeSingle>() {
                    if crate::lang::ir_literal_value(&*single.subx).ok().flatten().is_some() {
                        *count += 1;
                    }
                }
            }
            CAT => {
                if let Some(d) = n.as_any().downcast_ref::<IRNodeDouble>() {
                    if bytes_literal(&*d.subx).is_some() && bytes_literal(&*d.suby).is_some() {
                        *count += 1;
                    }
                }
            }
            _ => {}
        }
        for c in children(n) {
            walkc(c, count);
        }
    }
    let mut count = 0;
    for n in nodes {
        walkc(&**n, &mut count);
    }
    count
}

/// Static per-slot access counts over the whole body: (reads, writes),
/// where XOP counts once each way (it reads and writes the slot).
pub(crate) fn slot_use_counts(nodes: &[Box<dyn IRNode>]) -> (Vec<usize>, Vec<usize>) {
    fn walkc(n: &dyn IRNode, reads: &mut Vec<usize>, writes: &mut Vec<usize>) {
        if let Some(w) = n.as_any().downcast_ref::<IRNodeWrapOne>() {
            return walkc(&*w.node, reads, writes);
        }
        let op = match Bytecode::try_from_u8(n.bytecode()) {
            Ok(o) => o,
            Err(_) => return,
        };
        use Bytecode::*;
        match op {
            GET | GET0 | GET1 | GET2 | GET3 => {
                let idx = match op {
                    GET => {
                        n.as_any().downcast_ref::<IRNodeParam1>().map(|p| p.para as usize)
                    }
                    GET0 => Some(0),
                    GET1 => Some(1),
                    GET2 => Some(2),
                    _ => Some(3),
                };
                if let Some(i) = idx {
                    if reads.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    reads[i] += 1;
                }
            }
            PUT => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    let i = p.para as usize;
                    if writes.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    writes[i] += 1;
                }
            }
            XOP => {
                if let Some(p) = n.as_any().downcast_ref::<IRNodeParam1Single>() {
                    let (_, idx) = decode_local_operand_mark(p.para);
                    let i = idx as usize;
                    if writes.len() <= i {
                        reads.resize(i + 1, 0);
                        writes.resize(i + 1, 0);
                    }
                    reads[i] += 1;
                    writes[i] += 1;
                }
            }
            // T8: the high-slot local compound fragment counts once each
            // way, like XOP (the children walk below still counts the
            // value subtree's own reads)
            IRLIST
                if n
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .map(|a| local_compound_assign_parts(&*a).is_some())
                    .unwrap_or(false) =>
            {
                let (_, idx, _) = local_compound_assign_parts(n).unwrap();
                let i = idx as usize;
                if writes.len() <= i {
                    reads.resize(i + 1, 0);
                    writes.resize(i + 1, 0);
                }
                reads[i] += 1;
                writes[i] += 1;
            }
            _ => {}
        }
        for c in children(n) {
            walkc(c, reads, writes);
        }
    }
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for n in nodes {
        walkc(&**n, &mut reads, &mut writes);
    }
    (reads, writes)
}

/// Index of the slot a GET/GET0..3 node reads, or None.
pub(crate) fn slot_get_index(node: &dyn IRNode) -> Option<u8> {
    let op = Bytecode::try_from_u8(node.bytecode()).ok()?;
    match op {
        Bytecode::GET => Some(node.as_any().downcast_ref::<IRNodeParam1>()?.para),
        Bytecode::GET0 => Some(0),
        Bytecode::GET1 => Some(1),
        Bytecode::GET2 => Some(2),
        Bytecode::GET3 => Some(3),
        // T11: a constant-index local_x(i) IS a GET i for every slot-aware
        // walk (web reads, pair slot-touch, loop-carried marks)
        Bytecode::GETX => local_x_slot_index(
            &*node.as_any().downcast_ref::<IRNodeSingle>()?.subx,
            "GETX",
        )
        .ok(),
        _ => None,
    }
}

/// T11 (pure-compute subset extension): the constant local slot index of a
/// GETX/PUTX index operand, as the interpreter's `ops_{peek,pop}_to_u16`
/// would read it. `Err` = the reject face (a dynamic index, a literal the
/// u16 extractor would fault CastParamFail(72) on at run time, or a
/// non-uint literal).
pub(crate) fn local_x_slot_index(idx_node: &dyn IRNode, op: &str) -> Ret<u8> {
    let lit = crate::lang::ir_literal_value(idx_node).ok().flatten();
    let Some(v) = lit else {
        return errf!(
            "rv64 lower: opcode {op} is outside the translatable kernel subset \
             (a dynamic local index — the kernel frame is statically addressed)"
        );
    };
    let n = match v.extract_u128() {
        Ok(n) => n,
        Err(_) => {
            return errf!(
                "rv64 lower: opcode {op} is outside the translatable kernel subset \
                 (the local index must be a u16 literal)"
            )
        }
    };
    if n > u16::MAX as u128 {
        return errf!(
            "rv64 lower: opcode {op} is outside the translatable kernel subset \
             (local index {n} exceeds u16 — the interpreter faults CastParamFail(72))"
        );
    }
    if n > u8::MAX as u128 {
        return errf!(
            "rv64 lower: opcode {op} is outside the translatable kernel subset \
             (local index {n} exceeds the kernel's 256-slot frame)"
        );
    }
    Ok(n as u8)
}

/// The single effective statement of an arm body (looking through wrappers
/// and single-statement blocks), or None when the body is empty/compound.
pub(crate) fn effective_single(node: &dyn IRNode) -> Option<&dyn IRNode> {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return effective_single(&*w.node);
    }
    if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
        let mut eff = arr.subs.iter().filter(|s| !is_zero_byte_stmt(&***s));
        let first = eff.next()?;
        if eff.next().is_some() {
            return None;
        }
        return effective_single(&**first);
    }
    if is_zero_byte_stmt(node) {
        return None;
    }
    Some(node)
}

/// One matched arm of a constant-select chain.
pub(crate) struct SelArm {
    pub(crate) key: u64,
    pub(crate) val: u64,
}

/// Match `IRIF (EQ <GET s> <int lit>) { PUT w <int lit ≤ 255> }` with an
/// empty else. Returns (read slot, write slot, key, value).
pub(crate) fn match_sel_arm(node: &dyn IRNode) -> Option<(u8, u8, SelArm)> {
    let n = node.as_any().downcast_ref::<IRNodeTriple>()?;
    if n.inst != Bytecode::IRIF {
        return None;
    }
    let c = n.subx.as_any().downcast_ref::<IRNodeDouble>()?;
    if c.inst != Bytecode::EQ {
        return None;
    }
    let (g, k) = match (
        slot_get_index(&*c.subx),
        int_literal(&*c.subx),
        slot_get_index(&*c.suby),
        int_literal(&*c.suby),
    ) {
        (Some(s), None, None, Some(k)) => (s, k),
        (None, Some(k), Some(s), None) => (s, k),
        _ => return None,
    };
    let body = effective_single(&*n.suby)?;
    let p = body.as_any().downcast_ref::<IRNodeParam1Single>()?;
    if p.inst != Bytecode::PUT {
        return None;
    }
    let v = int_literal(&*p.subx)?;
    if v > 255 {
        return None;
    }
    if !is_zero_byte_stmt(&*n.subz) {
        return None;
    }
    Some((g, p.para, SelArm { key: k, val: v }))
}

/// True when the statement before a chain (skipping zero-byte statements)
/// is an unconditional `PUT slot D` — the default value D that table holes
/// must reproduce (nothing else writes the slot between D and the chain).
pub(crate) fn default_predecessor(subs: &[Box<dyn IRNode>], run_start: usize, slot: u8) -> Option<u64> {
    let mut i = run_start;
    while i > 0 {
        i -= 1;
        let n = &*subs[i];
        if is_zero_byte_stmt(n) {
            continue;
        }
        let p = n.as_any().downcast_ref::<IRNodeParam1Single>()?;
        if p.inst != Bytecode::PUT || p.para != slot {
            return None;
        }
        return int_literal(&*p.subx).filter(|v| *v <= 255);
    }
    None
}

/// True when the condition subtree cannot fault (literals, untracked slot
/// reads, comparisons over fault-free operands, logical ops over fault-free
/// operands only). Fault-free conjunctions can be lowered as branch chains
/// without changing observable behavior; impure ones must keep eager
/// evaluation (every operand must still execute — the bytecode channel is
/// eager for AND/OR, so a skipped operand would swallow a consensus-visible
/// trap; BUG-014). A read of a Nil-TRACKED slot counts as faultable: its
/// consumer guard must fire (BUG-012).
pub(crate) fn is_pure_cond(node: &dyn IRNode, tracked: &[bool]) -> bool {
    if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
        return is_pure_cond(&*w.node, tracked);
    }
    let op = match Bytecode::try_from_u8(node.bytecode()) {
        Ok(o) => o,
        Err(_) => return false,
    };
    use Bytecode::*;
    match op {
        P0 | P1 | P2 | P3 | PU8 | PU16 | PTRUE | PFALSE => true,
        GET | GET0 | GET1 | GET2 | GET3 => match slot_get_index(node) {
            Some(i) => !tracked.get(i as usize).copied().unwrap_or(false),
            None => true,
        },
        // comparisons and logical wraps are fault-free ONLY when every
        // operand is: `byte(data,25) > 0` faults, so it must not join a
        // short-circuit chain
        EQ | NEQ | LT | GT | LE | GE | NOT | AND | OR => {
            children(node).iter().all(|c| is_pure_cond(*c, tracked))
        }
        _ => false,
    }
}
