use super::super::*;

impl<'a> Lower<'a> {

    /* ---- P3-V2IMPL: compo-list v2 shape lowering ---------------------- *
     * plan() is the single admit/reject source: every helper below only   *
     * runs on shapes the plan walk already validated (receiver = GET of a *
     * List slot; index/value expressions are u64-typed). Trap codes and   *
     * their ORDER mirror the interpreter exactly:                        *
     *   - itemget: extract_u32 first — key Nil → 72, key ≥ 2³² → 72       *
     *     (value/convert.rs), then idx ≥ len → CompoNoFindItem(85)        *
     *     (value/list.rs:36-40);                                         *
     *   - append: value Nil → CastBeValueFail(76) (check_scalar), then    *
     *     len ≥ 128 → OutOfCompoLen(40) (compo.rs:160-172);              *
     *   - packlist creation: element expressions fault in IR order; the  *
     *     deferred check_scalar pass cannot fire (may-Nil elements are   *
     *     compile-rejected, the B-family fail-fast precedent);           *
     *   - length/haskey fold (u64 width — the pinned D5/D7 family).      */

    /// The list slot index of an admitted consumer's receiver (defensive
    /// re-validation; plan() already guaranteed the shape).
    pub(crate) fn list_slot_of(&self, node: &dyn IRNode, op: Bytecode) -> Ret<usize> {
        list_receiver_slot(node, self.slot, op)
    }

    /// The bytes slot index of a BUFSET receiver (defensive re-validation).
    pub(crate) fn list_slot_of_bytes(&self, node: &dyn IRNode, op: Bytecode) -> Ret<usize> {
        bytes_receiver_slot(node, self.slot, op)
    }

    /// T16: the receiver slot of a read-only list consumer — admits the
    /// uint-element class and the map-produced Bytes class.
    pub(crate) fn list_any_slot_of(&self, node: &dyn IRNode, op: Bytecode) -> Ret<usize> {
        list_any_receiver_slot(node, self.slot, op)
    }

    /// T16: `keys(m)[k]` / `values(m)[k]` — a Bytes-element arena list.
    /// Element cells are (tag=8, ptr, len); the result is a bytes pair.
    pub(crate) fn emit_list_bytes_itemget(&mut self, index: &dyn IRNode, s: usize) -> Ret<()> {
        let cell = self.slot_offset(s as u8);
        let lit = crate::lang::ir_literal_value(index)
            .ok()
            .flatten()
            .and_then(|v| v.extract_u128().ok());
        if let Some(k) = lit {
            if k >= 1u128 << 32 {
                return errf!(
                    "rv64 lower: ITEMGET constant index {k} exceeds the u32 index space"
                );
            }
            let l85 = self.nil_thunk(85);
            let (sc1, sc2, a0) = (self.asm.sc1, self.asm.sc2, self.a0);
            self.asm.li(sc2, k as u64);
            self.asm.ld(sc1, S0, cell); // obj
            self.asm.ld(a0, sc1, 0); // len
            self.asm.bgeu(sc2, a0, l85);
            let tmp = self.push_vreg();
            self.ctr_elem_addr(a0, sc1, sc2, tmp);
            self.pop_vreg();
            let ptr = self.push_vreg();
            let len = self.push_vreg();
            self.asm.ld(ptr, a0, 8);
            self.asm.ld(len, a0, 16);
            return Ok(());
        }
        // variable index: Nil → 72, ≥ 2³² → 72, ≥ len → 85
        self.eval_int(index)?;
        let kd = self.vstack.len() - 1;
        if self.nil_val(index) {
            self.nil_guard_tag(kd, 72);
        }
        let kr = loc_reg(self.vstack[kd]);
        match self.lists.kind.get(s) {
            Some(ListKind::Arena { .. }) => self.emit_list_bounds_guards_arena(kr, s)?,
            _ => {
                return errf!(
                    "rv64 lower: opcode ITEMGET is outside the translatable kernel subset \
                     (internal: bytes-element list without an Arena class)"
                )
            }
        }
        self.pop_vreg(); // index
        let (sc1, a0) = (self.asm.sc1, self.a0);
        self.asm.ld(sc1, S0, cell); // obj
        let tmp = self.push_vreg();
        self.ctr_elem_addr(a0, sc1, kr, tmp);
        self.pop_vreg();
        let ptr = self.push_vreg();
        let len = self.push_vreg();
        self.asm.ld(ptr, a0, 8);
        self.asm.ld(len, a0, 16);
        Ok(())
    }

    /* ---- B2 task 5: ITEMSET lowering ----------------------------------- *
     * plan() is the single admit/reject source: the receiver is a local    *
     * list slot, the index and value are u64-typed, the value is           *
     * definite (may-Nil rejected — BUG-029). The interpreter's 83 (non-    *
     * compo) / 80 (map) receiver faces are structurally absent on a local  *
     * list slot; 76 (value check_scalar) cannot fire on a definite uint.   *
     * Runtime faces, in interpreter order: extract_u32 → 72 (Nil index     *
     * via the tag guard, ≥ 2³²), then idx ≥ len (incl. ==) → 85, then the  *
     * store (no shift, no length change).                                  */

    /// The post-evaluation guards of an in-place element write: the
    /// extract_u32 boundary (≥ 2³² → 72; the Nil-index tag guard is emitted
    /// by the caller at the same fault position) and the live-length bound
    /// (idx ≥ len → 85).
    pub(crate) fn itemset_bounds_guards(&mut self, s: usize, kr: u8) -> Ret<()> {
        let l72 = self.nil_thunk(72);
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(kr, sc1, l72);
        let l85 = self.nil_thunk(85);
        let cell = self.slot_offset(s as u8);
        self.asm.ld(sc1, S0, cell); // live len
        self.asm.bgeu(kr, sc1, l85);
        Ok(())
    }

