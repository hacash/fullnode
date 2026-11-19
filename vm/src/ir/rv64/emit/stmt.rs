use super::super::*;

impl<'a> Lower<'a> {

    /* ---- top level ---- */

    /// Prologue + body + fallthrough exit. (Thunks + trap epilogue are
    /// appended by `lower_body_with_input`.)
    pub(crate) fn emit_prologue_body_epilogue(&mut self, nodes: &[Box<dyn IRNode>]) -> Ret<()> {
        if self.frame_bytes > 2040 {
            if self.heap_bytes > 0 {
                // task 7: the heap reservation rides the SAME kernel memory
                // budget as the list blocks and the divmod workspace — the
                // 2040B S0 window. A reservation that does not fit rejects
                // at compile time (iron rule: budget faces may differ, and
                // this one is decided before any execution).
                return errf!(
                    "rv64 lower: the constant heap_grow reserves {}B of frame for the heap \
                     and the locals frame {}B exceeds the 2040B kernel memory budget; hint: \
                     fewer simultaneously-live locals (or a smaller segment count) shrink \
                     the frame",
                    self.heap_bytes,
                    self.frame_bytes
                );
            }
            // GT-c: actionable hint (doc 66 §1.4 rated "medium": said "should suggest
                    // reducing locals / compacting" — say how)
                    return errf!(
                        "rv64 lower: locals frame {}B exceeds the addi range; hint: fewer \
                         simultaneously-live locals shrink the frame — reuse one name for \
                         values with disjoint lifetimes",
                        self.frame_bytes
                    );
        }
        // prologue: capture the ABI registers, init the arena and frame base.
        self.asm.mv(S2, 12); // a2 output_base
        self.asm.mv(S3, 13); // a3 output_max
        self.asm.mv(S4, 10); // a0 input_base
        self.asm.mv(S5, 11); // a1 input_len
        self.asm.li(S1, ARENA_BASE);
        self.asm.addi(S0, SP, -(self.frame_bytes as i32));
        if let Some((r, l)) = self.pool_base {
            self.asm.pcrel_addr(r, l);
        }
        // Zero only the slots a read could reach before the first write.
        // Banked (promoted) slots zero their register home instead — a
        // 2-byte c.li against a 4-byte sd of x0.
        // sd x0 never compresses (c.sd needs rs2 in the window): when two
        // or more stores need zeroing, a window register loaded with 0
        // makes each a 2-byte c.sd (LLVM's MakeCompressible trick). The
        // pool is empty here, so its first (window) register is free.
        let nz: Vec<usize> = (0..self.slot.len())
            .filter(|&i| {
                !matches!(self.slot[i], SlotTy::Input)
                    && self.plan.banked.get(i).copied().unwrap_or(BankKind::None) == BankKind::None
                    && self.needs_zero.get(i).copied().unwrap_or(true)
            })
            .collect();
        let zero_stores = nz
            .iter()
            .map(|&i| {
                if matches!(self.slot[i], SlotTy::Bytes) || self.slot_w[i] == Some(128) {
                    2
                } else {
                    1
                }
            })
            .sum::<usize>();
            // task 7 / D5: the heap region no longer joins this count —
            // its zeroing is a counted loop whose single sd encodes 4B
            // with either zero source, so a MakeCompressible register can
            // never pay for its (8+seg·256)/8 words. Only the SLOT stores
            // above decide zreg; a heap-only body skips the dead `li 0`.
        let zreg = if zero_stores >= 2 { Some(self.vregs[0]) } else { None };
        if let Some(r) = zreg {
            self.asm.addi(r, ZERO, 0);
        }
        for (i, s) in self.slot.iter().enumerate() {
            if matches!(s, SlotTy::Input) {
                continue;
            }
            if self.plan.banked.get(i).copied().unwrap_or(BankKind::None) != BankKind::None {
                // a banked slot needs the entry zero only when its first
                // web segment is an init segment (first touch is a read or
                // a conditional write); a write-first slot re-defines its
                // value before any read, so the zero would be dead bytes
                if self.plan.init_zero.get(i).copied().unwrap_or(false) {
                    if let Some(regs) = self.plan.home_at(i, 0, true) {
                        match self.plan.banked[i] {
                            BankKind::Int => self.asm.addi(regs[0], ZERO, 0),
                            BankKind::Bytes => {
                                self.asm.addi(regs[0], ZERO, 0);
                                self.asm.addi(regs[1], ZERO, 0);
                            }
                            BankKind::None => unreachable!(),
                        }
                    }
                }
                continue;
            }
            if self.needs_zero.get(i).copied().unwrap_or(true) {
                let zr = match zreg {
                    Some(z) => z,
                    None => ZERO,
                };
                if let Some(o) = self.frame_off.get(i).filter(|o| **o >= 0) {
                    self.asm.sd(zr, S0, *o);
                    // a bytes descriptor and a u128 cell both occupy 16 bytes
                    if matches!(s, SlotTy::Bytes) || self.slot_w[i] == Some(128) {
                        self.asm.sd(zr, S0, *o + 8);
                    }
                }
            }
        }
        // narrow-PUT face: the width shadows start at the u64 face (a
        // never-assigned slot reads as the u64 zero on both engines; the
        // first assignment re-stores the shadow with its own width)
        for i in 0..self.slot.len() {
            if self.dyn_w_flag(i as u8) {
                if let Some(&off) = self.dyn_w_off.get(i) {
                    if off >= 0 {
                        self.asm.addi(self.asm.sc1, ZERO, 8);
                        self.asm.sb(self.asm.sc1, S0, off);
                    }
                }
            }
        }
        // BUG-012: Nil-init flags start CLEARED (every tracked slot reads
        // as Nil until its first store). The tag stack needs no zeroing —
        // every tag byte is written before any read (write-on-produce /
        // read-on-consume at the same vstack depth). The region is word
        // aligned (see the constructor).
        if self.tag_base >= 0 {
            let end = self.tag_base + 8;
            let mut o = self.nil_flag_start;
            while o < end {
                self.asm.sd(ZERO, S0, o);
                o += 8;
            }
        }
        // task 7 / D5 (fin-control survey): the heap region reads as zeros
        // — the flag word (length 0 until the grow executes) and the whole
        // reserved segment block (heap_grow zero-fills the grown bytes).
        // The stores are a 5-instruction counted loop over the frame
        // window (cursor a0, end address a1 — the documented loop/address
        // scratch pair: the ABI registers are already captured into
        // S2..S5, and in every mode the pair sits outside the value pool,
        // the bank candidates and sc1/sc2, so nothing live crosses the
        // prologue here). Write content and order are byte-identical to
        // the old static unroll: ascending 8-byte words of zero from
        // heap_flag_off to heap_off + heap_bytes (zreg may carry the zero
        // from the slot pass above; ZERO stores encode the same size —
        // the loop's sd is 4B with either source, so the heap no longer
        // pays for the MakeCompressible register, see zero_stores below).
        // Measured against the unrolled form (K=0: the loop wins from the
        // smallest admitted reservation): unroll = (8+seg·256)/8 sd
        // stores — 33 stores ≈132B at seg=1 up to 225 ≈900B at seg=7
        // (frame budget 2040B caps seg at 7); the loop is 5 instructions,
        // 16-20B, at every seg. Cycles rise (≈4 insts/word executed
        // before the grow's readers) — this pass optimizes static size.
        if self.heap_bytes > 0 {
            let zr = zreg.unwrap_or(ZERO);
            let end = self.heap_off + self.heap_bytes as i32;
            let p = self.a0;
            let e = self.a1;
            let l = self.asm.new_label();
            if self.heap_flag_off == 0 {
                self.asm.mv(p, S0);
            } else {
                self.asm.addi(p, S0, self.heap_flag_off);
            }
            self.asm.addi(e, S0, end);
            self.place(l);
            self.asm.sd(zr, p, 0);
            self.asm.addi(p, p, 8);
            self.asm.bne(p, e, l);
        }
        // body
        self.stmt_list(nodes, false)?;
        if !self.vstack.is_empty() {
            return errf!("rv64 lower: value stack not empty at function end");
        }
        // fallthrough: empty output — dead when the body cannot fall out
        // (ends in return/throw/abort/break/continue)
        let falls_out = nodes
            .last()
            .map(|n| !ends_in_unconditional_jump(&**n))
            .unwrap_or(true);
        if falls_out {
            self.asm.addi(A0, ZERO, 0);
            self.asm.ret();
        }
        Ok(())
    }

