use super::super::*;

impl<'a> Lower<'a> {

    /// `slot OP= value` — the compound-assignment lowering shared by the
    /// `XOP` opcode (slots 0–63) and the T8 high-slot local compound
    /// fragment `IRLIST[value, GET idx; SWAP; op; PUT idx]` (slots ≥ 64);
    /// the two shapes have identical semantics (RHS-first, then the old
    /// value's convert-71 guard, then the checked arithmetic and the
    /// write-back at the slot/rhs common width `w`). `xop_old_nil` is the
    /// may-Nil bit of the OLD slot value from the pre-pass (the occ entry
    /// of the XOP node, or of the fragment IRLIST node).
    pub(crate) fn emit_xop_stmt(
        &mut self,
        idx: u8,
        lxop: LxOp,
        value: &dyn IRNode,
        w: u32,
        xop_old_nil: bool,
    ) -> Ret<()> {
        // constant rhs: the guards/arithmetic absorb the constant
        // (`i += 1` → c.addi + c.beqz; `x /= 16` → c.srli …)
        let kop = lxop_inst(lxop);
        let rhs_nil = self.nil_val(value);
        // task 3: a u128 slot stages its old (lo, hi) pair in pool
        // registers BEFORE the rhs evaluates — the pair then sits at
        // vstack positions 0/1 and the rhs at 2/3, so the probe pass
        // sizes the pool for exactly this shape and no scratch
        // register can alias an operand (pool and scratches are
        // disjoint in every mode). The OLD-value flag guard keeps the
        // interpreter's order regardless: rhs faults fire first.
        let staged128 = w == 128;
        let (slo, shi) = if staged128 {
            let off0 = self.slot_offset(idx);
            let lo = self.push_vreg();
            let hi = self.push_vreg();
            self.asm.ld(lo, S0, off0);
            self.asm.ld(hi, S0, off0 + 8);
            (Some(lo), Some(hi))
        } else {
            (None, None)
        };
        if let Some(k) = int_literal(value).filter(|_| w < 128) {
            if Self::arith_k_handles(kop, k) {
                if xop_old_nil {
                    self.nil_guard_flag(idx, 71);
                }
                if let Some(r) = self.bank_int(idx) {
                    self.kill_fwd_reg(r);
                    if self.emit_arith_k(kop, r, k, w) {
                        // narrow-PUT face: compound writes only grow the shadow
                        // (C13: the checked arithmetic above stays — fault
                        // face — but its shadow grow is dead when no read of
                        // the slot remains; store_is_dead carries the
                        // loop-carried / backedge exemptions)
                        if !self.store_is_dead(idx) {
                            self.shadow_grow(idx, self.int_width(value));
                        }
                        return Ok(());
                    }
                    unreachable!("arith_k_handles said yes");
                }
                let off = self.slot_offset(idx);
                let sc1 = self.asm.sc1;
                self.asm.ld(sc1, S0, off);
                if !self.emit_arith_k(kop, sc1, k, w) {
                    unreachable!("arith_k_handles said yes");
                }
                if self.store_is_dead(idx) {
                    self.fwd_int[idx as usize] = None;
                } else {
                    self.asm.sd(sc1, S0, off);
                    // narrow-PUT face: compound writes only grow the shadow
                    self.shadow_grow(idx, self.int_width(value));
                    // BUG-018: the addi-path store went to memory
                    // through the dedicated scratch (sc1 is never a
                    // pool home), so there is nothing to cache — but
                    // a forward record left by an EARLIER store to
                    // this slot must die with it, or the next
                    // empty-stack GET reads the stale register
                    // (`a += 1u64` immediately read returned the
                    // pre-XOP value in Im/Imc).
                    self.fwd_int[idx as usize] = None;
                }
                return Ok(());
            }
        }
        // T7: constant-rhs absorption on a u128 slot — the staged
        // old pair updates in place, no rhs evaluation at all (the
        // `x += 1` shape on a 16-byte cell; the rhs literal's own
        // width is irrelevant, the common width is the slot's 128).
        if staged128 {
            if let Some(k) = int_literal(value) {
                if self.slot_w.get(idx as usize).copied().flatten() != Some(128) {
                    return errf!(
                        "rv64 lower: opcode XOP is outside the translatable kernel \
                         subset (a u128 compound result cannot fit a narrower slot — \
                         keep the slot's width uniform)"
                    );
                }
                if xop_old_nil {
                    self.nil_guard_flag(idx, 71);
                }
                self.emit_xop128_k(kop, idx, k, slo.unwrap(), shi.unwrap())?;
                let _ = self.pop_vreg(); // staged hi
                let _ = self.pop_vreg(); // staged lo
                return Ok(());
            }
        }
        self.eval_int(value)?;
        if xop_old_nil {
            self.nil_guard_flag(idx, 71);
        }
        if rhs_nil {
            // the rhs tag sits on its lowest entry (the high half of
            // a u128 pair carries no tag byte); a narrower rhs keeps
            // a single entry (T5/T7: the interpreter zero-extends it)
            let low = if self.int_width(value) == Some(128) {
                2
            } else {
                1
            };
            self.nil_guard_tag(self.vstack.len() - low, 71);
        }
        if staged128 {
            // task 3 + T5: compound arithmetic on a u128 slot. The
            // slot keeps its 16-byte frame cell (u128 slots are never
            // banked) and the single-register forwarding does not
            // apply. A narrower rhs zero-extends exactly (the
            // interpreter's arithmetic_args2 widens both sides —
            // the common width is the slot's 128).
            if self.slot_w.get(idx as usize).copied().flatten() != Some(128) {
                return errf!(
                    "rv64 lower: opcode XOP is outside the translatable kernel subset \
                     (a u128 compound result cannot fit a narrower slot — keep the \
                     slot's width uniform)"
                );
            }
            let slo = slo.unwrap();
            let shi = shi.unwrap();
            let off = self.slot_offset(idx);
            let (rlo, rhi) = self.pop_u128_pair(self.int_width(value));
            match lxop_inst(lxop) {
                Bytecode::DIV => {
                    let s = self.divmod_save_off;
                    debug_assert!(s >= 0, "divmod call without a reserved save area");
                    self.asm.sd(slo, S0, s);
                    self.asm.sd(shi, S0, s + 8);
                    self.asm.sd(rlo, S0, s + 16);
                    self.asm.sd(rhi, S0, s + 24);
                    let lbl = self.u128_divmod_label();
                    self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
                    self.asm.ld(slo, S0, s + 32);
                    self.asm.ld(shi, S0, s + 40);
                    self.asm.sd(slo, S0, off);
                    self.asm.sd(shi, S0, off + 8);
                    self.fwd_int[idx as usize] = None;
                }
                Bytecode::MOD => {
                    return errf!(
                        "rv64 lower: opcode XOP is outside the translatable kernel \
                         subset (compound % on a u128 slot — write `x = x % y` instead)"
                    );
                }
                op2 => {
                    match op2 {
                        Bytecode::ADD => self.emit_u128_add(slo, shi, rlo, rhi),
                        Bytecode::SUB => self.emit_u128_sub(slo, shi, rlo, rhi),
                        Bytecode::MUL => self.emit_u128_mul(slo, shi, rlo, rhi),
                        _ => self.emit_u128_bitop(op2, slo, shi, rlo, rhi),
                    }
                    self.asm.sd(slo, S0, off);
                    self.asm.sd(shi, S0, off + 8);
                    self.fwd_int[idx as usize] = None;
                }
            }
            let _ = self.pop_vreg(); // staged hi
            let _ = self.pop_vreg(); // staged lo
            return Ok(());
        }
        if let Some(rw) = self.bank_int(idx) {
            // promoted: the arithmetic lands on the WRITE home; the
            // old value may live in the previous segment's register
            // (XOP is a segment boundary) — copy it across first.
            // The rhs must not alias `rw` (it may be a Ref to the
            // same slot: e.g. `i += i` — materialize it first).
            let rr = self.bank_int_read(idx).unwrap_or(rw);
            self.kill_fwd_reg(rw);
            let rv = match self.pop_vreg() {
                VLoc::Pool(v) => v,
                VLoc::Ref(v) if v == rw => {
                    // the rhs aliases the write home: materialize it
                    // into a pool register. The push/pop pair keeps
                    // the vstack balanced (BUG-013: the push without
                    // the pop leaked one entry per self-ref XOP and
                    // rejected the whole ImcRemap body at function
                    // end); the value survives in `t`, consumed by
                    // the emit_arith below before any new push.
                    let t = self.push_vreg();
                    self.asm.mv(t, v);
                    let _ = self.pop_vreg();
                    t
                }
                VLoc::Ref(v) => v,
            };
            if rr != rw {
                self.asm.mv(rw, rr);
            }
            self.emit_arith(lxop_inst(lxop), rw, rv, w)?;
            // narrow-PUT face: compound writes only grow the shadow
            // (C13: the arithmetic's fault face stays in full; the shadow
            // grow dies with a proven-dead store — same store_is_dead basis
            // as the frame arm below, exemptions included)
            if !self.store_is_dead(idx) {
                self.shadow_grow(idx, self.int_width(value));
            }
            return Ok(());
        }
        let rv = self.pop_reg();
        let off = self.slot_offset(idx);
        // Forwarded old value: a live fwd entry names a pool register at
        // or below the current depth (E2 rule, fwd_reg_reissuable — XOP
        // is statement-level, so the stack is empty here and this
        // degenerates to the historical bottom-register rule). Bank-register
        // fwd entries are excluded: the arithmetic destination must never be
        // another slot's bank home.
        let fr = self
            .fwd_int
            .get(idx as usize)
            .copied()
            .flatten()
            .filter(|r| {
                self.fwd_enabled()
                    && self.fwd_slot_ok(idx as usize)
                    && self.fwd_reg_reissuable(*r)
            });
        // at statement level (empty pool) the first pool registers
        // are free and window-resident, so the slot traffic
        // compresses (c.ld/c.sd) — pick one the rhs is not using;
        // nested inside an expression the pool is live, so fall
        // back to the dedicated scratch.
        let sc1 = match fr {
            Some(r) => r,
            None if self.vstack.is_empty() => {
                if rv != self.vregs[0] {
                    self.vregs[0]
                } else {
                    self.vregs[1]
                }
            }
            None => self.asm.sc1,
        };
        // E2 companion kill: the arithmetic below OVERWRITES sc1 (either the
        // loaded old value or, on an fr hit, the forwarded one in place), so
        // every fwd entry naming a pool register here must die — including
        // entries of OTHER slots that name the same register (they were
        // valid only while the old value lived there). Before the read-side
        // generalization such entries were inert (only vregs[0] was ever
        // re-issued, and the fr filter sees it first); with `r == vregs[j],
        // j <= depth` reads they become live again, so the kill discipline
        // must cover this overwrite source too. A no-op for the dedicated
        // scratch (no entry can name it).
        self.kill_fwd_reg(sc1);
        if fr.is_none() {
            self.asm.ld(sc1, S0, off);
        }
        self.emit_arith(lxop_inst(lxop), sc1, rv, w)?;
        if self.store_is_dead(idx) {
            // dead write: the faulting arithmetic above is still
            // fully emitted; only the store disappears
            self.fwd_int[idx as usize] = None;
        } else {
            self.asm.sd(sc1, S0, off);
            // narrow-PUT face: compound writes only grow the shadow (the
            // arithmetic register is dead here; the shadow helpers use
            // their own scratch)
            self.shadow_grow(idx, self.int_width(value));
            if self.vregs.contains(&sc1) {
                // sc1 is a pool register — cache the value
                self.fwd_int[idx as usize] = Some(sc1);
            } else {
                self.fwd_int[idx as usize] = None;
            }
        }
        Ok(())
    }