    /// `l[i] = v` (the plain ITEMSET statement): index and value evaluate
    /// first (interpreter stack order: container, index, value), then the
    /// 72/85 guards, then the in-place store. The container result is not
    /// pushed — the statement block pops nothing for a list-valued node.
    pub(crate) fn emit_itemset_write(
        &mut self,
        index: &dyn IRNode,
        value: &dyn IRNode,
        s: usize,
    ) -> Ret<()> {
        let arena_kind = matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. }));
        if !arena_kind && !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
            return errf!(
                "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                 (internal: item_set on a non-growable storage class)"
            );
        }
        // BUG-029 family: a may-Nil stored value is compile-REJECTED —
        // check_scalar ADMITS Nil, the interpreter silently stores the Nil
        // element and only a later read of the hole faults; the u64 element
        // block cannot represent Nil (may-Nil APPEND value precedent).
        if self.nil_val(value) {
            return errf!(
                "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                 (the stored value may be Nil: the interpreter stores a Nil element \
                 and faults only when that element is later read, which a u64 frame \
                 block cannot represent — compute the value from definite assignments)"
            );
        }
        // interpreter order: index subtree, then value subtree, then the
        // instruction's own checks. T17: a list VALUE is item_set's
        // check_scalar 76 face (it runs before any index check).
        self.eval_int(index)?;
        if matches!(node_res_ty(value, self.slot)?, Ty::List) {
            for _ in 0..Self::entries_of_width(self.int_width(index)) {
                self.pop_vreg();
            }
            self.trap(76);
            return Ok(());
        }
        let kd = self.vstack.len() - 1;
        self.eval_int(value)?;
        // T19: a u128 element value is a (lo, hi) pair — two entries (the
        // width replay guarantees the value width equals the slot's)
        let vent = Self::entries_of_width(self.int_width(value));
        debug_assert_eq!(vent, if self.list_step(s) == 16 { 2 } else { 1 });
        let vd = self.vstack.len() - 1;
        if self.nil_val(index) {
            self.nil_guard_tag(kd, 72); // extract_u32(Nil) → 72
        }
        let kr = loc_reg(self.vstack[kd]);
        // P2: the arena item_set — the same 72/85 guard pair with the
        // length read through the descriptor, then the in-place
        // tagged-cell store (no shift, no length change; visible through
        // every alias)
        if arena_kind {
            let sc1 = self.asm.sc1;
            let l72 = self.nil_thunk(72);
            self.asm.li(sc1, 1u64 << 32);
            self.asm.bgeu(kr, sc1, l72);
            let l85 = self.nil_thunk(85);
            let cell = self.slot_offset(s as u8);
            self.asm.ld(sc1, S0, cell); // obj
            self.asm.ld(sc1, sc1, 0); // len
            self.asm.bgeu(kr, sc1, l85);
            let a1 = self.a1;
            let ew = self.lists.ew.get(s).copied().unwrap_or(64);
            // obj again (sc1 held len) — into sc1, NOT sc2: ImcRemap maps
            // a1 and sc2 to the same x31, and ctr_elem_addr needs
            // dst(obj), obj and tmp pairwise distinct
            self.asm.ld(sc1, S0, cell);
            self.ctr_elem_addr(a1, sc1, kr, self.a0);
            if self.list_step(s) == 16 {
                let vlo = loc_reg(self.vstack[vd - 1]);
                let vhi = loc_reg(self.vstack[vd]);
                self.ctr_store_elem(a1, ew, vlo, Some(vhi));
            } else {
                let v = loc_reg(self.vstack[vd]);
                self.ctr_store_elem(a1, ew, v, None);
            }
            for _ in 0..vent {
                self.pop_vreg();
            }
            self.pop_vreg(); // index
            return Ok(());
        }
        self.itemset_bounds_guards(s, kr)?;
        let base = self.list_data_off[s];
        debug_assert!(base >= 0, "list data block missing from the frame plan");
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        // B2 task 4: the element stride follows the slot's fixed width
        let step = self.list_step(s);
        let sh = step.trailing_zeros();
        if sh > 0 {
            self.asm.slli_(sc2, kr, sh); // idx * step (idx < len ≤ capacity)
        } else {
            self.asm.mv(sc2, kr);
        }
        self.asm.addi(sc1, S0, base);
        self.asm.add(sc1, sc1, sc2);
        if step == 16 {
            let vlo = loc_reg(self.vstack[vd - 1]);
            let vhi = loc_reg(self.vstack[vd]);
            self.list_store_pair(vlo, vhi, sc1, 0);
        } else {
            let v = loc_reg(self.vstack[vd]);
            self.list_store_inst(v, sc1, 0, step);
        }
        for _ in 0..vent {
            self.pop_vreg(); // value (pair halves first when u128)
        }
        self.pop_vreg(); // index
        Ok(())
    }

    /// Bytes per element of list slot `s` — the element block stride (B2
    /// task 4: the slot's fixed element width; the width is plan-fixed, so
    /// this is a compile-time constant per slot). T19: a u128 slot
    /// answers 16 — its elements are (lo, hi) pairs stored as two
    /// consecutive little-endian words.
    pub(crate) fn list_step(&self, s: usize) -> u32 {
        match self.lists.ew.get(s).copied() {
            Some(w) if w >= 8 => w / 8,
            _ => 8,
        }
    }

    /// The zero-extending element load and the truncating element store
    /// for the slot's element width (the frame block / rodata table store
    /// each element little-endian at its fixed stride). Step 16 (u128
    /// elements, T19) has no single-register form — the pair helpers below
    /// cover it.
    pub(crate) fn list_load_inst(&mut self, dst: u8, addr: u8, imm: i32, step: u32) {
        match step {
            8 => self.asm.ld(dst, addr, imm),
            4 => self.asm.lwu(dst, addr, imm),
            2 => self.asm.lhu(dst, addr, imm),
            _ => self.asm.lbu(dst, addr, imm),
        }
    }
    pub(crate) fn list_store_inst(&mut self, val: u8, addr: u8, imm: i32, step: u32) {
        match step {
            8 => self.asm.sd(val, addr, imm),
            4 => self.asm.sw(val, addr, imm),
            2 => self.asm.sh(val, addr, imm),
            _ => self.asm.sb(val, addr, imm),
        }
    }

    /// T19: the 16B-stride element load/store — a u128 element is the
    /// (lo, hi) pair at the element address (little-endian words).
    pub(crate) fn list_load_pair(&mut self, lo: u8, hi: u8, addr: u8, imm: i32) {
        self.asm.ld(lo, addr, imm);
        self.asm.ld(hi, addr, imm + 8);
    }
    pub(crate) fn list_store_pair(&mut self, lo: u8, hi: u8, addr: u8, imm: i32) {
        self.asm.sd(lo, addr, imm);
        self.asm.sd(hi, addr, imm + 8);
    }

    /// `item_concat(l)` — list → bytes. Every element contributes its
    /// `extract_bytes` form (a uint is its width's big-endian bytes — the
    /// element block already stores exactly those bytes little-endian, so
    /// the splice is a straight byte copy of the element block; same splice
    /// as `extract_call_data`'s one-layer list). Faces: OutOfValueSize 39
    /// when the total exceeds the kernel's output budget (S3), then the
    /// arena-exhaustion resource face. Empty lists yield the empty bytes.
    pub(crate) fn emit_itemcat(&mut self, s: usize) -> Ret<()> {
        let step = self.list_step(s); // bytes per element (a power of two)
        let sh = step.trailing_zeros();
        // element count: folded for the static classes, the live length
        // shadow for the growable one
        let (static_n, rodata) = match self.lists.kind[s] {
            ListKind::Empty => (Some(0u64), None),
            ListKind::Rodata { n } => (Some(n), Some(self.list_rodata_lbl(s)?)),
            ListKind::FrameStatic { n } => (Some(n), None),
            ListKind::Growable { .. } => (None, None),
            // P2 prework boundary: the tagged-cell splice is T17's
            // lowering (plan already rejects the shape with the arena
            // message; this is the defensive twin)
            ListKind::Arena { .. } => {
                return errf!(
                    "rv64 lower: opcode ITEMCAT is outside the translatable kernel subset \
                     (the receiver list escapes into the arena container model; item_concat \
                     on an aliased list is not yet lowered)"
                );
            }
            ListKind::NotList => {
                return errf!(
                    "rv64 lower: opcode ITEMCAT is outside the translatable kernel subset"
                );
            }
        };
        let n_r = self.push_vreg();
        match static_n {
            Some(n) => self.asm.li(n_r, n),
            None => {
                let cell = self.slot_offset(s as u8);
                self.asm.ld(n_r, S0, cell);
            }
        }
        let base = self.push_vreg();
        match rodata {
            Some(lbl) => self.lit_addr(base, lbl),
            None => {
                if static_n == Some(0) {
                    // Empty: the loop never runs — a dead base value (the
                    // slot has no data block at all)
                    self.asm.mv(base, ZERO);
                } else {
                    let off = self.list_data_off[s];
                    debug_assert!(off >= 0, "list data block missing from the frame plan");
                    self.asm.addi(base, S0, off);
                }
            }
        }
        // total = n * step; OutOfValueSize 39 when it exceeds the kernel's
        // output budget (S3; == is legal — checked_value_output_add against
        // value_size), then the arena-exhaustion resource face
        let total = self.push_vreg();
        if sh > 0 {
            self.asm.slli_(total, n_r, sh); // len ≤ cap: no wrap
        } else {
            self.asm.mv(total, n_r);
        }
        let l39 = self.nil_thunk(39);
        self.asm.bltu(S3, total, l39);
        self.emit_arena_check(total);
        // the splice: every element contributes its `extract_bytes` form —
        // the width's big-endian bytes (the element block stores each
        // element little-endian, so this is a per-element byte-reverse,
        // NOT a straight block copy). Empty lists fall through the loop.
        // The STORAGE stride can differ from the splice width: an
        // all-u32-fitting rodata table packs 4-byte-stride (M2-d rule,
        // loaded by lwu zero-extend — the loaded VALUE is identical).
        let ld_step = step;
        let ld_sh = ld_step.trailing_zeros();
        let i = self.push_vreg();
        self.asm.addi(i, ZERO, 0);
        let addr = self.push_vreg();
        let t = self.push_vreg();
        let b = self.push_vreg();
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(i, n_r, l_done);
        if ld_sh > 0 {
            self.asm.slli_(addr, i, ld_sh);
            self.asm.add(addr, base, addr);
        } else {
            self.asm.add(addr, base, i);
        }
        // load the element (zero-extend — the table/block stores the
        // value at the storage stride). T19: a 16-byte-stride element is
        // a (lo, hi) pair — each word is loaded and reversed separately
        // (the high word's BE bytes precede the low word's).
        match ld_step {
            8 => self.asm.ld(t, addr, 0),
            4 => self.asm.lwu(t, addr, 0),
            2 => self.asm.lhu(t, addr, 0),
            1 => self.asm.lbu(t, addr, 0),
            _ => {}
        }
        if step == 16 {
            for half in [8i32, 0] {
                self.asm.ld(t, addr, half);
                for k in (0..8u32).rev() {
                    if k > 0 {
                        self.asm.srli_(b, t, 8 * k);
                    } else {
                        self.asm.mv(b, t);
                    }
                    self.asm.sb(b, S1, 0);
                    self.asm.addi(S1, S1, 1);
                }
            }
        } else {
            for j in 0..step {
                let k = step - 1 - j;
                if k > 0 {
                    self.asm.srli_(b, t, 8 * k as u32);
                } else {
                    self.asm.mv(b, t);
                }
                self.asm.sb(b, S1, 0);
                self.asm.addi(S1, S1, 1);
            }
        }
        self.asm.addi(i, i, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        let _ = self.pop_vreg(); // b
        let _ = self.pop_vreg(); // t
        let _ = self.pop_vreg(); // addr
        let _ = self.pop_vreg(); // i
        // the result descriptor: (S1 - total, total)
        let tr = loc_reg(self.vstack[self.vstack.len() - 1]); // total
        let _ = self.pop_vreg(); // total
        let _ = self.pop_vreg(); // base
        let _ = self.pop_vreg(); // n
        let rp = self.push_vreg();
        self.asm.r(0x20, tr, S1, 0, rp, 0b0110011); // sub rp, S1, total
        let rl = self.push_vreg();
        self.asm.mv(rl, tr);
        Ok(())
    }

    /// `buf_set(b, ost, src)` on the local bytes slot `s` — copy-then-patch.
    /// The receiver's bytes are copied into a fresh arena region, the
    /// source bytes are written at `ost` inside the copy, and the copy is
    /// the RESULT. The slot itself stays untouched: on the interpreter the
    /// receiver is a deep-copied stack value (`bufset` mutates it in
    /// place; every other holder — the local variable included — keeps
    /// its pre-mutation bytes). rv64's descriptor may alias anything (a
    /// cut of `data`, another slot, the literal pool), so an in-place
    /// patch would corrupt every holder — compiler.md premise 24.
    /// Length never changes.
    ///
    /// Faces in interpreter order (execute.rs BUFSET → operand.rs bufset):
    /// src extract → 91 (Nil), ost extract_u16 → 72 (Nil / ≥ 2¹⁶),
    /// receiver extract → 91 (an unassigned tracked slot), boundary → 42
    /// (`ost + src.len` wraps or exceeds the length; == length is legal).
    /// The arena-exhaustion face (71) is the kernel's memory budget, not a
    /// semantic code.
    pub(crate) fn emit_bufset(&mut self, s: usize, ost: &dyn IRNode, src: &dyn IRNode) -> Ret<()> {
        let stracked = self.tracked.get(s).copied().unwrap_or(false);
        // stack order: receiver (static), ost, src — subexpr faults in that
        // order; the instruction's own checks run after both evaluations.
        //
        // Register discipline: EVERYTHING goes on the vstack first (the
        // depth→register handout is collision-free while values stay
        // pushed); all pops happen together at the end, and no push_vreg
        // may run between the pops and the last use of a popped register.
        self.eval_int(ost)?;
        let od = self.vstack.len() - 1;
        let src_is_bytes = node_res_ty(src, self.slot)? == Ty::Bytes;
        self.eval(src)?;
        let src_len_d = self.vstack.len() - 1; // the len half / the scalar
        // src extract_bytes → BytesHandle 91 on a Nil (tag under the len
        // half for a bytes pair, at the value for a scalar)
        let src_tag_d = self.vstack.len() - if src_is_bytes { 2 } else { 1 };
        if self.nil_val(src) {
            self.nil_guard_tag(src_tag_d, 91);
        }
        // ost extract_u16 → 72: Nil, then ≥ 2¹⁶
        if self.nil_val(ost) {
            self.nil_guard_tag(od, 72);
        }
        let kr = loc_reg(self.vstack[od]);
        let l72 = self.nil_thunk(72);
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, 1u64 << 16);
        self.asm.bgeu(kr, sc1, l72);
        // A scalar source's canonical byte length is the width's byte size
        // (1 for the pure-bool width) — a compile-time constant, so the
        // boundary's checked_add cannot wrap. The scalar's canonical bytes
        // are materialized in the arena NOW (while the scalar is the
        // vstack top); this moves the arena-exhaustion RESOURCE face ahead
        // of the remaining semantic faces, which the iron law allows.
        let src_w = if src_is_bytes {
            None
        } else {
            match self.int_width(src) {
                Some(w) => Some(w),
                None => {
                    return errf!(
                        "rv64 lower: buf_set is outside the translatable kernel subset \
                         (the written value has no static uint width — pin bool or \
                         comparison values with `as u64`)"
                    );
                }
            }
        };
        if let Some(w) = src_w {
            self.emit_cbytes(w)?;
        }
        // after emit_cbytes both source shapes are the bytes pair
        // [.., sp, sl] (the scalar slot became the ptr, len pushed above)
        let sl_d = self.vstack.len() - 1;
        // receiver descriptor: fresh vregs — a banked slot has NO frame
        // cell, so the frame offset is only read in the un-banked branch
        let rp = self.push_vreg();
        let rl = self.push_vreg();
        match self.bank_bytes(s as u8) {
            Some((bp, bl)) => {
                self.asm.mv(rp, bp);
                self.asm.mv(rl, bl);
            }
            None => {
                let off = self.slot_offset(s as u8);
                self.asm.ld(rp, S0, off);
                self.asm.ld(rl, S0, off + 8);
            }
        }
        if stracked {
            self.nil_guard_flag(s as u8, 91); // receiver extract → 91
        }
        // boundary: ost + src.len must not wrap and must be ≤ receiver len
        // (== length is legal); only 42 / no-fault fall through
        let src_l = loc_reg(self.vstack[sl_d]);
        let l42 = self.nil_thunk(42);
        if src_is_bytes {
            self.asm.r(0, kr, src_l, 0, sc1, 0b0110011); // add sc1, ost, src_len
            self.asm.bltu(sc1, kr, l42); // wrapped
        } else {
            // ost + nb (nb ≤ 8, ost < 2¹⁶: no wrap)
            let nb = if src_w.unwrap() == 1 { 1i32 } else { (src_w.unwrap() / 8) as i32 };
            self.asm.addi(sc1, kr, nb);
        }
        self.asm.bltu(rl, sc1, l42); // end > len
        // copy targets before anything pops: t1 = d + ost (the write
        // address), t2 = d + rl (the post-copy bump) — d is the current S1
        let t1 = self.push_vreg();
        self.asm.add(t1, S1, kr);
        let t2 = self.push_vreg();
        self.asm.add(t2, S1, rl);
        self.emit_arena_check(rl); // the slot's own copy must fit
        // pop everything at once (depths top-down): the receiver pair, the
        // source pair, ost, then the two targets — their registers stay
        // valid through the two memcpys because NOTHING is pushed until
        // the result pair
        let _ = self.pop_vreg(); // t2
        let _ = self.pop_vreg(); // t1
        let rl_r = self.pop_reg(); // receiver len
        let rp_r = self.pop_reg(); // receiver ptr
        let src_l = self.pop_reg(); // source len
        let src_p = self.pop_reg(); // source ptr
        let kr_r = loc_reg(self.vstack[self.vstack.len() - 1]); // ost
        self.pop_vreg(); // ost
        // 1. copy the slot's own bytes into a fresh arena region — the
        // patch lands in THIS copy and the copy IS the result. The slot
        // itself stays untouched: on the interpreter the receiver is a
        // deep-copied stack value (bufset mutates it in place; "other
        // slots keep their pre-mutation copy" — the local variable reads
        // back UNPATCHED, probed: `buf_set(b,0,v); byte(b,0)` = the old
        // byte). Never patch a shared/owned buffer in place.
        self.emit_memcpy(rp_r, rl_r); // S1 advances to d + len (= t2)
        // 2. patch the copy: redirect the bump to d + ost, memcpy the
        // source bytes, restore
        self.asm.mv(S1, t1);
        self.emit_memcpy(src_p, src_l);
        self.asm.mv(S1, t2);
        // 3. the result pair: new ptr = d (= t2 − rl), unchanged len
        let rp2 = self.push_vreg();
        self.asm.r(0x20, rl_r, t2, 0, rp2, 0b0110011); // sub rp2, t2, rl
        let rl2 = self.push_vreg();
        self.asm.mv(rl2, rl_r);
        Ok(())
    }

    /// `l[i] op= v` — the native lowering of the parser's raw stack
    /// fragment (`DUPN 2; ITEMGET; ROLL 3; op; ITEMSET`, RHS-first). The
    /// IR children carry the actual subtrees, so each evaluates exactly
    /// once, in the fragment's order: value, receiver (a static slot —
    /// nothing to evaluate), index. Faces in interpreter order: index
    /// extract_u32 → 72, bounds → 85 (the ITEMGET stage), then the
    /// value's CastFail 71 face, then the arithmetic's overflow 90 —
    /// all before the write-back (the fragment's own ITEMSET re-checks
    /// cannot refire on the same index/length).
    pub(crate) fn emit_item_compound(
        &mut self,
        value: &dyn IRNode,
        receiver: &dyn IRNode,
        index: &dyn IRNode,
        aop: Bytecode,
    ) -> Ret<()> {
        let s = self.list_slot_of(receiver, Bytecode::ITEMSET)?;
        if matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            // P2 prework boundary: the compound read-modify-write over a
            // tagged arena cell is T17's lowering; B-family reject here
            return errf!(
                "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                 (compound item assignment on an arena-model (aliased/cloned) list is \
                 not yet lowered — use item reads and item_set writes separately)"
            );
        }
        if !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
            return errf!(
                "rv64 lower: opcode ITEMSET is outside the translatable kernel subset \
                 (internal: compound assignment on a non-growable storage class)"
            );
        }
        // RHS-first: value, then index (the receiver is the slot itself).
        // Web cursor: `self.web_idx` is the compound IRLIST's own position;
        // walk_stmt numbers the subs value=+1, receiver=+2, index=+3,
        // raw=+4, mark=+5 (event-less positions for the receiver GET, the
        // raw fragment and the ROLL0 mark — the value/index evals must read
        // their bank homes exactly there, INV-check invariant).
        let base = self.web_idx;
        let lv = self.nil_val(value);
        self.web_idx = base + 1;
        self.eval_int(value)?;
        // T19: a u128 compound value is a (lo, hi) pair — the Nil tag sits
        // on its LOW entry (like every u128 pair)
        let vent = Self::entries_of_width(self.int_width(value));
        let vd = self.vstack.len() - 1;
        self.web_idx = base + 3;
        self.eval_int(index)?;
        // the mark sub (+5) is the last consumed position: the next
        // statement starts its own count from here
        self.web_idx = base + 5;
        let kd = self.vstack.len() - 1;
        if self.nil_val(index) {
            self.nil_guard_tag(kd, 72); // extract_u32(Nil) → 72 (itemget)
        }
        let kr = loc_reg(self.vstack[kd]);
        self.itemset_bounds_guards(s, kr)?; // 72 (≥ 2³²) then 85 (≥ len)
        if lv {
            // vd is the value's TOP entry as an absolute vstack position;
            // a pair's Nil tag sits on its low half (vd - 1)
            let tag_d = if vent == 2 { vd - 1 } else { vd };
            self.nil_guard_tag(tag_d, 71); // CastFail on the Nil rhs
        }
        // read the element, apply the arithmetic, write back — the element
        // is a definite uint (may-Nil stores are compile-rejected), so the
        // op runs at the element block's width (task 1: the interpreter
        // widens to the common width; a u64 element forces 64)
        let base = self.list_data_off[s];
        debug_assert!(base >= 0, "list data block missing from the frame plan");
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        // B2 task 4: read-modify-write runs at the slot's fixed element
        // width (the interpreter widens the operands to the element width;
        // for a u64 element that is the historical 64-bit form)
        let step = self.list_step(s);
        let sh = step.trailing_zeros();
        let scale = |lw: &mut Self| {
            if sh > 0 {
                lw.asm.slli_(sc2, kr, sh); // idx * step
            } else {
                lw.asm.mv(sc2, kr);
            }
            lw.asm.addi(sc1, S0, base);
            lw.asm.add(sc1, sc1, sc2); // element address
        };
        if step == 16 {
            // T19: the u128 read-modify-write. The element is a (lo, hi)
            // pair; the rhs is a pair (statically u128) or a single
            // sub-128 value zero-extended to 128 — the interpreter widens
            // both sides to the common width, which a u128 element always
            // dominates. The element address is recomputed after the op:
            // the u128 helpers and the shared divmod routine clobber every
            // scratch (pool and scratches stay disjoint in every mode, so
            // the operand registers are safe).
            scale(self);
            let it_lo = self.push_vreg();
            let it_hi = self.push_vreg();
            self.list_load_pair(it_lo, it_hi, sc1, 0);
            let (vlo, vhi) = if vent == 2 {
                (loc_reg(self.vstack[vd - 1]), loc_reg(self.vstack[vd]))
            } else {
                (loc_reg(self.vstack[vd]), ZERO)
            };
            match aop {
                Bytecode::ADD => self.emit_u128_add(it_lo, it_hi, vlo, vhi),
                Bytecode::SUB => self.emit_u128_sub(it_lo, it_hi, vlo, vhi),
                Bytecode::MUL => self.emit_u128_mul(it_lo, it_hi, vlo, vhi),
                Bytecode::DIV => {
                    // same frame-workspace convention as the u128 XOP DIV
                    // (the routine's q lands back in the item registers)
                    let ws = self.divmod_save_off;
                    debug_assert!(ws >= 0, "divmod call without a reserved save area");
                    self.asm.sd(it_lo, S0, ws);
                    self.asm.sd(it_hi, S0, ws + 8);
                    self.asm.sd(vlo, S0, ws + 16);
                    self.asm.sd(vhi, S0, ws + 24);
                    let lbl = self.u128_divmod_label();
                    self.asm.sd(RA, S0, ws + 128);
                    self.asm.jal_(RA, lbl);
                    self.asm.ld(RA, S0, ws + 128);
                    self.asm.ld(it_lo, S0, ws + 32);
                    self.asm.ld(it_hi, S0, ws + 40);
                }
                other => {
                    return errf!(
                        "rv64 lower: internal: compound op {other:?} has no u128 list \
                         lowering (plan should have rejected it)"
                    )
                }
            }
            scale(self); // recompute the element address (scratches died)
            self.list_store_pair(it_lo, it_hi, sc1, 0);
            let _ = self.pop_vreg(); // item hi
            let _ = self.pop_vreg(); // item lo
            let _ = self.pop_vreg(); // index
            for _ in 0..vent {
                let _ = self.pop_vreg(); // value
            }
            return Ok(());
        }
        scale(self);
        let item = self.push_vreg();
        self.list_load_inst(item, sc1, 0, step);
        let v = loc_reg(self.vstack[vd]);
        let w = self.lists.ew.get(s).copied().unwrap_or(64).max(8);
        self.emit_arith(aop, item, v, w)?;
        self.list_store_inst(item, sc1, 0, step);
        self.pop_vreg(); // item
        self.pop_vreg(); // index
        self.pop_vreg(); // value
        Ok(())
    }

    /// The pool label of a list's rodata table; `must_register` registers
    /// it on first (creation) use.
    pub(crate) fn list_rodata_lbl(&mut self, s: usize) -> Ret<usize> {
        if self.list_lbl.get(s).copied().unwrap_or(usize::MAX) != usize::MAX {
            return Ok(self.list_lbl[s]);
        }
        let vals = self.lists.lits.get(s).cloned().unwrap_or_default();
        // B2 task 4: the table stride IS the slot's element width (the
        // element type is fixed per slot; a u32-element table takes the
        // 4-byte form the M2-d rule used to give all-u32-fitting tables —
        // with per-slot widths that packing IS the natural stride). T19:
        // a u128 slot's table materializes each element's full 16 LE
        // bytes (lo word then hi word); `lits` carries one u128 per
        // element, so the low `stride` bytes of each entry are exactly
        // the ≤64-bit tables' old bytes.
        let stride = self.list_step(s) as usize;
        let n = match self.lists.kind.get(s) {
            Some(ListKind::Rodata { n }) => *n as usize,
            _ => vals.len(),
        };
        let mut bytes = Vec::with_capacity(n * stride);
        for v in vals.iter().take(n) {
            let raw = v.to_le_bytes();
            bytes.extend_from_slice(&raw[..stride]);
        }
        let lbl = self.lit_label(&bytes);
        self.list_lbl[s] = lbl;
        Ok(lbl)
    }

    /// Emit `idx ≥ 2³² → 72-thunk` (the extract_u32 boundary) and
    /// `idx ≥ len → 85-thunk`; `idx_reg` holds the variable index.
    pub(crate) fn emit_list_bounds_guards(
        &mut self,
        idx_reg: u8,
        s: usize,
        static_len: Option<u64>,
    ) -> Ret<()> {
        let l72 = self.nil_thunk(72);
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(idx_reg, sc1, l72);
        let l85 = self.nil_thunk(85);
        match static_len {
            Some(0) => {
                // empty list: every index is out of bounds
                self.asm.jal_(ZERO, l85);
            }
            Some(n) => {
                self.asm.addi(sc1, ZERO, n as i32);
                self.asm.bgeu(idx_reg, sc1, l85);
            }
            None => {
                let cell = self.slot_offset(s as u8);
                self.asm.ld(sc1, S0, cell);
                self.asm.bgeu(idx_reg, sc1, l85);
            }
        }
        Ok(())
    }

    /// Load list element `idx_reg` into `dst` in place (base materialized
    /// in sc1, scaled index in sc2). B2 task 4: the stride and the
    /// zero-extending load width are the slot's fixed element width (the
    /// scaled index stays < 1024 by the block budget). T19: a 16-byte
    /// stride loads the (lo, hi) pair — `dst_hi` carries the high half.
    pub(crate) fn emit_list_load(
        &mut self,
        dst: u8,
        dst_hi: Option<u8>,
        idx_reg: u8,
        s: usize,
        rodata: Option<usize>,
    ) -> Ret<()> {
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match rodata {
            Some(lbl) => {
                self.lit_addr(sc1, lbl);
            }
            None => {
                let base = self.list_data_off[s];
                debug_assert!(base >= 0, "list data block missing from the frame plan");
                self.asm.addi(sc1, S0, base);
            }
        }
        let step = self.list_step(s);
        let sh = step.trailing_zeros();
        if sh > 0 {
            self.asm.slli_(sc2, idx_reg, sh);
            self.asm.add(sc1, sc1, sc2);
        } else {
            self.asm.add(sc1, sc1, idx_reg);
        }
        match dst_hi {
            Some(hi) => self.list_load_pair(dst, hi, sc1, 0),
            None => self.list_load_inst(dst, sc1, 0, step),
        }
        Ok(())
    }


    /// TAKEFIRST/TAKELAST: pop the front/back element of the shared object
    /// (empty → CompoOpOverflow 81) and push it. The receiver's descriptor
    /// stays in its slot cell; the popped element's mutation is visible
    /// through every alias (the shared header word is the live length).
    pub(crate) fn emit_take(&mut self, s: usize, front: bool) -> Ret<()> {
        if !matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            return errf!(
                "rv64 lower: opcode TAKEFIRST/TAKELAST is outside the translatable kernel \
                 subset (internal: take receiver without an Arena class)"
            );
        }
        let sc1 = self.asm.sc1; // obj
        let sc2 = self.asm.sc2; // len
        let a0 = self.a0;
        let step128 = self.list_step(s) == 16;
        self.ctr_load_obj(s, sc1);
        self.asm.ld(sc2, sc1, 0);
        let l81 = self.nil_thunk(81);
        self.asm.beq(sc2, ZERO, l81); // empty list → 81
        // element address: front → obj+16, back → obj+16+(len-1)*24
        let idx = self.push_vreg();
        if front {
            self.asm.addi(idx, ZERO, 0);
        } else {
            self.asm.addi(idx, sc2, -1);
        }
        let addr = self.push_vreg();
        self.ctr_elem_addr(addr, sc1, idx, a0);
        self.pop_vreg(); // addr — re-handed as the payload destination below
        self.pop_vreg(); // idx
        // load the element (the address register is re-handed as the
        // payload destination; `ld rd, rs, imm` with rd == rs is exact)
        let r = self.push_vreg();
        let r2 = if step128 { Some(self.push_vreg()) } else { None };
        self.ctr_load_elem(r, r2, addr);
        if front {
            // compact: elements 1..len-1 move one cell down (count = len-1)
            let cnt = self.push_vreg();
            self.asm.addi(cnt, sc2, -1);
            let one = self.push_vreg();
            self.asm.addi(one, ZERO, 1);
            self.ctr_shift_left_from(sc1, one, cnt);
            self.pop_vreg(); // one
            self.pop_vreg(); // cnt
        }
        // publish len-1 (the shared header word)
        self.asm.addi(sc2, sc2, -1);
        self.asm.sd(sc2, sc1, 0);
        Ok(())
    }

    /// MERGE(list, list): append every element of the source object into
    /// the receiver's object. Same shared object → CompoOpInvalid(80);
    /// new length over the 128-element cap → 40. The arena cells are
    /// tagged scalars only, so the interpreter's per-element check_scalar
    /// (76) cannot fire.
    pub(crate) fn emit_merge(&mut self, rs: usize, ss: usize) -> Ret<()> {
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        let a0 = self.a0;
        let src = self.push_vreg();
        let dst = self.push_vreg();
        self.ctr_load_obj(ss, src);
        self.ctr_load_obj(rs, dst);
        let l80 = self.nil_thunk(80);
        self.asm.beq(src, dst, l80); // same Rc → 80
        self.asm.ld(a0, src, 0); // source len
        self.asm.ld(sc2, dst, 0); // receiver len
        self.asm.add(sc1, a0, sc2); // new length
        let cap = self.push_vreg();
        self.asm.addi(cap, ZERO, CTR_LIST_CAP as i32 + 1);
        let l40 = self.nil_thunk(40);
        self.asm.bgeu(sc1, cap, l40); // new length > 128 → 40
        self.pop_vreg(); // cap
        // copy 3*source_len words from src+16 to dst+16+len*24
        let cnt = self.push_vreg();
        self.asm.ld(a0, src, 0); // source len again (0 → empty copy)
        self.asm.slli_(cnt, a0, 1);
        self.asm.add(cnt, cnt, a0); // 3*len
        let sp = self.push_vreg();
        let dp = self.push_vreg();
        self.asm.addi(sp, src, CTR_LIST_HDR as i32);
        self.ctr_elem_addr(dp, dst, sc2, a0); // dst + 16 + len*24
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_done);
        self.asm.ld(a0, sp, 0);
        self.asm.sd(a0, dp, 0);
        self.asm.addi(sp, sp, 8);
        self.asm.addi(dp, dp, 8);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.asm.sd(sc1, dst, 0); // publish the new length
        self.pop_vreg(); // dp
        self.pop_vreg(); // sp
        self.pop_vreg(); // cnt
        self.pop_vreg(); // dst
        self.pop_vreg(); // src
        Ok(())
    }

    /// INSERT(list, idx, v): the true shifting insert. Faces in the
    /// interpreter's order: the index expression evaluated first, then the
    /// value's check_scalar (76 — a list value, trapped here), then
    /// extract_u32 (72), then idx > len → CompoOpOverflow(81), then
    /// len ≥ 128 → 40.
    pub(crate) fn emit_insert(&mut self, s: usize, index: &dyn IRNode, value: &dyn IRNode) -> Ret<()> {
        if !matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            return errf!(
                "rv64 lower: opcode INSERT is outside the translatable kernel subset \
                 (internal: insert receiver without an Arena class)"
            );
        }
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        let a0 = self.a0;
        // the index rides ONE register into the extract_u32 comparison; a
        // u128 index would need the pair path (B-family reject)
        if self.int_width(index) == Some(128) {
            return errf!(
                "rv64 lower: opcode INSERT is outside the translatable kernel subset \
                 (a u128 insert index is not representable — use a u64 index)"
            );
        }
        // BUG-029 family (may-Nil pair value precedent): the interpreter
        // ADMITS a Nil insert value (check_scalar) and faults only at a
        // later read of the hole; a tagged scalar cell cannot represent
        // Nil, so the shape is a B-family compile reject.
        if matches!(node_res_ty(value, self.slot)?, Ty::Int) && self.nil_val(value) {
            return errf!(
                "rv64 lower: opcode INSERT is outside the translatable kernel subset \
                 (the inserted value may be Nil: the interpreter stores a Nil element \
                 and faults only when that element is later read, which a tagged \
                 scalar cell cannot represent — compute the value from definite \
                 assignments)"
            );
        }
        self.eval_int(index)?;
        let kd = self.vstack.len() - 1;
        if matches!(node_res_ty(value, self.slot)?, Ty::List) {
            // check_scalar(value) → CastBeValueFail(76), after the index
            // evaluated (both operands are on the interpreter stack first)
            self.pop_vreg();
            self.trap(76);
            return Ok(());
        }
        self.eval_int(value)?;
        let vent = Self::entries_of_width(self.int_width(value));
        let vd = self.vstack.len() - 1;
        if self.nil_val(index) {
            self.nil_guard_tag(kd, 72);
        }
        let kr = loc_reg(self.vstack[kd]);
        let l72 = self.nil_thunk(72);
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(kr, sc1, l72); // extract_u32: ≥ 2³² → 72
        let cell = self.slot_offset(s as u8);
        self.asm.ld(sc1, S0, cell); // obj
        self.asm.ld(sc2, sc1, 0); // len
        let l81 = self.nil_thunk(81);
        self.asm.bltu(sc2, kr, l81); // len < idx → 81 (i > len)
        let l40 = self.nil_thunk(40);
        self.asm.li(a0, CTR_LIST_CAP);
        self.asm.bgeu(sc2, a0, l40); // len ≥ 128 → 40
        // shift elements idx..len-1 one cell up (count = len - idx)
        let cnt = self.push_vreg();
        self.asm.sub(cnt, sc2, kr);
        self.ctr_shift_right_from(sc1, sc2, cnt);
        // store the new element at cell idx (recompute the address: the
        // shift loop only wrote its own pool temps and a0)
        let addr = self.push_vreg();
        self.ctr_elem_addr(addr, sc1, kr, a0);
        let ew = self.lists.ew.get(s).copied().unwrap_or(64);
        if self.list_step(s) == 16 {
            let vlo = loc_reg(self.vstack[vd - 1]);
            let vhi = loc_reg(self.vstack[vd]);
            self.ctr_store_elem(addr, ew, vlo, Some(vhi));
        } else {
            let v = loc_reg(self.vstack[vd]);
            self.ctr_store_elem(addr, ew, v, None);
        }
        // publish len+1 (the shared header word)
        self.asm.ld(sc2, sc1, 0);
        self.asm.addi(sc2, sc2, 1);
        self.asm.sd(sc2, sc1, 0);
        self.pop_vreg(); // addr
        self.pop_vreg(); // cnt
        for _ in 0..vent {
            self.pop_vreg(); // value
        }
        self.pop_vreg(); // index
        Ok(())
    }

    /// REMOVE(list, idx): extract_u32 (72), then idx ≥ len →
    /// CompoNoFindItem(85), then the left shift.
    pub(crate) fn emit_remove(&mut self, s: usize, index: &dyn IRNode) -> Ret<()> {
        if !matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            return errf!(
                "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                 (internal: remove receiver without an Arena class)"
            );
        }
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        if self.int_width(index) == Some(128) {
            return errf!(
                "rv64 lower: opcode REMOVE is outside the translatable kernel subset \
                 (a u128 remove index is not representable — use a u64 index)"
            );
        }
        self.eval_int(index)?;
        let kd = self.vstack.len() - 1;
        if self.nil_val(index) {
            self.nil_guard_tag(kd, 72);
        }
        let kr = loc_reg(self.vstack[kd]);
        let l72 = self.nil_thunk(72);
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(kr, sc1, l72); // extract_u32: ≥ 2³² → 72
        let cell = self.slot_offset(s as u8);
        self.asm.ld(sc1, S0, cell); // obj
        self.asm.ld(sc2, sc1, 0); // len
        let l85 = self.nil_thunk(85);
        self.asm.bgeu(kr, sc2, l85); // idx ≥ len → 85
        // count = len - 1 - idx, start = idx + 1
        self.asm.addi(sc2, sc2, -1);
        self.asm.sub(sc2, sc2, kr);
        let start = self.push_vreg();
        self.asm.addi(start, kr, 1);
        self.ctr_shift_left_from(sc1, start, sc2);
        self.pop_vreg(); // start
        // publish len-1
        self.asm.ld(sc2, sc1, 0);
        self.asm.addi(sc2, sc2, -1);
        self.asm.sd(sc2, sc1, 0);
        self.pop_vreg(); // index
        Ok(())
    }

    /// EQ/NEQ over two list objects: the interpreter's `value_content_eq`
    /// — pointer equality first, else element-wise numeric equality over
    /// the payload words (uints compare by value across widths; the tag is
    /// ignored). Pushes one 0/1 value.
    pub(crate) fn emit_list_eq(&mut self, op: Bytecode, sx: usize, sy: usize) -> Ret<()> {
        if !matches!(self.lists.kind.get(sx), Some(ListKind::Arena { .. }))
            || !matches!(self.lists.kind.get(sy), Some(ListKind::Arena { .. }))
        {
            return errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (internal: list operand without an Arena class)"
            );
        }
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        let a0 = self.a0;
        let a1 = self.a1;
        let r = self.push_vreg();
        let a = self.push_vreg();
        let b = self.push_vreg();
        self.ctr_load_obj(sx, a);
        self.ctr_load_obj(sy, b);
        self.asm.addi(r, ZERO, 1);
        let l_diff = self.asm.new_label();
        let l_end = self.asm.new_label();
        self.asm.beq(a, b, l_end); // same Rc → equal
        self.asm.ld(sc1, a, 0);
        self.asm.ld(sc2, b, 0);
        self.asm.bne(sc1, sc2, l_diff); // different lengths → false
        // compare each element's two payload words (the tag word between
        // cells is skipped; uints compare by numeric value across widths)
        let cnt = self.push_vreg();
        self.asm.mv(cnt, sc1); // live element count
        let pa = self.push_vreg();
        let pb = self.push_vreg();
        self.asm.addi(pa, a, CTR_LIST_HDR as i32 + 8);
        self.asm.addi(pb, b, CTR_LIST_HDR as i32 + 8);
        let l_loop = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_end);
        self.asm.ld(a0, pa, 0);
        self.asm.ld(a1, pb, 0); // a1 == sc2 in ImcRemap; sc2 is dead here
        self.asm.bne(a0, a1, l_diff);
        self.asm.ld(a0, pa, 8);
        self.asm.ld(a1, pb, 8);
        self.asm.bne(a0, a1, l_diff);
        self.asm.addi(pa, pa, CTR_ELEM_BYTES as i32);
        self.asm.addi(pb, pb, CTR_ELEM_BYTES as i32);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_diff);
        self.asm.addi(r, ZERO, 0);
        self.place(l_end);
        if op == Bytecode::NEQ {
            self.asm.i(1, r, 4, r, 0b0010011); // xori r, r, 1
        }
        self.pop_vreg(); // pb
        self.pop_vreg(); // pa
        self.pop_vreg(); // cnt
        self.pop_vreg(); // b
        self.pop_vreg(); // a
        Ok(())
    }

    /// APPEND(l, v): the value evaluates first (its faults), then the
    /// 128 cap (40), then the store — compo.rs append runs check_scalar
    /// (76; the plan-guaranteed list-value trap) BEFORE the cap, and a
    /// definite uint has no further fault face. Shared by the statement
    /// form and the `var m = append(l, v)` bind (T17).
    pub(crate) fn emit_append(&mut self, s: usize, value: &dyn IRNode) -> Ret<()> {
        let arena_kind = matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. }));
        if !arena_kind && !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
            return errf!(
                "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                 (internal: append on a non-growable storage class)"
            );
        }
        // T17: a list value is the interpreter's check_scalar 76 face
        // (before the cap); only the plain GET form is admitted by plan()
        if matches!(node_res_ty(value, self.slot)?, Ty::List) {
            self.trap(76);
            return Ok(());
        }
        // BUG-029 (P3-PAIRPKG FIXUP): a may-Nil appended value is
        // compile-REJECTED. The old emission guarded it with a 76-thunk
        // — but the interpreter does NOT fault at the mutation:
        // check_scalar ADMITS Nil (canbe.rs is_scalar_value contains
        // Nil), the Nil element is silently STORED, and only a LATER read
        // of the hole faults (CastFail 71 — probed, doc 98 §E4). Faithful
        // storage would need per-element Nil tags in the frame block —
        // feature-level, ruled out; the B-family fail-fast (may-Nil
        // PACKLIST element, may-Nil pair value precedents) is the fix.
        if self.nil_val(value) {
            return errf!(
                "rv64 lower: opcode APPEND is outside the translatable kernel subset \
                 (the appended value may be Nil: the interpreter stores a Nil element \
                 and faults only when that element is later read, which a u64 frame \
                 block cannot represent — compute the value from definite assignments)"
            );
        }
        self.eval_int(value)?;
        // T19: a u128 element value is a (lo, hi) pair — two vstack
        // entries (the width replay guarantees the value width equals the
        // slot's element width)
        let vent = Self::entries_of_width(self.int_width(value));
        debug_assert_eq!(vent, if self.list_step(s) == 16 { 2 } else { 1 });
        let vd = self.vstack.len() - 1;
        let cell = self.slot_offset(s as u8);
        if arena_kind {
            let sc1 = self.asm.sc1;
            let sc2 = self.asm.sc2;
            let a0 = self.a0;
            let ew = self.lists.ew.get(s).copied().unwrap_or(64);
            self.asm.ld(sc1, S0, cell); // obj
            self.asm.ld(sc2, sc1, 0); // len
            let addr = self.push_vreg();
            let l40 = self.nil_thunk(40);
            self.asm.li(a0, CTR_LIST_CAP);
            self.asm.bgeu(sc2, a0, l40); // len >= 128 → 40
            self.ctr_elem_addr(addr, sc1, sc2, a0); // addr = obj+16+len*24
            self.asm.addi(sc2, sc2, 1); // len + 1
            self.asm.sd(sc2, sc1, 0); // the shared header word
            if self.list_step(s) == 16 {
                let vlo = loc_reg(self.vstack[vd - 1]);
                let vhi = loc_reg(self.vstack[vd]);
                self.ctr_store_elem(addr, ew, vlo, Some(vhi));
            } else {
                let v = loc_reg(self.vstack[vd]);
                self.ctr_store_elem(addr, ew, v, None);
            }
            self.pop_vreg(); // the address scratch
            for _ in 0..vent {
                self.pop_vreg();
            }
            return Ok(());
        }
        let base = self.list_data_off[s];
        debug_assert!(base >= 0, "list data block missing from the frame plan");
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        // B2 task 4: the block budget is LIST_BLOCK_MAX bytes and the
        // element stride is the slot's fixed width — the cap face (40)
        // compares len*step against the budget (for the u64 width that is
        // the historical len ≥ 128 ⟺ len*8 ≥ 1024; for the u128 width it
        // is len ≥ 64 ⟺ len*16 ≥ 1024)
        let step = self.list_step(s);
        let sh = step.trailing_zeros();
        self.asm.ld(sc1, S0, cell); // len
        self.asm.mv(sc2, sc1); // sc2 = len (survives the guard)
        if sh > 0 {
            self.asm.slli_(sc2, sc2, sh); // len*step
        }
        self.asm.li(sc1, LIST_BLOCK_MAX);
        let l40 = self.nil_thunk(40);
        self.asm.bgeu(sc2, sc1, l40); // len*step ≥ budget
        self.asm.addi(sc1, S0, base);
        self.asm.add(sc1, sc1, sc2); // addr = base + len*step
        if step == 16 {
            let vlo = loc_reg(self.vstack[vd - 1]);
            let vhi = loc_reg(self.vstack[vd]);
            self.list_store_pair(vlo, vhi, sc1, 0);
        } else {
            let v = loc_reg(self.vstack[vd]);
            self.list_store_inst(v, sc1, 0, step);
        }
        if sh > 0 {
            self.asm.srli_(sc2, sc2, sh); // len back
        }
        self.asm.addi(sc2, sc2, 1); // len + 1
        self.asm.sd(sc2, S0, cell);
        for _ in 0..vent {
            self.pop_vreg();
        }
        Ok(())
    }

    /// CLEAR(l): the interpreter's clear writes the shared header / the
    /// length shadow to zero; the frame contents stay (later appends
    /// overwrite). Shared by the statement form and the bind (T17).
    pub(crate) fn emit_clear(&mut self, s: usize) -> Ret<()> {
        if matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
            // P2: clear writes the shared object's header word — every
            // alias observes the empty list
            let cell = self.slot_offset(s as u8);
            let sc1 = self.asm.sc1;
            self.asm.ld(sc1, S0, cell);
            self.asm.sd(ZERO, sc1, 0);
            return Ok(());
        }
        if !matches!(self.lists.kind.get(s), Some(ListKind::Growable { .. })) {
            return errf!(
                "rv64 lower: opcode CLEAR is outside the translatable kernel subset \
                 (internal: clear on a non-growable storage class)"
            );
        }
        let cell = self.slot_offset(s as u8);
        self.asm.sd(ZERO, S0, cell);
        Ok(())
    }

    /// T17: replay the receiver-yielding mutation that produced a bound
    /// container value (`var m = append(l, v)` / insert / remove / merge /
    /// clear / item_set). The statement forms call the same helpers.
    pub(crate) fn emit_list_bind_mutation(&mut self, src: &dyn IRNode) -> Ret<()> {
        let op = Bytecode::try_from_u8(src.bytecode())
            .map_err(|e| format!("rv64 lower: {e}"))?;
        use Bytecode::*;
        match op {
            APPEND => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: append node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_append(s, &*n.suby)
            }
            CLEAR => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: clear node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_clear(s)
            }
            INSERT => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: insert node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_insert(s, &*n.suby, &*n.subz)
            }
            REMOVE => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: remove node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_remove(s, &*n.suby)
            }
            MERGE => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeDouble>()
                    .ok_or("rv64 lower: merge node shape unexpected")?;
                let rs = self.list_slot_of(&*n.subx, op)?;
                let ss = self.list_slot_of(&*n.suby, op)?;
                self.emit_merge(rs, ss)
            }
            ITEMSET => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: itemset node shape unexpected")?;
                let s = self.list_slot_of(&*n.subx, op)?;
                self.emit_itemset_write(&*n.suby, &*n.subz, s)
            }
            _ => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (internal: not a receiver-yielding list mutation)"
            ),
        }
    }

    /// T17: the PUT write into a list slot from a container-VALUE source —
    /// the `Rc` copy the interpreter's slot store performs (alias shares
    /// the object; CLONE is the only deep copy; a mutation result aliases
    /// its receiver; CHOOSE selects one descriptor; KEYS/VALUES traps 83).
    pub(crate) fn emit_put_list_bind(&mut self, idx: u8, src: &dyn IRNode, bind: ListBindSrc) -> Ret<()> {
        let sc1 = self.asm.sc1;
        let off = self.slot_offset(idx);
        match bind {
            ListBindSrc::Alias(s) => {
                self.ctr_load_obj(s, sc1);
                self.asm.sd(sc1, S0, off);
            }
            ListBindSrc::Clone(s) => {
                if !matches!(self.lists.kind.get(s), Some(ListKind::Arena { .. })) {
                    return errf!(
                        "rv64 lower: opcode CLONE is outside the translatable kernel \
                         subset (internal: clone receiver without an Arena class)"
                    );
                }
                let src_r = self.push_vreg();
                self.ctr_load_obj(s, src_r);
                let dst_r = self.push_vreg();
                self.ctr_clone_list(dst_r, src_r);
                self.asm.sd(dst_r, S0, off);
                self.pop_vreg();
                self.pop_vreg();
            }
            ListBindSrc::Mutation(s) => {
                self.emit_list_bind_mutation(src)?;
                self.ctr_load_obj(s, sc1);
                self.asm.sd(sc1, S0, off);
            }
            ListBindSrc::Choose(a, b) => {
                let n = src
                    .as_any()
                    .downcast_ref::<IRNodeTriple>()
                    .ok_or("rv64 lower: choose node shape unexpected")?;
                let l_no = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.branch_cond(&*n.subx, false, l_no)?;
                self.ctr_load_obj(a, sc1);
                self.asm.sd(sc1, S0, off);
                self.asm.jal_(ZERO, l_end);
                self.place(l_no);
                self.ctr_load_obj(b, sc1);
                self.asm.sd(sc1, S0, off);
                self.place(l_end);
            }
            ListBindSrc::Trap83 => {
                // KEYS/VALUES(list): the interpreter faults before the
                // store — the target is never written
                self.trap(83);
            }
        }
        Ok(())
    }

    /// T17: does this pack creation contain a nested list element? plan()
    /// admits only plain list GETs there (their evaluation is free of
    /// side effects and faults).
    pub(crate) fn pack_has_list_element(&self, arr: &IRNodeArray, n: u64) -> Ret<bool> {
        for e in arr.subs[..n as usize].iter() {
            if node_res_ty(&**e, self.slot)? == Ty::List {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// T17: evaluate (and drop) every SCALAR element of a pack creation in
    /// IR order — the interpreter pushes all elements before PACKLIST runs
    /// its checks, so their own faults fire first. List elements are plain
    /// GETs (no effects, plan-guaranteed).
    pub(crate) fn eval_pack_scalar_elems(&mut self, arr: &IRNodeArray, n: u64) -> Ret<()> {
        for e in arr.subs[..n as usize].iter() {
            if node_res_ty(&**e, self.slot)? == Ty::List {
                continue;
            }
            self.eval_int(&**e)?;
            for _ in 0..Self::entries_of_width(self.int_width(&**e)) {
                self.pop_vreg();
            }
        }
        Ok(())
    }

    /// T17: the nested-container pack face — every scalar element
    /// evaluates in order (its own faults fire first, exactly like the
    /// interpreter's element pushes), then pack_list's check_scalar
    /// faults CastBeValueFail(76).
    pub(crate) fn emit_pack_nested_trap(&mut self, arr: &IRNodeArray, n: u64) -> Ret<()> {
        self.eval_pack_scalar_elems(arr, n)?;
        self.trap(76);
        Ok(())
    }

    /// The list-creation PUT on an `Arena` slot: allocate the object, fill
    /// `init_n` tagged element cells from the creation expressions (in IR
    /// order — the interpreter's element faults fire there), store the
    /// descriptor into the slot cell. A fill over the 128-element capacity
    /// faults 40 up front (the interpreter's `take_items_from_ops` cap
    /// face; a cap face, so the kernel may also have rejected earlier).
    pub(crate) fn emit_list_create_arena(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let s = slot_idx as usize;
        match self.lists.kind.get(s).copied() {
            Some(ListKind::Arena { .. }) => {}
            _ => {
                return errf!(
                    "rv64 lower: opcode PUT is outside the translatable kernel subset \
                     (internal: arena creation without an Arena storage class)"
                )
            }
        }
        let (arr, n) = match list_create_src(src) {
            Some(ListCreate::NewList) => (None, 0u64),
            Some(ListCreate::PackList { arr, n }) => (Some(arr), n),
            None => {
                return errf!(
                    "rv64 lower: opcode PUT is outside the translatable kernel subset \
                     (unsupported list creation shape)"
                )
            }
        };
        // The slot's Arena class carries the FIRST creation's fill count,
        // but a re-bind may create the slot again with a different shape
        // (`var l = [1,2]; l = [9]`). The emitted object is always shaped
        // by THIS creation site's source, so n is authoritative.
        if n > CTR_LIST_CAP {
            // the interpreter's pack cap face (take_items_from_ops → 40).
            // The elements still evaluate first (they are pushed before
            // PACKLIST runs), so their own faults win.
            if let Some(arr) = arr {
                self.eval_pack_scalar_elems(arr, n)?;
            }
            let l40 = self.nil_thunk(40);
            self.asm.jal_(ZERO, l40);
            return Ok(());
        }
        if let Some(arr) = arr {
            if self.pack_has_list_element(arr, n)? {
                return self.emit_pack_nested_trap(arr, n);
            }
        }
        let ew = self.lists.ew.get(s).copied().unwrap_or(64);
        // evaluate every element FIRST? No — in IR order, storing as we go
        // (the interpreter's faults fire in element order; the alloc itself
        // cannot fault on element values, only the arena budget face which
        // the interpreter does not have — a kernel budget face).
        // The object base rides a fresh pool register across the element
        // evaluations (element expressions may use the full pool).
        let base = self.push_vreg();
        self.ctr_list_alloc(base, CTR_LIST_CAP, n);
        if let Some(arr) = arr {
            for (i, e) in arr.subs[..n as usize].iter().enumerate() {
                // the may-Nil element face keeps the B-family reject
                if self.nil_val(&**e) {
                    return errf!(
                        "rv64 lower: opcode PACKLIST is outside the translatable kernel \
                         subset (element {i} may be Nil; bind list elements unconditionally \
                         first)"
                    );
                }
                self.eval_int(&**e)?;
                let sc1 = self.asm.sc1;
                let step = step_of(Some(ew)) as i32;
                let off = CTR_LIST_HDR as i32 + (i as i32) * CTR_ELEM_BYTES as i32;
                if step == 16 {
                    let hi = self.pop_reg();
                    let lo = self.pop_reg();
                    self.asm.addi(sc1, base, off);
                    self.ctr_store_elem(sc1, ew, lo, Some(hi));
                } else {
                    let v = self.pop_reg();
                    self.asm.addi(sc1, base, off);
                    self.ctr_store_elem(sc1, ew, v, None);
                }
            }
        }
        // the slot cell := the descriptor (the alias/re-bind write target)
        let cell = self.slot_offset(slot_idx);
        self.asm.sd(base, S0, cell);
        self.pop_vreg();
        Ok(())
    }

    /// The (single) list-creation PUT: fill the frame block, or register
    /// the rodata table, or nothing at all (empty static lists). P2: an
    /// Arena slot routes to the descriptor-model creation (fresh object).
    pub(crate) fn emit_list_create(&mut self, slot_idx: u8, src: &dyn IRNode) -> Ret<()> {
        let s = slot_idx as usize;
        let kind = self
            .lists
            .kind
            .get(s)
            .copied()
            .ok_or("rv64 lower: list slot without a storage plan")?;
        if matches!(kind, ListKind::Arena { .. }) {
            return self.emit_list_create_arena(slot_idx, src);
        }
        match list_create_src(src) {
            Some(ListCreate::NewList) => {
                match kind {
                    ListKind::Empty => Ok(()),
                    ListKind::Growable { .. } => {
                        // len shadow ← 0 (always an explicit store: the
                        // slot cell is the only runtime length state)
                        let cell = self.slot_offset(slot_idx);
                        self.asm.sd(ZERO, S0, cell);
                        Ok(())
                    }
                    _ => errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (internal: NEWLIST source with storage class {kind:?})"
                    ),
                }
            }
            Some(ListCreate::PackList { arr, n }) => {
                // T17: a nested container element is the interpreter's
                // runtime 76 face — every (scalar) element evaluates in
                // order, then pack_list's check_scalar faults.
                if self.pack_has_list_element(arr, n)? {
                    return self.emit_pack_nested_trap(arr, n);
                }
                match kind {
                    ListKind::Rodata { .. } => {
                        // Read-only table: the literals cannot fault, so
                        // the creation site emits nothing at all.
                        self.list_rodata_lbl(s)?;
                        Ok(())
                    }
                    ListKind::FrameStatic { .. } | ListKind::Growable { .. } => {
                        if let ListKind::Growable { init_n } = kind {
                            if init_n != n {
                                return errf!(
                                    "rv64 lower: opcode PUT is outside the translatable kernel \
                                     subset (internal: PACKLIST count {n} vs plan {init_n})"
                                );
                            }
                        }
                        let base = self.list_data_off[s];
                        debug_assert!(base >= 0, "list data block missing from the frame plan");
                        for (i, e) in arr.subs[..n as usize].iter().enumerate() {
                            // may-Nil elements are a B-family compile reject:
                            // the interpreter's deferred check_scalar(76)
                            // cannot be ordered faithfully without keeping
                            // every element's tag live past the pool limit
                            if self.nil_val(&**e) {
                                return errf!(
                                    "rv64 lower: opcode PACKLIST is outside the translatable \
                                     kernel subset (element {i} may be Nil; bind list elements \
                                     unconditionally first)"
                                );
                            }
                            self.eval_int(&**e)?;
                            // T19: a u128 element is a (lo, hi) pair — pop
                            // two entries and store both halves
                            let step = step_of(self.lists.ew.get(s).copied()) as i32;
                            if step == 16 {
                                let hi = self.pop_reg();
                                let lo = self.pop_reg();
                                self.list_store_pair(lo, hi, S0, base + (i as i32) * step);
                            } else {
                                let v = self.pop_reg();
                                self.list_store_inst(v, S0, base + (i as i32) * step, step as u32);
                            }
                        }
                        if let ListKind::Growable { .. } = kind {
                            let cell = self.slot_offset(slot_idx);
                            let sc1 = self.asm.sc1;
                            self.asm.addi(sc1, ZERO, n as i32);
                            self.asm.sd(sc1, S0, cell);
                        }
                        Ok(())
                    }
                    _ => errf!(
                        "rv64 lower: opcode PUT is outside the translatable kernel subset \
                         (internal: PACKLIST source with storage class {kind:?})"
                    ),
                }
            }
            None => errf!(
                "rv64 lower: opcode PUT is outside the translatable kernel subset \
                 (unsupported list creation shape)"
            ),
        }
    }
}