    /* ---- statements ---- */

    pub(crate) fn stmt_list(&mut self, nodes: &[Box<dyn IRNode>], keep_last: bool) -> Ret<()> {
        let last = nodes.len().saturating_sub(1);
        let mut i = 0usize;
        while i < nodes.len() {
            // top-level constant-select runs (the array arm covers nested
            // blocks; this covers the function body itself)
            if !keep_last || i != last {
                let run_o0 = self.asm.buf.len();
                if let Some(consumed) = self.try_const_select_run(nodes, i)? {
                    let cnt: usize = nodes[i..i + consumed]
                        .iter()
                        .map(|n| stmt_walk_count(&**n))
                        .sum::<usize>();
                    // one table replaces the whole arm run: attribute its
                    // bytes to the run's walk-index span ([base+1 .. base+cnt])
                    if let Some(v) = self.stmt_trace.as_mut() {
                        v.push(StmtRangeDbg {
                            w_lo: self.web_idx + 1,
                            w_hi: self.web_idx + cnt,
                            off_start: run_o0,
                            off_end: self.asm.buf.len(),
                        });
                    }
                    self.web_idx += cnt;
                    i += consumed;
                    continue;
                }
            }
            let n = &nodes[i];
            // P3-PAIRPKG: a recognized remove+insert pair — the fused
            // overwrite is emitted at the REMOVE root, the INSERT sibling
            // is subsumed (its dispatch is skipped, so the web_idx cursor
            // advances by both statements' counts explicitly).
            if let Some(ins) = self.pair_sibling(nodes, i) {
                self.web_idx += stmt_walk_count(&*nodes[i]) + stmt_walk_count(&*nodes[i + 1]);
                self.emit_pair(&*nodes[i], ins)?;
                i += 2;
                continue;
            }
            let keep = keep_last && i == last;
            self.stmt_one(&**n, keep)?;
            i += 1;
        }
        Ok(())
    }