    /// T15: the entry copy of a rebound `data` param — `S4/S5` hold the ABI
    /// input (ptr, len); copy it into the arena and bind slot 0's home (a
    /// frame cell, or the bank pair when the web promoted it) to the copy.
    /// Runs at the prelude (the body's first statement), so every read of
    /// slot 0 sees the input until the first user store — the interpreter's
    /// slot-0 rebindings and buf_set only ever mutate freshly extracted
    /// values, so the copy is semantics-faithful and side-effect free for
    /// the caller. The arena-exhaustion face (71) is the kernel's memory
    /// budget, not a semantic code.
    pub(crate) fn emit_input_local_copy(&mut self) -> Ret<()> {
        self.emit_arena_check(S5);
        match self.bank_bytes_write(0) {
            Some((bp, bl)) => {
                self.emit_memcpy(S4, S5);
                // emit_memcpy bumped S1 by exactly S5: ptr = S1 - S5
                // (asm.r takes (rs2, rs1): sub rd, rs1, rs2)
                self.asm.r(0x20, S5, S1, 0, bp, 0b0110011); // sub bp, S1, S5
                self.asm.mv(bl, S5);
                // B9: the entry copy IS the arena top (bp + bl == S1); the
                // unbanked arm below keeps its descriptor in scratch/ABI
                // registers — no pool pair to attach the fact to, and no
                // fact survives (the memcpy's kill stands)
                self.cat_top_establish(bp, bl);
            }
            None => {
                let off = self.slot_offset(0);
                self.emit_memcpy(S4, S5);
                let sc1 = self.asm.sc1;
                self.asm.r(0x20, S5, S1, 0, sc1, 0b0110011); // sub sc1, S1, S5
                self.asm.sd(sc1, S0, off);
                self.asm.sd(S5, S0, off + 8);
                // no forwarding: the descriptor lives in scratch/ABI
                // registers, not a pool pair
                self.fwd_bytes[0] = None;
            }
        }
        Ok(())
    }


    /* ---- checked arithmetic: `d = d OP rhs` ---- */

    /// Task-1 A3: range check `d < 2^w` — the interpreter computes an
    /// overflow-face op at the operands' common width w, so a result ≥ 2^w
    /// is Arithmetic 90. Both inputs are < 2^w (the width invariant), so
    /// for w < 64 the u64 operation itself cannot wrap and THIS check is
    /// exactly the interpreter's checked op.
    pub(crate) fn emit_range_check(&mut self, d: u8, w: u32) {
        debug_assert!(matches!(w, 8 | 16 | 32));
        self.asm.li(self.a1, 1u64 << w);
        self.asm.bgeu(d, self.a1, self.fault_arith);
    }

    /// Task-1 A3: truncate `d` to its low w bits — the interpreter's
    /// narrow-width result (Rust `<<` on a fixed-width int discards the
    /// shifted-out bits, which is the value `checked_shl` returns).
    pub(crate) fn emit_truncate(&mut self, d: u8, w: u32) {
        match w {
            8 => self.asm.andi(d, d, 0xFF),
            16 => {
                self.asm.slli_(d, d, 48);
                self.asm.srli_(d, d, 48);
            }
            32 => {
                self.asm.slli_(d, d, 32);
                self.asm.srli_(d, d, 32);
            }
            _ => {}
        }
    }

    /// B2 task 4: the width oracle bound to THIS body's tables — Int slots
    /// read the joined slot width, List slots read their fixed element
    /// width (see [`slot_width_lookup`]).
    pub(crate) fn int_width(&self, node: &dyn IRNode) -> Option<u32> {
        node_int_width(node, self.slot, &self.slot_w, &self.lists.ew)
    }

    /// Static arithmetic width of a node for the checked-emit forms: real
    /// widths pass through; bool-ish (`None`) and pure-bool (`Some(1)`)
    /// fall back to the legacy uniform u64 (the interpreter faults 71 on
    /// bool arithmetic — the pinned D5BoolUint deviation family).
    pub(crate) fn arith_w(&self, node: &dyn IRNode) -> u32 {
        match self.int_width(node) {
            Some(w) if w >= 8 => w,
            _ => 64,
        }
    }