    /// Lower one statement/expression node. `keep`: a trailing value must
    /// remain on the vstack (expression-position block).
    pub(crate) fn stmt_one(&mut self, node: &dyn IRNode, keep: bool) -> Ret<()> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.stmt_one(&*w.node, keep);
        }
        // Debug side-channel: attribute the emitted bytes to the statement's
        // walk-index range. Mirrors `WebWalk::walk_stmt` numbering (every
        // statement node counts exactly one idx, consumed by the inner call).
        if self.stmt_trace.is_some() {
            let w0 = self.web_idx;
            let o0 = self.asm.buf.len();
            let r = self.stmt_one_inner(node, keep);
            if r.is_ok() {
                if let Some(v) = self.stmt_trace.as_mut() {
                    v.push(StmtRangeDbg {
                        w_lo: w0 + 1,
                        w_hi: self.web_idx,
                        off_start: o0,
                        off_end: self.asm.buf.len(),
                    });
                }
            }
            return r;
        }
        self.stmt_one_inner(node, keep)
    }

    pub(crate) fn stmt_one_inner(&mut self, node: &dyn IRNode, keep: bool) -> Ret<()> {
        // statement-index cursor: +1 per statement node, mirroring
        // `WebWalk::walk_stmt` exactly (WRAP transparent, Empty counted)
        self.web_idx += 1;
        if node.as_any().downcast_ref::<IRNodeEmpty>().is_some()
            || node.as_any().downcast_ref::<IRNodeText>().is_some()
        {
            return Ok(());
        }
        // P3-IDIOMS: a chain-continuation PUT whose value is subsumed by
        // the fused wide load emitted at the chain head (the recognition
        // refuses keep-tail chains, so a skipped statement is never the
        // kept value; the PUT statement form has no value of its own).
        if let Some(FuseMark::Skip) = self.fuse.get(&node_addr(fuse_unwrap(node))) {
            return Ok(());
        }
        // P3-PAIRPKG: a recognized pair's INSERT is fully emitted at the
        // REMOVE root (one guarded sd). This early return happens after the
        // web_idx increment so the statement numbering stays aligned with
        // WebWalk::walk_stmt (plan refuses statement forms inside the pair
        // subtrees, so no deeper counts are skipped).
        if self.pair_skip.contains(&node_addr(fuse_unwrap(node))) {
            return Ok(());
        }
        // B2 task 5: the compound item assignment `l[i] op= v` — the whole
        // IRLIST is consumed natively (the raw DUPN/ROLL fragment is NOT
        // spliced; see `compound_assign_parts`). The web cursor advances
        // INSIDE emit_item_compound: the value/index evals must read their
        // bank homes at the SUBS' own walk positions (INV-check invariant),
        // not past the whole statement.
        let is_compound = node
            .as_any()
            .downcast_ref::<IRNodeArray>()
            .map(|a| a.inst == Bytecode::IRLIST && compound_assign_parts(&*a).is_some())
            .unwrap_or(false);
        if is_compound {
            if keep {
                return errf!(
                    "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                     (the compound-assignment result is a list value)"
                );
            }
            let (value, receiver, index, aop) = compound_assign_parts(node).unwrap();
            return self.emit_item_compound(value, receiver, index, aop);
        }
        // T8: the high-slot local compound assignment `x op= v` — the whole
        // IRLIST is consumed natively with the XOP semantics (the raw
        // GET/SWAP/op/PUT fragment is NOT spliced). The web cursor STAYS at
        // this statement's position: WebWalk recorded the value's reads and
        // the slot's read+write events all at this one index (the
        // XOP-shaped one-statement numbering, unlike the item compound's
        // per-sub positions).
        if node
            .as_any()
            .downcast_ref::<IRNodeArray>()
            .map(|a| a.inst == Bytecode::IRLIST && local_compound_assign_parts(&*a).is_some())
            .unwrap_or(false)
        {
            let (value, idx, aop) = local_compound_assign_parts(node).unwrap();
            // P3-V2IMPL defensive: a list slot is plan-rejected (the
            // interpreter faults CastFail(71) on the Compo old value)
            if matches!(self.slot.get(idx as usize), Some(SlotTy::List)) {
                return errf!(
                    "rv64 lower: opcode XOP is outside the translatable kernel subset \
                     (compound assignment on a list is not supported)"
                );
            }
            let lxop = match aop {
                Bytecode::ADD => LxOp::Add,
                Bytecode::SUB => LxOp::Sub,
                Bytecode::MUL => LxOp::Mul,
                Bytecode::DIV => LxOp::Div,
                _ => unreachable!("local_compound_parts checked the op"),
            };
            // the compound result keeps the slot/rhs common width — the
            // same join the XOP node's width oracle performs (the fragment
            // IRLIST has no retval, so arith_w(node) cannot be used here)
            let old_w = match self.slot.get(idx as usize) {
                Some(SlotTy::Int) => self.slot_w.get(idx as usize).copied().flatten(),
                _ => None,
            };
            let w = match widen2(old_w, self.int_width(value)) {
                Some(w) if w >= 8 => w,
                _ => 64,
            };
            // BUG-012: the OLD value's Nil check runs before the op (the
            // occ entry rides the fragment IRLIST node — see nil_plan)
            let xop_old_nil = self
                .nil_occ
                .get(&node_addr(node))
                .map(|o| o.xop_old)
                .unwrap_or(false);
            return self.emit_xop_stmt(idx, lxop, value, w, xop_old_nil);
        }
        let op = Bytecode::try_from_u8(node.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            IRLIST | IRBLOCK | IRBLOCKR => {
                let arr = node
                    .as_any()
                    .downcast_ref::<IRNodeArray>()
                    .ok_or("rv64 lower: block node shape unexpected")?;
                let keep_last = keep && matches!(op, IRBLOCKR | IRLIST);
                let last = arr.subs.len().saturating_sub(1);
                let mut i = 0usize;
                while i < arr.subs.len() {
                    // statement-position runs of `if x == K { slot = V }`
                    // collapse into one table lookup (arms are statements:
                    // no value-position interaction; a kept-last value is
                    // never an IRIF, so the run never swallows it)
                    if !keep_last || i != last {
                        if let Some(consumed) = self.try_const_select_run(&arr.subs, i)? {
                            // the caller already counted sub[i]'s own idx;
                            // skip the rest of the swallowed subtree counts
                            self.web_idx += arr.subs[i..i + consumed]
                                .iter()
                                .map(|n| stmt_walk_count(&**n))
                                .sum::<usize>();
                            i += consumed;
                            continue;
                        }
                    }
                    let n = &arr.subs[i];
                    // P3-PAIRPKG: a recognized pair inside the block — same
                    // sequence-level consumption as the body loop (the
                    // drop check below is bypassed: the fused emission is
                    // vstack-balanced, and plan refuses kept-tail pairs).
                    if let Some(ins) = self.pair_sibling(&arr.subs, i) {
                        self.web_idx += stmt_walk_count(&*arr.subs[i]) + stmt_walk_count(&*arr.subs[i + 1]);
                        self.emit_pair(&*arr.subs[i], ins)?;
                        i += 2;
                        continue;
                    }
                    let k = keep_last && i == last && n.hasretval();
                    self.stmt_one(&**n, k)?;
                    // C11: an eliminated pure dead statement pushed nothing —
                    // there is no value here to drop. (Without the skip this
                    // pop underflowed the vstack: the default arm pops its
                    // own result, so ANY value statement inside a block
                    // rejected the body with a pop-without-push depth error.)
                    if n.hasretval() && !k && !self.is_pure_dead_value(&**n) {
                        let ty = node_res_ty(&**n, self.slot)?;
                        match ty {
                            Ty::Int => {
                                // item 10: a u128-valued statement (e.g. a
                                // dropped `increase(1, xu128)`) occupies a
                                // (lo, hi) PAIR — pop by the value's entry
                                // count, not a blind single
                                for _ in 0..Self::entries_of_width(self.int_width(&**n)) {
                                    self.pop_vreg();
                                }
                            }
                            Ty::Bytes | Ty::Address => {
                                self.pop_vreg();
                                self.pop_vreg();
                            }
                            // defensive: plan() rejects container-valued tails
                            Ty::List | Ty::Map => {}
                        }
                    }
                    i += 1;
                }
                Ok(())
            }
            IRIF => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: if node shape unexpected")?;
                // `if <cmp> { slot = <operand> }` fuses to max/min
                if self.try_maxmin_update(node)? {
                    // the arm + else subtree mirror-counts were consumed
                    self.web_idx += stmt_walk_count(&*n.suby) + stmt_walk_count(&*n.subz);
                    return Ok(());
                }
                // An else arm that emits no bytes makes the then-arm's
                // jump-to-end dead; so does a then arm that cannot fall
                // through (return/throw/break/continue).
                let skip_join =
                    is_zero_byte_stmt(&*n.subz) || ends_in_unconditional_jump(&*n.suby);
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.stmt_one(&*n.suby, false)?;
                if !skip_join {
                    self.asm.jal_(ZERO, l_end);
                }
                self.place(l_else);
                self.stmt_one(&*n.subz, false)?;
                self.place(l_end);
                Ok(())
            }
            IRIFR => {
                // statement-position value-if: branch-lazy like the
                // bytecode channel (compile_if). The caller drops the
                // kept value. CHOOSE (ternary/choose()) no longer lands
                // here: it must evaluate ALL THREE operands eagerly (the
                // bytecode channel has no lazy path for it), so it falls
                // to the eval-and-drop default arm below.
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let l_else = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_else)?;
                self.eval(&*n.suby)?;
                self.asm.jal_(ZERO, l_end);
                self.place(l_else);
                self.eval(&*n.subz)?;
                self.place(l_end);
                Ok(())
            }
            IRWHILE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: while node shape unexpected")?;
                if !self.vstack.is_empty() {
                    return errf!("rv64 lower: while loop in expression position is not supported");
                }
                let l_cond = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.place(l_cond);
                self.branch_cond(&*n.subx, false, l_end)?;
                self.break_labels.push(l_end);
                self.continue_labels.push(l_cond);
                self.stmt_one(&*n.suby, false)?;
                self.break_labels.pop();
                self.continue_labels.pop();
                if !self.vstack.is_empty() {
                    return errf!("rv64 lower: while body leaves values on the stack");
                }
                // D6 (fin-control survey): a body that cannot fall out of
                // its end (break/continue/return/throw/abort tail) makes
                // the backedge jal physically unreachable — skip it. The
                // same `ends_in_unconditional_jump` proof the if-join
                // (skip_join above) and the fallthrough exit already use;
                // pure dead code, zero semantics, one fixup fewer.
                if !ends_in_unconditional_jump(&*n.suby) {
                    self.asm.jal_(ZERO, l_cond);
                }
                self.place(l_end);
                Ok(())
            }
            IRBREAK => match self.break_labels.last() {
                Some(&l) => {
                    self.asm.jal_(ZERO, l);
                    Ok(())
                }
                None => errf!("rv64 lower: break outside a loop"),
            },
            IRCONTINUE => match self.continue_labels.last() {
                Some(&l) => {
                    self.asm.jal_(ZERO, l);
                    Ok(())
                }
                None => errf!("rv64 lower: continue outside a loop"),
            },
            REQ => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: require node shape unexpected")?;
                if self.int_width(&*n.subx) == Some(128) {
                    // T7: the error VALUE is the kernel's 64-bit a0 exit
                    // channel — a u128 error value only rides it when the
                    // constant is statically known to fit u64 (dynamic u128
                    // values cannot serialize faithfully; validate_error_
                    // scalar's other faces cannot fire: a literal is never
                    // Nil and its 16-byte payload is inside every kernel
                    // value_size — SpaceCap::new(0).value_size = 1280).
                    let lit = u128_err_literal(&*n.subx);
                    match lit {
                        Some(v) => {
                            let ok = self.asm.new_label();
                            self.branch_cond(&*n.suby, true, ok)?;
                            self.li_k(A0, v as u64);
                            self.asm.jal_(ZERO, self.trap_label);
                            self.place(ok);
                            return Ok(());
                        }
                        None => {
                            return errf!(
                                "rv64 lower: a u128 error value is outside the translatable \
                                 kernel subset (the kernel's exit channel is 64-bit — use a \
                                 u64 error value or a u128 constant that fits u64)"
                            );
                        }
                    }
                }
                let mev = self.nil_val(&*n.subx);
                self.eval_int(&*n.subx)?; // error value
                let ok = self.asm.new_label();
                self.branch_cond(&*n.suby, true, ok)?;
                let ev = self.pop_reg();
                self.asm.mv(A0, ev);
                self.asm.jal_(ZERO, self.trap_label);
                self.place(ok);
                // validate_error_scalar runs on the interpreter AFTER the
                // condition: a Nil error value faults CastBeValueFail(76)
                // whether the condition passes or not. The value's tag sits
                // at the entry depth (it was popped; the tag byte stays).
                if mev {
                    self.nil_guard_tag(self.vstack.len(), 76);
                }
                Ok(())
            }
            AST => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: assert node shape unexpected")?;
                let ok = self.asm.new_label();
                self.branch_cond(&*n.subx, true, ok)?;
                self.trap(1);
                self.place(ok);
                Ok(())
            }
            ERR => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: throw node shape unexpected")?;
                if self.int_width(&*n.subx) == Some(128) {
                    // T7 (same channel argument as REQ): only a u128
                    // constant fitting u64 can ride the 64-bit exit.
                    let lit = u128_err_literal(&*n.subx);
                    match lit {
                        Some(v) => {
                            self.li_k(A0, v as u64);
                            self.asm.jal_(ZERO, self.trap_label);
                            return Ok(());
                        }
                        None => {
                            return errf!(
                                "rv64 lower: a u128 error value is outside the translatable \
                                 kernel subset (the kernel's exit channel is 64-bit — use a \
                                 u64 error value or a u128 constant that fits u64)"
                            );
                        }
                    }
                }
                let mev = self.nil_val(&*n.subx);
                self.eval_int(&*n.subx)?;
                if mev {
                    // throw <nil> → validate_error_scalar → 76
                    self.nil_guard_tag(self.vstack.len() - 1, 76);
                }
                let ev = self.pop_reg();
                self.asm.mv(A0, ev);
                self.asm.jal_(ZERO, self.trap_label);
                Ok(())
            }
            ABT => {
                self.trap(1);
                Ok(())
            }
            RET => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: return node shape unexpected")?;
                self.eval_bytes(&*n.subx)?;
                self.emit_return()?;
                Ok(())
            }
            END => {
                self.asm.addi(A0, ZERO, 0);
                self.asm.ret();
                Ok(())
            }
            NOP => Ok(()),
            // The surface compiler opens the local window and balances the
            // call frame with these; the proto pre-allocates a typed frame.
            ALLOC => Ok(()),
            POP => {
                if !self.vstack.is_empty() {
                    self.pop_vreg();
                }
                Ok(())
            }
            /* ---- P3-V2IMPL: the list mutation statements ---- */
            APPEND => {
                if keep {
                    return errf!(
                        "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                         (the append result is a list value)"
                    );
                }
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: append node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_append(s, &*n.suby)
            }
            CLEAR => {
                if keep {
                    return errf!(
                        "rv64 lower: opcode CLEAR is outside the translatable kernel subset \
                         (the clear result is a list value)"
                    );
                }
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: clear node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_clear(s)
            }
            /* ---- task 7: the single admitted constant heap_grow ---- */
            HGROW => {
                // plan admits exactly one constant HGROW (heap_plan); its
                // segment count reserved the frame block, so the emission
                // is the length flip. seg == 0 keeps the interpreter's
                // runtime HeapError face: the trap fires where execution
                // reaches the grow (the frame reservation is empty then).
                let seg = self.heap_seg.unwrap_or_else(|| {
                    panic!("rv64 lower: HGROW statement without a heap plan");
                });
                if seg < 1 {
                    self.trap(44); // HeapError
                    return Ok(());
                }
                // length 0 → seg × SEGLEN: publish the grown flag (the
                // block itself was zero-filled in the prologue)
                let sc = self.asm.sc1;
                self.asm.addi(sc, ZERO, 1);
                self.asm.sd(sc, S0, self.heap_flag_off);
                Ok(())
            }
            /* ---- B2 task 5: the in-place element store ---- */
            ITEMSET => {
                if keep {
                    return errf!(
                        "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                         (the item_set result is a container value)"
                    );
                }
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: itemset node shape unexpected")?;
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    return self.emit_map_itemset_stmt(n);
                }
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_itemset_write(&*n.suby, &*n.subz, s)
            }
            // T16/T17: the map and list mutation statements (the receiver
            // is the dropped value; nothing is pushed).
            INSERT => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: insert node shape unexpected")?;
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    if keep {
                        return errf!(
                            "rv64 lower: opcode INSERT is outside the translatable kernel \
                             subset (the insert result is a map value)"
                        );
                    }
                    return self.emit_map_insert_stmt(n);
                }
                if keep {
                    return errf!(
                        "rv64 lower: opcode INSERT is outside the translatable kernel subset \
                         (the insert result is a list value)"
                    );
                }
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_insert(s, &*n.suby, &*n.subz)
            }
            REMOVE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: remove node shape unexpected")?;
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    if keep {
                        return errf!(
                            "rv64 lower: opcode REMOVE is outside the translatable kernel \
                             subset (the remove result is a map value)"
                        );
                    }
                    return self.emit_map_remove_stmt(n);
                }
                if keep {
                    return errf!(
                        "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                         (the remove result is a list value)"
                    );
                }
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_remove(s, &*n.suby)
            }
            MERGE => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: merge node shape unexpected")?;
                if is_map_slot(
                    slot_get_index(&*n.subx)
                        .and_then(|i| self.slot.get(i as usize).copied())
                        .unwrap_or(SlotTy::None),
                ) {
                    if keep {
                        return errf!(
                            "rv64 lower: opcode MERGE is outside the translatable kernel \
                             subset (the merge result is a map value)"
                        );
                    }
                    return self.emit_map_merge(n);
                }
                if keep {
                    return errf!(
                        "rv64 lower: opcode MERGE is outside the translatable kernel subset \
                         (the merge result is a list value)"
                    );
                }
                let rs = self.list_slot_of(&*n.subx, op)?;
                let ss = self.list_slot_of(&*n.suby, op)?;
                self.emit_merge(rs, ss)
            }
            _ => {
                // C11 (impl/ctrl-dse): a statement-position value whose whole
                // subtree is fault-free and side-effect-free and whose result
                // is dropped right here — skip the emission entirely. The
                // eager path below pushes then pops the same value, so
                // emitting nothing leaves the identical (empty) vstack state;
                // whitelisted subtrees carry no fault face, so nothing
                // observable disappears.
                if self.is_pure_dead_value(node) {
                    return Ok(());
                }
                // value-position node in statement position: eval and drop
                self.eval(node)?;
                if node.hasretval() {
                    let ty = node_res_ty(node, self.slot)?;
                    match ty {
                        Ty::Int => {
                            // item 10: a u128-valued dropped statement is a
                            // (lo, hi) pair — pop the value's entry count
                            for _ in 0..Self::entries_of_width(self.int_width(node)) {
                                self.pop_vreg();
                            }
                        }
                        Ty::Bytes | Ty::Address => {
                            self.pop_vreg();
                            self.pop_vreg();
                        }
                        // defensive: plan() rejects container-valued statements
                        Ty::List | Ty::Map => {}
                    }
                }
                Ok(())
            }
        }
    }

    /* ---- expressions ---- */

    /// Pop `vals` arm-result locations (LIFO) and return them bottom-first,
    /// with every Ref materialized into ITS OWN consecutive pool position
    /// (the `mv` lands inside the arm's own region). The stack stays at the
    /// entry level: the caller re-pushes the uniform locations once BOTH
    /// arms agree. Re-pushing position-by-position (not push/pop per value)
    /// is what keeps a bytes pair off the same register: with the old
    /// push/pop-per-value form the len materialization re-handed the PTR's
    /// register and clobbered it (BUG-011). A Pool value is already in its
    /// position's register — the arm pushed it there — so only a Ref moves.
    pub(crate) fn take_arm_values(&mut self, vals: usize) -> Vec<VLoc> {
        let mut popped: Vec<VLoc> = (0..vals).map(|_| self.pop_vreg()).collect();
        popped.reverse(); // bottom-first: mirror the push order
        let mut out = Vec::with_capacity(vals);
        for l in popped.drain(..) {
            let t = self.push_vreg(); // hands vregs[entry + i]
            if let VLoc::Ref(r) = l {
                if t != r {
                    self.asm.mv(t, r);
                }
            }
            out.push(VLoc::Pool(t));
        }
        for _ in 0..vals {
            self.pop_vreg();
        }
        out
    }

    /// Store a local slot read onto the vstack (the eval GET arm's body;
    /// T11's constant-index GETX folds onto the same path).
    pub(crate) fn emit_get_slot(&mut self, idx: u8, mnil: bool) -> Ret<()> {
                match self.slot.get(idx as usize) {
                    Some(SlotTy::Input) => {
                        // the kernel's `data: bytes` parameter: (input_base,
                        // len) — the ABI capture registers ARE the slot's
                        // home (read-only for the whole body), so reads are
                        // zero-cost references, exactly like a banked bytes
                        // local
                        self.push_ref(S4);
                        self.push_ref(S5);
                    }
                    // P3-V2IMPL defensive: a bare GET of a list slot never
                    // reaches lowering (plan() rejects every consumer but
                    // item_get/append/clear/length/has_key)
                    Some(SlotTy::List) | Some(SlotTy::ListBytes) => {
                        return errf!(
                            "rv64 lower: opcode GET is outside the translatable kernel subset \
                             (a list value cannot be used as a value)"
                        );
                    }
                    Some(SlotTy::Map(_)) => {
                        return errf!(
                            "rv64 lower: opcode GET is outside the translatable kernel subset \
                             (a map value cannot be used as a value)"
                        );
                    }
                    _ if self.bank_bytes(idx).is_some() => {
                        // promoted bytes local: two zero-cost references
                        let (rp, rl) = self.bank_bytes(idx).unwrap();
                        self.push_ref(rp);
                        self.push_ref(rl);
                        if mnil {
                            self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                        }
                    }
                    _ if self.bank_int_read(idx).is_some() => {
                        // promoted int local: zero-cost reference
                        self.push_ref(self.bank_int_read(idx).unwrap());
                        if mnil {
                            self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                        }
                    }
                    // T13 stain fix: an Address slot's runtime cell is the
                    // SAME (ptr, len) pair shape as a bytes slot — one read
                    // path (the stain lives only in the static type model,
                    // consumed by node_res_ty → TIS/TID)
                    Some(SlotTy::Bytes) | Some(SlotTy::Address) => {
                        // Forwarding soundness: a Ref may only ever name the
                        // BOTTOM pool register pair, pushed at an empty stack
                        // (position 0). Any later push hands vregs[2..] while
                        // the refs live, so nothing can re-hand — and then
                        // clobber — the watched registers mid-expression.
                        // (The int arm below carries the E2 generalization
                        // `r == vregs[j], j <= depth`; a bytes version must
                        // re-issue BOTH registers of the pair and is not
                        // attempted here.)
                        let hit = self
                            .fwd_enabled()
                            .then(|| {
                                if self.fwd_slot_ok(idx as usize) {
                                    self.fwd_bytes.get(idx as usize).copied().flatten()
                                } else {
                                    None
                                }
                            })
                            .flatten();
                        match hit {
                            Some((rp, rl))
                                if self.vstack.is_empty()
                                    && rp == self.vregs[0]
                                    && self.vregs.len() > 1
                                    && rl == self.vregs[1] =>
                            {
                                self.push_ref(rp);
                                self.push_ref(rl);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                            _ => {
                                let off = self.slot_offset(idx);
                                let rp = self.push_vreg();
                                let rl = self.push_vreg();
                                self.asm.ld(rp, S0, off);
                                self.asm.ld(rl, S0, off + 8);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                        }
                    }
                    _ if self.slot_w.get(idx as usize).copied().flatten() == Some(128) => {
                        // task 3: a u128 slot reads as a (lo, hi) pair — two
                        // vstack entries, low half first; the tag byte lives
                        // on the LOW entry.
                        // C7 (impl/cat-u128fwd): pair store→load forwarding.
                        // The entry rides the fwd_bytes PAIR table: a u128
                        // slot is SlotTy::Int (join_slot_widths only records
                        // widths for Int slots) and is never banked (web.rs),
                        // so no other producer can ever write this entry.
                        // Hit discipline is the bytes arm's: empty stack,
                        // the forwarded registers ARE the bottom pair, and
                        // any push_vreg re-hand (kill_fwd_reg) or place()
                        // label barrier kills the entry wholesale. The
                        // may-Nil tag byte is independent of forwarding —
                        // nil_wtag_from_flag reads the slot's flag byte and
                        // writes the tag through the dedicated scratch, no
                        // pool register traffic.
                        let hit = self
                            .fwd_enabled()
                            .then(|| {
                                if self.fwd_slot_ok(idx as usize) {
                                    self.fwd_bytes.get(idx as usize).copied().flatten()
                                } else {
                                    None
                                }
                            })
                            .flatten();
                        match hit {
                            Some((rp, rl))
                                if self.vstack.is_empty()
                                    && rp == self.vregs[0]
                                    && self.vregs.len() > 1
                                    && rl == self.vregs[1] =>
                            {
                                self.push_ref(rp);
                                self.push_ref(rl);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                            _ => {
                                let off = self.slot_offset(idx);
                                let lo = self.push_vreg();
                                let hi = self.push_vreg();
                                self.asm.ld(lo, S0, off);
                                self.asm.ld(hi, S0, off + 8);
                                if mnil {
                                    self.nil_wtag_from_flag(idx, self.vstack.len() - 2);
                                }
                            }
                        }
                    }
                    _ => match self.fwd_int.get(idx as usize).copied().flatten() {
                        // E2: the read-side rule generalized off the empty
                        // stack — `r == vregs[j]` with `j <= vstack.len()`
                        // is the sound re-issue condition (the pushed Ref
                        // sits at position vstack.len(), so later pushes
                        // hand only registers ABOVE it; see
                        // fwd_reg_reissuable for the full argument and the
                        // push_vreg INV_CHECK mirror). Banked reads and the
                        // bytes/u128 arms above keep their own rules.
                        Some(r)
                            if self.fwd_enabled()
                                && self.fwd_slot_ok(idx as usize)
                                && self.fwd_reg_reissuable(r) =>
                        {
                            self.push_ref(r);
                            if mnil {
                                self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                            }
                        }
                        _ => {
                            let off = self.slot_offset(idx);
                            let r = self.push_vreg();
                            self.asm.ld(r, S0, off);
                            if mnil {
                                self.nil_wtag_from_flag(idx, self.vstack.len() - 1);
                            }
                        }
                    },
                }
        Ok(())
    }


    /// Store a value into local slot `idx` (the eval PUT arm's body; T11's
    /// constant-index PUTX folds onto the same path — never a prelude).
    pub(crate) fn emit_put_slot(&mut self, idx: u8, src: &dyn IRNode, prelude_ok: bool) -> Ret<()> {
                if prelude_ok && self.input_local && idx == 0 && !self.param_bound {
                    // T15: the prelude of a body that rebinds `data` — the
                    // param binding becomes an entry copy: the ABI input
                    // bytes move into the arena and slot 0's home takes the
                    // copy's (ptr, len). Reads before the first user store
                    // see the input exactly like the interpreter; the caller
                    // and the input registers are never touched.
                    self.param_bound = true;
                    self.emit_input_local_copy()?;
                    return Ok(());
                }
                if matches!(self.slot.get(idx as usize), Some(SlotTy::Input)) {
                    if prelude_ok && !self.param_bound {
                        // prelude binding: the value is stack machinery
                        // (ROLL0); the input registers already hold it.
                        self.param_bound = true;
                        return Ok(());
                    }
                    // GT-c: asserted core substring "input parameter cannot be assigned"
                        // stays contiguous (rv64_gen_corpus.rs P2 message pin);
                        // the hint appends after it
                        return errf!(
                            "rv64 lower: the input parameter cannot be assigned; the call-data \
                             argument is read-only — copy it into a local first \
                             (e.g. `var t = data`) and assign to that"
                        );
                }
                // T16: map binding — creation (new_map / map{..}, routed
                // here by the statement form) or a descriptor-word alias.
                if is_map_slot(self.slot.get(idx as usize).copied().unwrap_or(SlotTy::None)) {
                    if map_create_src(src).is_some() {
                        return self.emit_map_create(idx, src);
                    }
                    if let Some(si) = slot_get_index(src)
                        .filter(|&i| is_map_slot(self.slot.get(i as usize).copied().unwrap_or(SlotTy::None)))
                    {
                        // B14: the object now has TWO slot names. The fact is
                        // per-slot about the OBJECT's length, and an insert
                        // through either name changes it — no len-zero fact
                        // may survive an aliasing PUT (the destination never
                        // gets one, the source's dies).
                        self.map_zero_kill_all();
                        let sc1 = self.asm.sc1;
                        let off = self.slot_offset(idx);
                        self.ctr_load_obj(si as usize, sc1);
                        self.asm.sd(sc1, S0, off);
                        return Ok(());
                    }
                    return errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (internal: map slot {idx} bound from a non-map expression)"
                    );
                }
                // T16: `keys(m)` / `values(m)` snapshot bound to a list slot.
                // A LIST receiver is NOT a map snapshot: it falls through to
                // the T17 list_bind_src Trap83 form (the runtime 83 face).
                if matches!(
                    self.slot.get(idx as usize),
                    Some(SlotTy::List) | Some(SlotTy::ListBytes)
                ) && matches!(
                    Bytecode::try_from_u8(src.bytecode()),
                    Ok(Bytecode::KEYS) | Ok(Bytecode::VALUES)
                ) && src
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .and_then(|sn| slot_get_index(&*sn.subx))
                    .map(|i| {
                        is_map_slot(self.slot.get(i as usize).copied().unwrap_or(SlotTy::None))
                    })
                    .unwrap_or(false)
                {
                    return self.emit_map_to_list(idx, src);
                }
                // P3-V2IMPL: the (single) list-creation PUT fills the frame
                // block / registers the rodata table. T17 (§3 arena
                // container model) routes every container-VALUE source
                // through `list_bind_src`: alias PUT (descriptor copy —
                // one shared object), CLONE-bind (the deep copy),
                // receiver-yielding mutation results, a CHOOSE select and
                // the always-faulting KEYS/VALUES(list).
                if matches!(self.slot.get(idx as usize), Some(SlotTy::List)) {
                    if let Some(bind) = list_bind_src(src, self.slot) {
                        return self.emit_put_list_bind(idx, src, bind);
                    }
                    return self.emit_list_create(idx, src);
                }
                let ty = node_res_ty(src, self.slot)?;
                let msrc = self.nil_val(src);
                let stracked =
                    self.tracked.get(idx as usize).copied().unwrap_or(false);
                // T13: an Address value stores as its 21 content bytes —
                // the same two-slot store (an Address is never Nil)
                if matches!(ty, Ty::Bytes | Ty::Address) {
                    let (bp, bl) = match self.bank_bytes_write(idx) {
                        Some(x) => x,
                        None => {
                            let off = self.slot_offset(idx);
                            self.eval_bytes(src)?;
                            // the source's tag sits under its len half
                            let tag_d = self.vstack.len() - 2;
                            let rl = self.pop_reg();
                            let rp = self.pop_reg();
                            let s = idx as usize;
                            if self.store_is_dead(idx) {
                                self.fwd_bytes[s] = None;
                            } else {
                                self.asm.sd(rp, S0, off);
                                self.asm.sd(rl, S0, off + 8);
                                self.fwd_bytes[s] = Some((rp, rl));
                                if stracked {
                                    if msrc {
                                        self.nil_set_flag_from_tag(idx, tag_d);
                                    } else {
                                        self.nil_set_flag_one(idx);
                                    }
                                }
                            }
                            return Ok(());
                        }
                    };
                    // literals bind straight into the bank registers
                    if let Some(b) = bytes_literal(src) {
                        if self.store_is_dead(idx) {
                            // C13: a dead literal bind has no fault face at
                            // all — the pool registration, the descriptor
                            // materialization and the init flag all die with
                            // the store; the kill_fwd_reg bookkeeping stays
                            // (loop-carried / backedge exemptions live inside
                            // store_is_dead).
                            self.kill_fwd_reg(bp);
                            self.kill_fwd_reg(bl);
                            return Ok(());
                        }
                        let lbl = self.lit_label(&b);
                        if self.desc_fwd.get(&lbl).copied() == Some((bp, bl)) {
                            // C12 (impl/cse-dse-bank): the same pool label is
                            // already materialized into these very bank homes
                            // — nothing is emitted and nothing is rewritten,
                            // so the register-watching tables (fwd/desc/cat)
                            // stay valid and the kill discipline is skipped
                            // with the emissions.
                        } else {
                            self.kill_fwd_reg(bp);
                            self.kill_fwd_reg(bl);
                            self.lit_addr(bp, lbl);
                            self.asm.li(bl, b.len() as u64);
                            self.desc_fwd.insert(lbl, (bp, bl));
                        }
                        if stracked {
                            self.nil_set_flag_one(idx);
                        }
                        return Ok(());
                    }
                    self.eval_bytes(src)?;
                    let tag_d = self.vstack.len() - 2;
                    let ll = self.pop_reg();
                    let lp = self.pop_reg();
                    // B9: the stored value may be the arena-top descriptor —
                    // after it moves under the bank home (kill_fwd_reg above
                    // may legitimately kill the fact when the homes coincide
                    // with the watched registers) the claim transfers to the
                    // new home pair: same VALUES, so ptr + len == S1 holds
                    // for (bp, bl) exactly as it held for (lp, ll).
                    let was_top = self.cat_top == Some((lp, ll));
                    self.kill_fwd_reg(bp);
                    self.kill_fwd_reg(bl);
                    if self.store_is_dead(idx) {
                        // C13: the rhs ran in full (its fault faces are the
                        // interpreter's); the bank-home mv pair and the init
                        // flag are dead weight after the last read. The
                        // arena-top fact stays as it is: (lp, ll) still hold
                        // the descriptor (nothing was rewritten) and the
                        // kill_fwd_reg calls above behaved exactly as on the
                        // live path.
                    } else {
                        if lp != bp {
                            self.asm.mv(bp, lp);
                        }
                        if ll != bl {
                            self.asm.mv(bl, ll);
                        }
                        if was_top {
                            self.cat_top_establish(bp, bl);
                        }
                        if stracked {
                            if msrc {
                                self.nil_set_flag_from_tag(idx, tag_d);
                            } else {
                                self.nil_set_flag_one(idx);
                            }
                        }
                    }
                } else if let Some(r) = self.bank_int(idx) {
                    // plain int literals (no cast wrapper — the cast arm
                    // keeps its range checks) load directly into the bank reg
                    if is_plain_int_literal(src) {
                        let k = int_literal(src).unwrap();
                        self.kill_fwd_reg(r);
                        if self.store_is_dead(idx) {
                            // C13 (impl/cse-dse-bank): the timeline proves no
                            // read of this slot exists after this statement —
                            // the literal carries no fault face of its own, so
                            // the bank write, the width shadow and the init
                            // flag are all dead weight. Register bookkeeping
                            // (kill_fwd_reg above) is kept, and the
                            // loop-carried / backedge exemptions live inside
                            // store_is_dead (BUG-022 discipline, shared with
                            // the frame arm below).
                            return Ok(());
                        }
                        self.li_k(r, k);
                        // narrow-PUT face: the assignment replays its own width
                        self.shadow_put(idx, self.int_width(src));
                        if stracked {
                            self.nil_set_flag_one(idx);
                        }
                        return Ok(());
                    }
                    self.eval_int(src)?;
                    let tag_d = self.vstack.len() - 1;
                    let v = self.pop_reg();
                    self.kill_fwd_reg(r);
                    if self.store_is_dead(idx) {
                        // C13: the rhs evaluation above ran in full (its fault
                        // faces — convert/overflow — are the interpreter's);
                        // only the bank-home write and its shadow/flag
                        // auxiliary machine die with the proven-dead store.
                        // Same basis as the frame arm below; the loop-carried
                        // and backedge exemptions stand (store_is_dead).
                    } else {
                        if v != r {
                            self.asm.mv(r, v);
                        }
                        self.shadow_put(idx, self.int_width(src));
                        if stracked {
                            if msrc {
                                self.nil_set_flag_from_tag(idx, tag_d);
                            } else {
                                self.nil_set_flag_one(idx);
                            }
                        }
                    }
                } else if int_literal(src) == Some(0)
                    && self.zero_init_skip.get(idx as usize).copied().flatten()
                        == Some(self.web_idx)
                    && !stracked
                    // narrow-PUT face: the elided store must still narrow the
                    // width shadow (`var x = 0` makes x a u8 on the
                    // interpreter); a flagged slot takes the full store path
                    && !self.dyn_w_flag(idx)
                {
                    // `var x = 0` elided into the prologue frame zero (the
                    // timeline proved this store is the slot's first event
                    // and no backedge can resurrect a stale value). A
                    // TRACKED slot cannot take this path: its init flag
                    // must be set exactly when the store executes.
                    self.fwd_int[idx as usize] = None;
                    return Ok(());
                } else {
                    let off = self.slot_offset(idx);
                    let wide = self.slot_w.get(idx as usize).copied().flatten() == Some(128);
                    self.eval_int(src)?;
                    if wide {
                        // task 3: a u128 slot stores the (lo, hi) pair into
                        // its 16-byte cell.
                        // C7 (impl/cat-u128fwd): the pair forwards through
                        // the fwd_bytes pair table (see the GET arm). PUT is
                        // statement-level, so the source pair was pushed at
                        // an empty stack and lo/hi ARE the bottom pool pair
                        // (vregs[0]/vregs[1]) — exactly the shape the hit
                        // rule demands; a dead store forwards nothing (no
                        // reader exists, matching the bytes arm). Any later
                        // push_vreg re-hand or place() label kills the entry
                        // via the shared kill_fwd_reg/place discipline, so a
                        // following u128 XOP (which stages its old pair with
                        // push_vreg) clears it before it could go stale.
                        let tag_d = self.vstack.len() - 2;
                        let hi = self.pop_reg();
                        let lo = self.pop_reg();
                        let s = idx as usize;
                        if !self.store_is_dead(idx) {
                            self.asm.sd(lo, S0, off);
                            self.asm.sd(hi, S0, off + 8);
                            self.fwd_bytes[s] = Some((lo, hi));
                        } else {
                            self.fwd_bytes[s] = None;
                        }
                        self.fwd_int[s] = None;
                        if stracked {
                            if msrc {
                                self.nil_set_flag_from_tag(idx, tag_d);
                            } else {
                                self.nil_set_flag_one(idx);
                            }
                        }
                        return Ok(());
                    }
                    let tag_d = self.vstack.len() - 1;
                    let v = self.pop_reg();
                    let s = idx as usize;
                    if self.store_is_dead(idx) {
                        // dead store: the value is never read again on any
                        // path (the web timeline proves it); the expression
                        // (and its faults) still evaluate in full
                        self.fwd_int[s] = None;
                    } else {
                        self.asm.sd(v, S0, off);
                        self.fwd_int[s] = Some(v);
                        // narrow-PUT face: the assignment replays its own width
                        self.shadow_put(idx, self.int_width(src));
                        if stracked {
                            if msrc {
                                self.nil_set_flag_from_tag(idx, tag_d);
                            } else {
                                self.nil_set_flag_one(idx);
                            }
                        }
                    }
                }
        Ok(())
    }

    /// C11 (impl/ctrl-dse): is this statement-position value node's whole
    /// subtree fault-free, side-effect-free, and emitted purely to be
    /// dropped? The whitelist is deliberately conservative — everything
    /// with an evaluation face stays out:
    ///   - plain literals (P0..P3/PU8/PU16/PTRUE/PFALSE and PNIL — the nil
    ///     literal's tag write is dropped too, which is safe because a tag
    ///     byte is only ever READ for a value whose own lowering publishes
    ///     it at the same depth first: a may-Nil GET's `nil_wtag_from_flag`,
    ///     a PNIL itself, a CHOOSE select's `nil_set_tag_one`; the next
    ///     occupant of this vstack depth republishes before any consumer
    ///     looks),
    ///   - compile-time folded casts (eval lowers them to a single `li_k`;
    ///     an overflowing literal keeps the eager path's compile reject),
    ///   - SIZE over a bytes literal (the static fold; dynamic SIZE keeps
    ///     its BytesHandle(91) face and is NOT admitted),
    ///   - LENGTH over a static list class (Empty/Rodata/FrameStatic — the
    ///     Growable/Arena header reads stay out),
    ///   - GET of a non-tracked slot (a fault-free frame read; mirrors
    ///     `is_pure_cond`'s slot rule) — what makes the TNIL-family
    ///     constant answers reachable in practice,
    ///   - type tests with a provably constant answer (TNIL/TIS/TID with a
    ///     non-Nil operand, TLIST/TMAP always — the kernel has no Compo
    ///     values); a may-Nil operand keeps the tag-read path.
    /// HASKEY (the 72 face) is NOT admitted; anything dynamic is NOT
    /// admitted. On any shape or type doubt this returns false and the
    /// eager path lowers the node exactly as before.
    /// Fold a paren-group-wrapped literal cast chain to its u128 value,
    /// mirroring `ir_literal_value`'s CU* cast rules (out-of-range → None:
    /// the eager path's compile-time reject stays reachable). Needed because
    /// `ir_literal_value` itself does not see through IRNodeWrapOne.
    fn dead_cast_value(&self, node: &dyn IRNode) -> Option<u128> {
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.dead_cast_value(&*w.node);
        }
        if let Ok(Some(v)) = crate::lang::ir_literal_value(node) {
            return v.extract_u128().ok().filter(|n| *n <= u64::MAX as u128);
        }
        let single = node.as_any().downcast_ref::<IRNodeSingle>()?;
        let v = self.dead_cast_value(&*single.subx)?;
        let limit: u128 = match Bytecode::try_from_u8(node.bytecode()).ok()? {
            Bytecode::CU8 => 0xFF,
            Bytecode::CU16 => 0xFFFF,
            Bytecode::CU32 => 0xFFFF_FFFF,
            Bytecode::CU64 => u64::MAX as u128,
            _ => return None,
        };
        if v > limit {
            return None;
        }
        Some(v)
    }

    pub(crate) fn is_pure_dead_value(&self, node: &dyn IRNode) -> bool {        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return self.is_pure_dead_value(&*w.node);
        }
        let op = match Bytecode::try_from_u8(node.bytecode()) {
            Ok(o) => o,
            Err(_) => return false,
        };
        use Bytecode::*;
        match op {
            P0 | P1 | P2 | P3 | PU8 | PU16 | PTRUE | PFALSE | PNIL => true,
            GET | GET0 | GET1 | GET2 | GET3 | GETX => match slot_get_index(node) {
                Some(i) => !self.tracked.get(i as usize).copied().unwrap_or(false),
                None => false,
            },
            CU8 | CU16 | CU32 | CU64 => {
                // "already folded cast": the whole (paren-group-wrapped)
                // literal cast chain folds into the target's range — eval
                // lowers it to a single li_k. Anything else (bytes operand,
                // overflow) keeps its eager faces and is not admitted.
                self.dead_cast_value(node).is_some()
            }
            SIZE => {
                let sub = match node.as_any().downcast_ref::<IRNodeSingle>() {
                    Some(n) => &*n.subx,
                    None => return false,
                };
                bytes_literal(sub).is_some()
            }
            LENGTH => {
                let sub = match node.as_any().downcast_ref::<IRNodeSingle>() {
                    Some(n) => &*n.subx,
                    None => return false,
                };
                // the map-header path (emit_map_length) stays out
                if is_map_slot(slot_get_index(sub).and_then(|i| self.slot.get(i as usize).copied()).unwrap_or(SlotTy::None)) {
                    return false;
                }
                match self.list_any_slot_of(sub, op) {
                    Ok(s) => matches!(
                        self.lists.kind.get(s),
                        Some(ListKind::Empty)
                            | Some(ListKind::Rodata { .. })
                            | Some(ListKind::FrameStatic { .. })
                    ),
                    Err(_) => false,
                }
            }
            TNIL | TIS | TID | TLIST | TMAP => {
                // TIS with an unknown type id keeps the eager path's
                // fail-fast; a container/static-type-doubt operand too.
                let (sub, para) = if op == TIS {
                    match node.as_any().downcast_ref::<IRNodeParam1Single>() {
                        Some(n) => (&*n.subx, n.para),
                        None => return false,
                    }
                } else {
                    match node.as_any().downcast_ref::<IRNodeSingle>() {
                        Some(n) => (&*n.subx, 0u8),
                        None => return false,
                    }
                };
                if op == TIS && crate::value::parse_value_ty_param(para).is_err() {
                    return false;
                }
                let ty = match node_res_ty(sub, self.slot) {
                    Ok(t) => t,
                    Err(_) => return false,
                };
                if matches!(ty, Ty::List | Ty::Map) {
                    return false; // eval fail-fasts these; keep that path
                }
                let sid_known = match ty {
                    Ty::Bytes | Ty::Address => true,
                    Ty::Int => matches!(
                        self.int_width(sub),
                        Some(1) | Some(8) | Some(16) | Some(32) | Some(64) | Some(128)
                    ),
                    _ => false,
                };
                if !sid_known {
                    return false;
                }
                // TLIST/TMAP answer false unconditionally (no kernel value is
                // a Compo, even may-Nil); the identity tests need a
                // provably non-Nil operand for the constant answer.
                (matches!(op, TLIST | TMAP) || !self.nil_val(sub))
                    && self.is_pure_dead_value(sub)
            }
            _ => false,
        }
    }
}