    /* ---- task 3: u128 (lo, hi) u64 pairs ---- */

    /// How many vstack entries a value of static uint width `w` occupies:
    /// a u128 is a (lo, hi) register pair (two entries), everything else is
    /// one. Bytes values are two entries at every call site already.
    pub(crate) fn entries_of_width(w: Option<u32>) -> usize {
        if w == Some(128) { 2 } else { 1 }
    }

    /// Entry count of an already type-checked int/bytes value node.
    pub(crate) fn value_entries(&self, node: &dyn IRNode) -> Ret<usize> {
        let ty = node_res_ty(node, self.slot)?;
        Ok(match ty {
            Ty::Bytes | Ty::Address => 2,
            _ => Self::entries_of_width(self.int_width(node)),
        })
    }


    /// Size-optimized forms: fault paths are one `jal` to a shared thunk, so
    /// the straight-line cost is the minimum the semantics allow.
    ///
    /// `w` is the static result width (task-1 A3): 64 keeps the uniform
    /// u64-checked forms; 8/16/32 emit the interpreter's narrow-width
    /// semantics (add/mul range-check the result, sub's underflow check is
    /// width-exact already, shifts check the amount against w and shl
    /// truncates its result to w bits).
    pub(crate) fn emit_arith(&mut self, op: Bytecode, d: u8, rhs: u8, w: u32) -> Ret<()> {
        use Bytecode::*;
        match op {
            ADD => {
                if w < 64 {
                    // sum < 2^(w+1) <= 2^64: no u64 wrap, range-check only
                    self.asm.add(d, d, rhs);
                    self.emit_range_check(d, w);
                } else {
                    // overflow iff result < rhs (unsigned)
                    self.asm.add(d, d, rhs);
                    self.asm.bltu(d, rhs, self.fault_arith);
                }
            }
            SUB => {
                // underflow iff old d < rhs — exact at every width (both
                // sides < 2^w), check BEFORE the sub
                self.asm.bltu(d, rhs, self.fault_arith);
                self.asm.sub(d, d, rhs);
            }
            MUL => {
                if w < 64 {
                    // product < 2^2w <= 2^64: no u64 wrap, range-check only
                    self.asm.r(1, rhs, d, 0, d, 0b0110011); // mul
                    self.emit_range_check(d, w);
                } else {
                    self.asm.r(1, rhs, d, 3, self.a1, 0b0110011); // mulhu
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(1, rhs, d, 0, d, 0b0110011); // mul
                }
            }
            DIV => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
            }
            DIVEXACT => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, self.a1, 0b0110011); // remu
                self.asm.bne(self.a1, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
            }
            DIVUP => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, self.a1, 0b0110011); // remu (dividend in d)
                self.asm.r(1, rhs, d, 5, d, 0b0110011); // divu
                let join = self.asm.new_label();
                self.asm.beq(self.a1, ZERO, join);
                self.asm.addi(d, d, 1);
                self.place(join);
            }
            MOD => {
                self.asm.beq(rhs, ZERO, self.fault_arith);
                self.asm.r(1, rhs, d, 7, d, 0b0110011); // remu
            }
            MAX => self.emit_max(d, rhs),
            MIN => self.emit_min(d, rhs),
            ABSDIFF => {
                let l_b = self.asm.new_label();
                let l_e = self.asm.new_label();
                self.asm.bgeu(d, rhs, l_b);
                self.asm.r(0x20, d, rhs, 0, d, 0b0110011); // d = rhs - d
                self.asm.jal_(ZERO, l_e);
                self.place(l_b);
                self.asm.r(0x20, rhs, d, 0, d, 0b0110011); // d = d - rhs
                self.place(l_e);
            }
            BXOR => self.asm.xor(d, d, rhs),
            BOR => self.asm.or_(d, d, rhs),
            BAND => self.asm.and_(d, d, rhs),
            BSHL => {
                // Interpreter `bit_shl` = Rust `checked_shl`: the shift
                // AMOUNT must be < the width (Arithmetic 90 otherwise);
                // the value side truncates silently (`u64::MAX << 1` is
                // Ok(0xFF..FE), not a fault). The historical `andi 63`
                // masked the amount and turned `1u64 << 64` into `<< 0`.
                if w < 64 {
                    self.asm.addi(self.a1, ZERO, w as i32);
                    self.asm.bgeu(rhs, self.a1, self.fault_arith);
                    self.asm.r(0, rhs, d, 1, d, 0b0110011); // sll
                    self.emit_truncate(d, w);
                } else {
                    self.asm.srli_(self.a1, rhs, 6);
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(0, rhs, d, 1, d, 0b0110011); // sll
                }
            }
            BSHR => {
                // same checked-amount rule (`checked_shr`); the value side
                // only loses low bits, so no truncation is needed
                if w < 64 {
                    self.asm.addi(self.a1, ZERO, w as i32);
                    self.asm.bgeu(rhs, self.a1, self.fault_arith);
                    self.asm.r(0, rhs, d, 5, d, 0b0110011); // srl
                } else {
                    self.asm.srli_(self.a1, rhs, 6);
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(0, rhs, d, 5, d, 0b0110011); // srl
                }
            }
            EQ => {
                self.asm.xor(d, d, rhs); // xor (c.xor in the window)
                self.asm.i(1, d, 3, d, 0b0010011); // sltiu d, d, 1
            }
            NEQ => {
                self.asm.xor(d, d, rhs); // xor (c.xor in the window)
                self.asm.r(0, d, ZERO, 3, d, 0b0110011); // sltu d, x0, d
            }
            LT => {
                self.asm.r(0, rhs, d, 3, d, 0b0110011); // sltu d, d, rhs
            }
            GT => {
                self.asm.r(0, d, rhs, 3, d, 0b0110011); // sltu d, rhs, d
            }
            LE => {
                // d <= rhs  ⇔  !(rhs < d): sltu sc1, rhs, d — rs1=rhs,
                // rs2=d (the r() signature lists rs2 first). Two past bugs
                // lived here: xori funct3 written 12 (bit-spill into rs1)
                // and this operand swap, which made LE behave as GE. Both
                // passed naive tests by value coincidence; the HIP5 scorer
                // caught them (see module docs).
                let sc1 = self.asm.sc1;
                self.asm.r(0, d, rhs, 3, sc1, 0b0110011); // sltu sc1, rhs, d
                self.asm.i(1, sc1, 4, d, 0b0010011); // xori d, sc1, 1
            }
            GE => {
                // d >= rhs  ⇔  !(d < rhs)
                self.asm.r(0, rhs, d, 3, d, 0b0110011); // sltu d, d, rhs
                self.asm.i(1, d, 4, d, 0b0010011); // xori d, d, 1
            }
            other => {
                // GT-c: "prototype" wording retired (doc 66 §1.4) — the
                // kernel subset is a product contract, not a prototype
                // limitation; this arm is plan-should-have-rejected
                // territory, so it is an internal report, not a user error
                return errf!(
                    "rv64 lower: internal: arithmetic op {other:?} has no lowering (opcode \
                     outside the translatable kernel subset; plan should have rejected it)"
                )
            }
        }
        Ok(())
    }

    /// E3: swapped-operand form for a commutative binary arithmetic —
    /// (MUL w == 64 never reaches here: pop2_dest keeps the mv so the A19
    /// mul-check merge keeps its copy-feeding-mulhu shape)
    /// `d = lhs OP rhs` where the left value still lives in `lhs` (its
    /// bank home; pop2_dest skipped the materializing `mv`) and `d` is the
    /// fresh destination pool register. The fault faces are byte-for-byte
    /// the ones `emit_arith` emits: the w=64 ADD overflow check and the
    /// MUL mulhu guard compare against the ORIGINAL rhs, which stays the
    /// `rhs` argument here, and the narrower widths range-check the
    /// destination — only the add/mul operand wiring is mirrored, which
    /// commutativity makes value-identical. EQ/NEQ keep the xor+sltiu /
    /// xor+sltu chain (xor is commutative; the second step reads d only).
    pub(crate) fn emit_arith_commut(
        &mut self,
        op: Bytecode,
        d: u8,
        lhs: u8,
        rhs: u8,
        w: u32,
    ) -> Ret<()> {
        use Bytecode::*;
        match op {
            ADD => {
                if w < 64 {
                    // sum < 2^(w+1) <= 2^64: no u64 wrap, range-check only
                    self.asm.r(0, rhs, lhs, 0, d, 0b0110011); // add d, lhs, rhs
                    self.emit_range_check(d, w);
                } else {
                    // overflow iff result < rhs (unsigned) — the ORIGINAL
                    // rhs (lhs is the swapped-in left value)
                    self.asm.r(0, rhs, lhs, 0, d, 0b0110011);
                    self.asm.bltu(d, rhs, self.fault_arith);
                }
            }
            MUL => {
                if w < 64 {
                    // product < 2^2w <= 2^64: no u64 wrap, range-check only
                    self.asm.r(1, rhs, lhs, 0, d, 0b0110011); // mul
                    self.emit_range_check(d, w);
                } else {
                    self.asm.r(1, rhs, lhs, 3, self.a1, 0b0110011); // mulhu (commutative)
                    self.asm.bne(self.a1, ZERO, self.fault_arith);
                    self.asm.r(1, rhs, lhs, 0, d, 0b0110011); // mul
                }
            }
            BXOR => self.asm.xor(d, lhs, rhs),
            BOR | OR => self.asm.or_(d, lhs, rhs),
            BAND | AND => self.asm.and_(d, lhs, rhs),
            EQ => {
                self.asm.xor(d, lhs, rhs); // xor (rd != rs1: full width)
                self.asm.i(1, d, 3, d, 0b0010011); // sltiu d, d, 1
            }
            NEQ => {
                self.asm.xor(d, lhs, rhs);
                self.asm.r(0, d, ZERO, 3, d, 0b0110011); // sltu d, x0, d
            }
            other => {
                return errf!(
                    "rv64 lower: internal: swapped-destination form requested for \
                     non-commutative op {other:?}"
                )
            }
        }
        Ok(())
    }

    /// Whether `emit_arith_k` has a constant-specialized form for
    /// `d = d OP k` (checked before the operand is consumed).
    pub(crate) fn arith_k_handles(op: Bytecode, k: u64) -> bool {
        use Bytecode::*;
        let pow2 = k.count_ones() == 1 && k != 0;
        match op {
            ADD | SUB => k <= 1,
            MUL => k <= 1 || pow2,
            DIV => k == 1 || pow2 || k >= 2, // any const: no zero-guard
            // A7 (survey arith.md): DIVUP joins DIVEXACT's constant forms —
            // pow2 rides the srli+andi+beq+addi 4-slot shape, any other
            // k >= 2 keeps the divu pair minus the dead zero guard (a
            // constant divisor is nonzero by proof). k == 0 stays
            // unhandled: the zero-divisor 90 face needs the general
            // path's runtime guard.
            DIVEXACT | DIVUP => k >= 1,
            MOD => k == 1 || pow2 || k >= 2,
            // any constant amount: k < 64 shifts, k >= 64 faults
            // unconditionally (checked shift: amount >= width)
            BSHL | BSHR => true,
            BAND => k <= 2047,
            _ => false,
        }
    }

    /// A non-negative constant that fits `andi`'s 12-bit signed immediate.
    /// Anything larger MUST ride a register (`li` + `and_`) — a cast through
    /// `as i32` silently truncates (2^32-1 became -1, an identity mask).
    pub(crate) fn i12_mask(v: u64) -> Option<i32> {
        (v <= 2047).then_some(v as i32)
    }

    /// `d = d OP k` with a constant rhs — the checked guards absorb what
    /// the constant proves (nonzero divisors, overflow-impossible forms)
    /// and the arithmetic folds into immediate/compressed shapes.
    /// Returns false when unhandled (caller falls back to the reg form).
    /// `w` is the static result width (task-1 A3; see `emit_arith`).
    pub(crate) fn emit_arith_k(&mut self, op: Bytecode, d: u8, k: u64, w: u32) -> bool {
        use Bytecode::*;
        let pow2j = |k: u64| -> Option<u32> {
            (k != 0 && k.count_ones() == 1).then(|| k.trailing_zeros())
        };
        match op {
            ADD if k == 0 => {}
            ADD if k == 1 => {
                if w < 64 {
                    // d < 2^w: no u64 wrap, range-check only
                    self.asm.addi(d, d, 1);
                    self.emit_range_check(d, w);
                } else {
                    // +1 overflows iff the result wrapped to 0
                    self.asm.addi(d, d, 1);
                    self.asm.beq(d, ZERO, self.fault_arith);
                }
            }
            SUB if k == 0 => {}
            SUB if k == 1 => {
                // -1 underflows iff d was already 0 — exact at every width
                self.asm.beq(d, ZERO, self.fault_arith);
                self.asm.addi(d, d, -1);
            }
            MUL if k == 0 => self.asm.addi(d, ZERO, 0),
            MUL if k == 1 => {}
            MUL if pow2j(k).is_some() => {
                // x * 2^j: at width w this overflows iff x >= 2^(w-j)
                // (the u64 form is the w = 64 instance; w > j always
                // holds because the literal's own width is <= w)
                let j = pow2j(k).unwrap();
                let t = self.push_vreg();
                self.asm.srli_(t, d, w - j);
                self.asm.bne(t, ZERO, self.fault_arith);
                let _ = self.pop_vreg();
                self.asm.slli_(d, d, j);
            }
            DIV if k == 1 => {}
            DIV if pow2j(k).is_some() => {
                // const nonzero divisor: no zero-guard; unsigned / 2^j = srli
                self.asm.srli_(d, d, pow2j(k).unwrap());
            }
            DIV if k >= 2 => {
                // const nonzero: the beq rhs, ZERO guard is dead
                let t = self.push_vreg();
                // A11 (survey arith.md): k > 0xFFFF rides the literal pool
                // (one ld) when a pool base is active; smaller constants
                // fall back to the plain li inside li_k — never worse.
                self.li_k(t, k);
                self.asm.r(1, t, d, 5, d, 0b0110011); // divu d, d, t
                let _ = self.pop_vreg();
            }
            DIVEXACT if k == 1 => {}
            DIVEXACT if pow2j(k).is_some() => {
                let j = pow2j(k).unwrap();
                let t = self.push_vreg();
                // three-way fix: (k-1) only rides `andi` when it fits the
                // 12-bit signed immediate — a larger mask truncated through
                // `as i32` silently became -1 (identity) or a wrong mask,
                // misjudging the exact-division check (x = 2^32 with
                // k = 2^32 would fault 90 instead of dividing)
                match Self::i12_mask(k - 1) {
                    Some(m) => self.asm.andi(t, d, m),
                    None => {
                        self.asm.li(t, k - 1);
                        self.asm.and_(t, d, t);
                    }
                }
                self.asm.bne(t, ZERO, self.fault_arith);
                let _ = self.pop_vreg();
                self.asm.srli_(d, d, j);
            }
            DIVEXACT if k >= 2 => {
                // A7: non-pow2 constant divisor — the zero guard drops
                // (nonzero by proof); the exactness face stays (a nonzero
                // remainder faults 90, the interpreter's div_exact face)
                let t = self.push_vreg();
                // A11: same literal-pool materialization as the DIV arm
                self.li_k(t, k);
                self.asm.r(1, t, d, 7, self.a1, 0b0110011); // remu a1, d, t
                self.asm.bne(self.a1, ZERO, self.fault_arith);
                self.asm.r(1, t, d, 5, d, 0b0110011); // divu d, d, t
                let _ = self.pop_vreg();
            }
            DIVUP if k == 1 => {}
            DIVUP if pow2j(k).is_some() => {
                // A7 (survey arith.md): div_up(x, 2^j) = (x >> j) +
                // (x & (2^j − 1) != 0). The quotient and the remainder
                // both read the SAME original x, so the remainder mask is
                // computed FIRST and the quotient shifts in place after
                // (srli + andi + beq + addi; no `x + k − 1` add — the
                // interpreter's div_with_round_checked has no such
                // overflow face, and the +1 cannot wrap: the quotient of
                // a k >= 2 division is < the dividend). Same i12_mask
                // three-way fix as the DIVEXACT pow2 arm above.
                let j = pow2j(k).unwrap();
                let t = self.push_vreg();
                match Self::i12_mask(k - 1) {
                    Some(m) => self.asm.andi(t, d, m),
                    None => {
                        self.asm.li(t, k - 1);
                        self.asm.and_(t, d, t);
                    }
                }
                self.asm.srli_(d, d, j);
                let join = self.asm.new_label();
                self.asm.beq(t, ZERO, join);
                self.asm.addi(d, d, 1);
                self.place(join);
                let _ = self.pop_vreg();
            }
            DIVUP if k >= 2 => {
                // A7: non-pow2 constant divisor — nonzero by proof, so the
                // general form's `beq rhs, ZERO` guard is dead and drops.
                // The ceil bump matches the general DIVUP arm (remu →
                // divu → +1 on a nonzero remainder; no overflow check —
                // quo < x for k >= 2).
                let t = self.push_vreg();
                // A11: same literal-pool materialization as the DIV arm
                self.li_k(t, k);
                self.asm.r(1, t, d, 7, self.a1, 0b0110011); // remu a1, d, t
                self.asm.r(1, t, d, 5, d, 0b0110011); // divu d, d, t
                let join = self.asm.new_label();
                self.asm.beq(self.a1, ZERO, join);
                self.asm.addi(d, d, 1);
                self.place(join);
                let _ = self.pop_vreg();
            }
            MOD if k == 1 => self.asm.addi(d, ZERO, 0),
            MOD if pow2j(k).is_some() => {
                // three-way fix (g10_fullnode_mod_pow2_bug_candidate): the
                // 2^j modulus mask only fits `andi` when k-1 <= 2047. A
                // larger mask truncated through `as i32` became -1 (for
                // k >= 2^32) or an assert/garbled immediate — `x % 2^32`
                // returned the ORIGINAL value (10000000000u64 % 2^32 →
                // 10000000000 instead of 1410065408). Masks above the
                // immediate window ride a pool temp (same shape as the
                // general `remu` arm below).
                match Self::i12_mask(k - 1) {
                    Some(m) => self.asm.andi(d, d, m),
                    None => {
                        let t = self.push_vreg();
                        self.asm.li(t, k - 1);
                        self.asm.and_(d, d, t);
                        let _ = self.pop_vreg();
                    }
                }
            }
            MOD if k >= 2 => {
                let t = self.push_vreg();
                // A11: same literal-pool materialization as the DIV arm above
                self.li_k(t, k);
                self.asm.r(1, t, d, 7, d, 0b0110011); // remu d, d, t
                let _ = self.pop_vreg();
            }
            BSHL if k >= w as u64 => {
                // constant amount >= the width: the interpreter's checked
                // shift faults unconditionally — no value side to compute
                self.asm.jal_(ZERO, self.fault_arith);
            }
            BSHL => {
                // constant shamt < w: the amount check folds away
                self.asm.slli_(d, d, k as u32);
                if w < 64 {
                    self.emit_truncate(d, w);
                }
            }
            BSHR if k >= w as u64 => {
                self.asm.jal_(ZERO, self.fault_arith);
            }
            BSHR => {
                self.asm.srli_(d, d, k as u32);
            }
            BAND if k <= 2047 => {
                self.asm.andi(d, d, k as i32);
            }
            _ => return false,
        }
        true
    }

    /// `d = min(d, rhs)` — branch form: 3 insts, 2 cycles (profile 0 has no
    /// Zbb min/max; the 5-inst branchless sequence loses on both axes).
    /// `d = min(d, rhs)` — take rhs only when rhs < d. Both directions
    /// were historically inverted (bltu skip forms) — a latent bug nothing
    /// e2e-tested (CLAMP/MAX/MIN never ran with distinguishing operands)
    /// until the max/min-update fusion exercised them.
    /// The three-way widened width of a triop's operands (the interpreter's
    /// `arithmetic_args3` → `cast_same_uint_width3`). Every operand needs a
    /// static width; bool-ish operands and widths below 8 reject.
    pub(crate) fn triop_arith_width(&self, n: &IRNodeTriple, op: Bytecode) -> Ret<u32> {
        let (wx, wy, wz) = (
            self.int_width(&*n.subx),
            self.int_width(&*n.suby),
            self.int_width(&*n.subz),
        );
        match widen2(widen2(wx, wy), wz) {
            Some(w) if w >= 8 => Ok(w),
            _ => errf!(
                "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                 (operands need static uint widths, got {wx:?}, {wy:?} and {wz:?})"
            ),
        }
    }

    /// `mul_div(x, y, z)` / `mul_div_up(x, y, z)` at widths <= 32 (the caller
    /// has rejected wider forms): the interpreter widens the three `to_uint`
    /// operands to their three-way max width, multiplies exactly (x·y < 2^64
    /// here), divides by z (zero → 90) and casts the quotient back to that
    /// width (ceil on a nonzero remainder for the `_up` form).
    pub(crate) fn emit_muldiv(&mut self, w: u32, up: bool) -> Ret<()> {
        // w <= 64: every operand is one vstack entry, so exactly three pops.
        let zlo = self.pop_reg();
        let ylo = self.pop_reg();
        let xlo = self.pop_reg();
        self.asm.beq(zlo, ZERO, self.fault_arith);
        // The quotient lives in a freshly pushed pool register: in ImcRemap
        // `a1` IS sc2 (x31), so emit_range_check's scratch would clobber a
        // quotient parked there.
        let d = self.push_vreg();
        let r = self.asm.sc1;
        self.asm.r(1, ylo, xlo, 0, d, 0b0110011); // mul d, x, y
        self.asm.r(1, zlo, d, 6, r, 0b0110011); // remu r, d, z
        self.asm.r(1, zlo, d, 5, d, 0b0110011); // divu d, d, z
        if up {
            let l_no = self.asm.new_label();
            self.asm.beq(r, ZERO, l_no);
            self.asm.addi(d, d, 1);
            self.place(l_no);
        }
        if w < 64 {
            self.emit_range_check(d, w);
        }
        Ok(())
    }

    /// `mul_add(x, y, z)` / `mul_sub(x, y, z)` (T3): the interpreter's full
    /// 256-bit intermediate (mul_xy_addsub_z_fit_u128) on the u256 library.
    /// The caller has nil-guarded the three operands (the to_uint 71 faces);
    /// here n = x·y (128×128→256), n ±= z at 256 bits (emit twins of
    /// add_u256_u128 / sub_u256_u128 — the 256-bit add cannot carry and the
    /// sub's underflow faults 90, both identical to the interpreter's
    /// `Option` closes), bits 128..255 must clear (the u128 fit — the
    /// interpreter's `hi != 0` → 90), and the u128 result range-checks to
    /// the common width `w` (cast_uint_result3's try_from → 90). Sub-128
    /// operands zero-extend into OWNED pairs: mul_wide repurposes its
    /// operand registers, and a popped bank ref must stay read-only.
    pub(crate) fn emit_muladdsub(
        &mut self,
        w: u32,
        add: bool,
        wx: Option<u32>,
        wy: Option<u32>,
        wz: Option<u32>,
    ) -> Ret<()> {
        let (x, y, _z) = self.stage_triop_pairs(wx, wy, wz)?;
        let n = self.emit_u256_mul_wide(x, y)?;
        // z's pair (top) retires first; its registers feed the ± helper
        let zhi = self.pop_reg();
        let zlo = self.pop_reg();
        if add {
            self.emit_u256_add_u128(&n, (zlo, zhi));
        } else {
            self.emit_u256_sub_u128(&n, (zlo, zhi));
        }
        // u128 fit: limbs n2/n3 (bits 128..255) must be clear → 90
        let sc1 = self.asm.sc1;
        self.asm.or_(sc1, n[2], n[3]);
        self.asm.bne(sc1, ZERO, self.fault_arith);
        // cast back to the common width; the limb entries retire from the
        // top, leaving the u128 result pair (w == 128) or the single lo
        // entry (w < 128)
        match w {
            128 => {
                let _ = self.pop_vreg(); // n3
                let _ = self.pop_vreg(); // n2
            }
            64 => {
                // u64::try_from(lo): the high limb must be zero → 90
                self.asm.bne(n[1], ZERO, self.fault_arith);
                let _ = self.pop_vreg();
                let _ = self.pop_vreg();
                let _ = self.pop_vreg();
            }
            _ => {
                debug_assert!(matches!(w, 8 | 16 | 32));
                self.emit_range_check(n[0], w);
                let _ = self.pop_vreg();
                let _ = self.pop_vreg();
                let _ = self.pop_vreg();
            }
        }
        Ok(())
    }

    /// `mul_div(x, y, z)` / `mul_div_up(x, y, z)` (T4) at the u64 / u128
    /// common widths: the interpreter's muldiv_with_round_checked pipeline —
    /// n = x·y on the u256 library, then the shared 256/128 soft-division
    /// routine (z == 0 and hi >= z — a quotient above u128 — fault
    /// Arithmetic 90 from INSIDE, div_u256_by_u128_to_u128's `None` faces),
    /// then the ceil bump on a nonzero remainder (carry out of u128 → 90,
    /// ceil_quot_if_rem_u128's checked_add) and the range check to the
    /// common width `w` (cast_uint_result3 → 90). The caller has
    /// nil-guarded the operands (the to_uint 71 faces).
    pub(crate) fn emit_muldiv_wide(
        &mut self,
        w: u32,
        up: bool,
        wx: Option<u32>,
        wy: Option<u32>,
        wz: Option<u32>,
    ) -> Ret<()> {
        debug_assert!(matches!(w, 64 | 128));
        let (x, y, _z) = self.stage_triop_pairs(wx, wy, wz)?;
        let n = self.emit_u256_mul_wide(x, y)?;
        // z's pair retires first (its registers feed the routine's divisor
        // staging); then the four limb entries free their slots for the
        // fresh quotient / remainder registers (the routine stages the
        // dividend through the frame workspace before pushing, so the
        // stored limbs are the only live copy)
        let zhi = self.pop_reg();
        let zlo = self.pop_reg();
        let _ = self.pop_vreg();
        let _ = self.pop_vreg();
        let _ = self.pop_vreg();
        let _ = self.pop_vreg();
        // quo (+ rem for the ceil form); z == 0 / quotient > u128 → 90 in
        // the routine itself
        self.emit_u256_divmod_u128(up, &n, (zlo, zhi))?;
        let (rlo, rhi) = if up {
            let rhi = self.pop_reg();
            let rlo = self.pop_reg();
            (rlo, rhi)
        } else {
            (ZERO, ZERO)
        };
        // [q0, q1] remain as the top two entries (fresh pool pushes)
        let q1 = loc_reg(self.vstack[self.vstack.len() - 1]);
        let q0 = loc_reg(self.vstack[self.vstack.len() - 2]);
        if up {
            // ceil_quot_if_rem_u128: rem != 0 → quo + 1 (checked → 90)
            let sc1 = self.asm.sc1;
            let l_exact = self.asm.new_label();
            self.asm.or_(sc1, rlo, rhi);
            self.asm.beq(sc1, ZERO, l_exact);
            self.asm.addi(q0, q0, 1);
            self.asm.i(1, q0, 3, sc1, 0b0010011); // seqz — carry out of q0
            self.asm.beq(sc1, ZERO, l_exact);
            self.asm.addi(q1, q1, 1);
            self.asm.i(1, q1, 3, sc1, 0b0010011); // seqz — carry out of u128
            self.asm.bne(sc1, ZERO, self.fault_arith);
            self.place(l_exact);
        }
        // cast back to the common width (the result keeps q0, plus q1 at
        // 128 — the first operand's width after the arithmetic_args3
        // widening)
        if w == 64 {
            self.asm.bne(q1, ZERO, self.fault_arith);
            let _ = self.pop_vreg(); // q1's entry retires; q0 keeps the result
        }
        Ok(())
    }

    /* ---- T2: add_mod / mul_mod (interpreter add_mod_u128 /
     * mul_mod_u128, both overflow-free in the branch form) ---- */

    /// `d = (x + y) mod z` at the u64 width (the caller popped x/y/z as
    /// single registers). The interpreter's add_mod_u128 reduces both sides
    /// first, then takes `lhs + rhs - z` when the sum reaches z — at u64 the
    /// raw add can only wrap when the true sum ≥ 2^64 > z, and the wrapped
    /// residue equals the true sum − 2^64 < z, so `carry OR sum ≥ z` picks
    /// exactly the subtract case. z == 0 faults 90 before any remu.
    pub(crate) fn emit_addmod64(&mut self, x: u8, y: u8, z: u8) {
        self.asm.beq(z, ZERO, self.fault_arith);
        let d = self.push_vreg();
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        self.asm.r(1, z, x, 7, d, 0b0110011); // remu d, x, z
        self.asm.r(1, z, y, 7, sc1, 0b0110011); // remu sc1, y, z
        let l_sub = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.add(d, d, sc1);
        // carry: the wrapped sum is < the addend sc1
        self.asm.r(0, sc1, d, 3, sc2, 0b0110011); // sltu sc2, d, sc1
        self.asm.bne(sc2, ZERO, l_sub);
        self.asm.bltu(d, z, l_done);
        self.place(l_sub);
        self.asm.sub(d, d, z);
        self.place(l_done);
    }

    /// `out = (x · y) mod z` at the u64 width: the interpreter's
    /// mul_mod_u128 double-and-add loop (rhs bit scan; `lhs` doubles each
    /// step), each partial sum via the add_mod branch form above. z == 0
    /// faults 90 before any remu. Four values stay live (lhs, rhs, out, z)
    /// against three popped pool slots, so z rides the dedicated scratch
    /// sc1 through the loop (pool-external in every mode; every helper the
    /// loop emits uses sc2 at most transiently) and the three pool slots
    /// carry lhs/rhs/out.
    pub(crate) fn emit_mulmod64(&mut self, x: u8, y: u8, z: u8) -> Ret<()> {
        self.asm.beq(z, ZERO, self.fault_arith);
        let (zr, sc2) = (self.asm.sc1, self.asm.sc2);
        self.asm.mv(zr, z);
        let lhs = self.push_vreg();
        if lhs != x {
            self.asm.mv(lhs, x);
        }
        let rhs = self.push_vreg();
        if rhs != y {
            self.asm.mv(rhs, y);
        }
        self.asm.r(1, zr, lhs, 7, lhs, 0b0110011); // remu lhs, lhs, z
        self.asm.r(1, zr, rhs, 7, rhs, 0b0110011); // remu rhs, rhs, z
        let out = self.push_vreg();
        self.asm.addi(out, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_bit = self.asm.new_label();
        let l_done = self.asm.new_label();
        let l_asub = self.asm.new_label();
        let l_adone = self.asm.new_label();
        let l_dsub = self.asm.new_label();
        let l_ddone = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(rhs, ZERO, l_done);
        // rhs bit 0 → out = add_mod(out, lhs)
        self.asm.andi(sc2, rhs, 1);
        self.asm.beq(sc2, ZERO, l_bit);
        self.asm.add(out, out, lhs);
        self.asm.r(0, lhs, out, 3, sc2, 0b0110011); // sltu sc2, out, lhs
        self.asm.bne(sc2, ZERO, l_asub);
        self.asm.bltu(out, zr, l_adone);
        self.place(l_asub);
        self.asm.sub(out, out, zr);
        self.place(l_adone);
        self.place(l_bit);
        self.asm.srli_(rhs, rhs, 1);
        self.asm.beq(rhs, ZERO, l_done);
        // lhs = add_mod(lhs, lhs): doubling, carry = the pre-shift msb
        self.asm.srli_(sc2, lhs, 63);
        self.asm.slli_(lhs, lhs, 1);
        self.asm.bne(sc2, ZERO, l_dsub);
        self.asm.bltu(lhs, zr, l_ddone);
        self.place(l_dsub);
        self.asm.sub(lhs, lhs, zr);
        self.place(l_ddone);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // retire the two upper temporaries (out, rhs): the bottom slot
        // (lhs's register) keeps the single result entry
        let _ = self.pop_vreg();
        let _ = self.pop_vreg();
        if lhs != out {
            self.asm.mv(lhs, out);
        }
        Ok(())
    }

    /// `d = (d + r) mod z` at 128 bits, IN d's registers — the branch form
    /// of the interpreter's add_mod_u128 (both inputs already < z: every
    /// caller feeds residues). The raw 128-bit add's carry-out OR `sum >= z`
    /// decides the subtract; a carry means the true sum ≥ 2^128 > z and the
    /// wrapped residue is the true sum − 2^128 < z, so `sub` lands back in
    /// range. z's registers are read-only here.
    pub(crate) fn emit_u128_addmod_inplace(&mut self, dlo: u8, dhi: u8, rlo: u8, rhi: u8, zlo: u8, zhi: u8) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let l_sub = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.add(dlo, dlo, rlo);
        self.asm.r(0, rlo, dlo, 3, sc1, 0b0110011); // sltu sc1, dlo, rlo — c1
        self.asm.mv(sc2, dhi); // t0 = the pre-add high half
        self.asm.add(dhi, dhi, rhi);
        self.asm.add(dhi, dhi, sc1); // fold the lo carry
        // total carry-out ⟺ the final high half wrapped below t0
        self.asm.r(0, sc2, dhi, 3, sc1, 0b0110011); // sltu sc1, dhi, sc2
        self.asm.bne(sc1, ZERO, l_sub);
        // d >= z (128-bit compare)?
        self.asm.bltu(dhi, zhi, l_done);
        self.asm.bne(dhi, zhi, l_sub);
        self.asm.bltu(dlo, zlo, l_done);
        self.place(l_sub);
        // d -= z (borrow chain)
        self.asm.r(0, zlo, dlo, 3, sc1, 0b0110011); // sltu sc1, dlo, zlo
        self.asm.sub(dlo, dlo, zlo);
        self.asm.sub(dhi, dhi, zhi);
        self.asm.sub(dhi, dhi, sc1);
        self.place(l_done);
    }

    /// `add_mod(x, y, z)` at the 128-bit common width: two shared-divmod
    /// residue calls (the routine faults 90 on z == 0 itself) then one
    /// in-place add_mod. Operand registers are read-only; the result lands
    /// in a fresh (lo, hi) pool pair.
    pub(crate) fn emit_addmod128(
        &mut self,
        xlo: u8,
        xhi: u8,
        ylo: u8,
        yhi: u8,
        zlo: u8,
        zhi: u8,
    ) -> Ret<()> {
        // lhs = x % z (pushes a 2-entry pair)
        self.emit_u128_divmod(true, xlo, xhi, zlo, zhi)?;
        // rhs = y % z — the pool-held lhs survives the routine (it only
        // touches its own fixed working registers and the frame workspace)
        self.emit_u128_divmod(true, ylo, yhi, zlo, zhi)?;
        let rhi = self.pop_reg();
        let rlo = self.pop_reg();
        let dhi = self.pop_reg();
        let dlo = self.pop_reg();
        self.emit_u128_addmod_inplace(dlo, dhi, rlo, rhi, zlo, zhi);
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        if out_lo != dlo {
            self.asm.mv(out_lo, dlo);
        }
        if out_hi != dhi {
            self.asm.mv(out_hi, dhi);
        }
        Ok(())
    }

    /// `mul_mod(x, y, z)` at the 128-bit common width: hand all three
    /// operands to the shared double-and-add routine through the frame
    /// workspace (the routine faults 90 on z == 0 itself) and reload the
    /// residue as a fresh (lo, hi) pool pair. Layout over the divmod
    /// workspace (never live at the same time): x → s+0/s+8, y →
    /// s+16/s+24, z → s+32/s+40, result → s+48/s+56. D1: the call links
    /// through the mode scratch a0 (the routine parks/restores it at
    /// s+128) — RA is never touched.
    pub(crate) fn emit_mulmod128(
        &mut self,
        xlo: u8,
        xhi: u8,
        ylo: u8,
        yhi: u8,
        zlo: u8,
        zhi: u8,
    ) -> Ret<()> {
        let s = self.divmod_save_off;
        debug_assert!(s >= 0, "mulmod call without a reserved save area");
        self.asm.sd(xlo, S0, s);
        self.asm.sd(xhi, S0, s + 8);
        self.asm.sd(ylo, S0, s + 16);
        self.asm.sd(yhi, S0, s + 24);
        self.asm.sd(zlo, S0, s + 32);
        self.asm.sd(zhi, S0, s + 40);
        let lbl = self.u128_mulmod_label();
        self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        self.asm.ld(out_lo, S0, s + 48);
        self.asm.ld(out_hi, S0, s + 56);
        Ok(())
    }

    /// The shared u128 mul_mod routine — the interpreter's mul_mod_u128:
    /// lhs = x % z and rhs = y % z through a 128-iteration restoring-mod
    /// loop (shift-subtract, the divmod routine's rem arm without the
    /// quotient), then the double-and-add bit scan with every partial sum
    /// reduced by add_mod's branch form. Convention identical in all three
    /// modes (see emit_divmod_routine): operands/results and the working
    /// registers' save area live in the frame workspace at divmod_save_off;
    /// x → s+0/s+8, y → s+16/s+24, z → s+32/s+40, result → s+48/s+56.
    /// Working registers (saved to s+64..128): z = x24:x23, the mod-loop
    /// shift source and later rhs = x7:x6, rem and later out = the mode
    /// scratch pair a1:a0 (pool-external in every mode), lhs = x29:x28,
    /// counter = x14, wrap carry = x22, transient = x17. z == 0 →
    /// fault_arith (90).
    pub(crate) fn emit_mulmod_routine(&mut self) {
        let s = self.divmod_save_off;
        const VLO: u8 = 23;
        const VHI: u8 = 24;
        const CARRY: u8 = 22;
        const TMP: u8 = 17; // a7 — unsaved scratch. DEAD mid-body ONLY
        // because x17 is excluded from the ImcRemap bank candidate list
        // (mod.rs P2-A27 — audit 3103669); see the TMP comment in
        // lib_u128's emit_divmod_routine. Never re-add 17 to the bank
        // candidates without adding it to this routine's save set too.
        debug_assert!(s >= 0, "mulmod routine without a reserved save area");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        // D4: mode-specialized save set — VLO/VHI/CARRY (x23/x24/x22) are
        // bank candidates, ImcRemap-only writers (the no-writers audit is
        // recorded in emit_divmod_routine; identical register classes
        // here). Dropped in Im/Imc (−6 instructions); full set in
        // ImcRemap. Cross-routine review: this routine never calls (or is
        // called by) another routine — its only external edges are the
        // body-side `jal a0` sites and the shared fault thunk (A0 only) —
        // so the reduced save set cannot break a sibling routine.
        let saved: &[u8] = if self.saves_bank_candidate_regs {
            &[T1, T2, 14, VLO, VHI, CARRY, T3, T4]
        } else {
            &[T1, T2, 14, T3, T4]
        };
        // D1: park the link register FIRST — the caller's `jal a0` saved
        // nothing (see emit_divmod_routine). The fault path (z == 0 below)
        // skips the restore exactly like the old call-site `ld RA` did.
        sd(self, self.a0, s + 128);
        for (i, &r) in saved.iter().enumerate() {
            sd(self, r, s + 64 + (i as i32) * 8);
        }
        ld(self, VLO, s + 32);
        ld(self, VHI, s + 40);
        // z == 0 → 90 (the interpreter's require_nonzero face)
        self.asm.or_(TMP, VLO, VHI);
        self.asm.beq(TMP, ZERO, self.fault_arith);
        // One 128-iteration restoring-mod pass: rem(self.a1:self.a0) =
        // src(T2:T1) % z, shifting src dry from its msb. Every partial rem
        // stays < z (the wrap carry forces the subtract — a wrapped
        // 2·rem+bit exceeds z and its wrapped residue is < z, so the plain
        // u128 subtract lands back in range).
        let mod_pass = |lw: &mut Self, src_lo: u8, src_hi: u8| {
            lw.asm.addi(lw.a0, ZERO, 0);
            lw.asm.addi(lw.a1, ZERO, 0);
            lw.asm.addi(14, ZERO, 128);
            let l_loop = lw.asm.new_label();
            let l_hi = lw.asm.new_label();
            let l_sub = lw.asm.new_label();
            let l_next = lw.asm.new_label();
            lw.place(l_loop);
            lw.asm.srli_(CARRY, lw.a1, 63);
            lw.asm.srli_(TMP, lw.a0, 63);
            lw.asm.slli_(lw.a1, lw.a1, 1);
            lw.asm.or_(lw.a1, lw.a1, TMP);
            lw.asm.srli_(TMP, src_hi, 63);
            lw.asm.slli_(lw.a0, lw.a0, 1);
            lw.asm.or_(lw.a0, lw.a0, TMP);
            lw.asm.srli_(TMP, src_lo, 63);
            lw.asm.slli_(src_hi, src_hi, 1);
            lw.asm.or_(src_hi, src_hi, TMP);
            lw.asm.slli_(src_lo, src_lo, 1);
            lw.asm.bne(CARRY, ZERO, l_sub);
            lw.asm.bne(lw.a1, VHI, l_hi);
            lw.asm.r(0, VLO, lw.a0, 3, TMP, 0b0110011); // rem < z (lo)
            lw.asm.beq(TMP, ZERO, l_sub);
            lw.asm.jal_(ZERO, l_next);
            lw.place(l_hi);
            lw.asm.r(0, lw.a1, VHI, 3, TMP, 0b0110011); // VHI < rem.hi
            lw.asm.beq(TMP, ZERO, l_next);
            lw.place(l_sub);
            lw.asm.r(0, VLO, lw.a0, 3, TMP, 0b0110011); // borrow
            lw.asm.sub(lw.a0, lw.a0, VLO);
            lw.asm.sub(lw.a1, lw.a1, VHI);
            lw.asm.sub(lw.a1, lw.a1, TMP);
            lw.place(l_next);
            lw.asm.addi(14, 14, -1);
            lw.asm.bne(14, ZERO, l_loop);
        };
        // lhs (x29:x28) = x % z
        ld(self, T1, s);
        ld(self, T2, s + 8);
        mod_pass(self, T1, T2);
        self.asm.mv(T3, self.a0);
        self.asm.mv(T4, self.a1);
        // rhs (x7:x6) = y % z
        ld(self, T1, s + 16);
        ld(self, T2, s + 24);
        mod_pass(self, T1, T2);
        self.asm.mv(T1, self.a0);
        self.asm.mv(T2, self.a1);
        // out = a1:a0 = 0; the interpreter's bit scan — for each set bit of
        // rhs add lhs (reduced), halve rhs, then double lhs (reduced) while
        // rhs still has bits.
        self.asm.addi(self.a0, ZERO, 0);
        self.asm.addi(self.a1, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_bit = self.asm.new_label();
        let l_dbl = self.asm.new_label();
        let l_done = self.asm.new_label();
        let l_asub = self.asm.new_label();
        let l_adone = self.asm.new_label();
        let l_dsub = self.asm.new_label();
        let l_ddone = self.asm.new_label();
        self.place(l_loop);
        self.asm.or_(TMP, T1, T2); // rhs == 0?
        self.asm.beq(TMP, ZERO, l_done);
        self.asm.andi(TMP, T1, 1);
        self.asm.beq(TMP, ZERO, l_bit);
        // out = add_mod(out, lhs) — inline branch form
        self.asm.add(self.a0, self.a0, T3);
        self.asm.r(0, T3, self.a0, 3, CARRY, 0b0110011); // c1
        self.asm.mv(TMP, self.a1); // t0
        self.asm.add(self.a1, self.a1, T4);
        self.asm.add(self.a1, self.a1, CARRY);
        self.asm.r(0, TMP, self.a1, 3, CARRY, 0b0110011); // carry-out
        self.asm.bne(CARRY, ZERO, l_asub);
        self.asm.bltu(self.a1, VHI, l_adone);
        self.asm.bne(self.a1, VHI, l_asub);
        self.asm.bltu(self.a0, VLO, l_adone);
        self.place(l_asub);
        self.asm.r(0, VLO, self.a0, 3, TMP, 0b0110011); // borrow
        self.asm.sub(self.a0, self.a0, VLO);
        self.asm.sub(self.a1, self.a1, VHI);
        self.asm.sub(self.a1, self.a1, TMP);
        self.place(l_adone);
        self.place(l_bit);
        // rhs >>= 1 (128-bit logical): hi's LOWEST bit shifts into lo's msb
        self.asm.andi(TMP, T2, 1);
        self.asm.srli_(T2, T2, 1);
        self.asm.srli_(T1, T1, 1);
        self.asm.slli_(TMP, TMP, 63);
        self.asm.or_(T1, T1, TMP);
        // lhs = add_mod(lhs, lhs) — doubling, carry = the pre-shift msb.
        // Doubling past the interpreter's `if rhs != 0` guard is inert: the
        // loop head retires on rhs == 0 and out never reads lhs again.
        self.place(l_dbl);
        self.asm.srli_(CARRY, T4, 63);
        self.asm.srli_(TMP, T3, 63);
        self.asm.slli_(T4, T4, 1);
        self.asm.or_(T4, T4, TMP);
        self.asm.slli_(T3, T3, 1);
        self.asm.bne(CARRY, ZERO, l_dsub);
        self.asm.bltu(T4, VHI, l_ddone);
        self.asm.bne(T4, VHI, l_dsub);
        self.asm.bltu(T3, VLO, l_ddone);
        self.place(l_dsub);
        self.asm.r(0, VLO, T3, 3, TMP, 0b0110011); // borrow
        self.asm.sub(T3, T3, VLO);
        self.asm.sub(T4, T4, VHI);
        self.asm.sub(T4, T4, TMP);
        self.place(l_ddone);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // park the result, restore, return
        sd(self, self.a0, s + 48);
        sd(self, self.a1, s + 56);
        for (i, &r) in saved.iter().enumerate() {
            ld(self, r, s + 64 + (i as i32) * 8);
        }
        ld(self, self.a0, s + 128); // D1: return through the link register
        self.asm.jr(self.a0);
    }

    pub(crate) fn emit_min(&mut self, d: u8, rhs: u8) {
        let skip = self.asm.new_label();
        self.asm.bgeu(rhs, d, skip);
        self.asm.mv(d, rhs);
        self.place(skip);
    }

    /// `d = max(d, rhs)` — take rhs only when d < rhs.
    pub(crate) fn emit_max(&mut self, d: u8, rhs: u8) {
        let skip = self.asm.new_label();
        self.asm.bgeu(d, rhs, skip);
        self.asm.mv(d, rhs);
        self.place(skip);
    }
}
