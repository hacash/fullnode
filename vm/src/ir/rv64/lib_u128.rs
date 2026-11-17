use super::*;

impl<'a> Lower<'a> {

    /// Pop one operand of static width `w` as a (lo, hi) register pair.
    /// A sub-128 operand is zero-extended (the interpreter's widen-to-max
    /// arithmetic casts uints up without loss); `None` (bool-ish) passes
    /// through as a single value with hi = 0 — exactly the legacy numeric
    /// face the u64 comparison paths use.
    pub(crate) fn pop_u128_pair(&mut self, w: Option<u32>) -> (u8, u8) {
        if w == Some(128) {
            let hi = self.pop_reg();
            let lo = self.pop_reg();
            (lo, hi)
        } else {
            let lo = self.pop_reg();
            (lo, ZERO)
        }
    }

    /// (T3/T4 triop prelude) Pop the three operands, park y and z in the
    /// u256 workspace's routine-save area, and push the three OWNED pairs
    /// as the stack tail [x, y, z] (z on top). Ownership matters because
    /// `emit_u256_mul_wide` repurposes its operand registers — a popped
    /// bank ref must stay read-only. The park matters because pushes re-hand
    /// the LOWEST pool slots: the y/z pushes would otherwise overwrite the
    /// slots still holding the not-yet-read operand values (x occupies the
    /// lowest slots, so its values survive in place; a sub-128 x's hi mv
    /// only ever writes y's already-parked slot). The routine-save area is
    /// dead between routine calls (the division routines save/restore it
    /// balanced around their jal), and the staged operands are consumed
    /// before any call.
    pub(crate) fn stage_triop_pairs(
        &mut self,
        wx: Option<u32>,
        wy: Option<u32>,
        wz: Option<u32>,
    ) -> Ret<((u8, u8), (u8, u8), (u8, u8))> {
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 triop staging needs the u256 frame workspace (body_needs_u256)"
            );
        }
        let z = self.pop_u128_pair(wz);
        let y = self.pop_u128_pair(wy);
        let x = self.pop_u128_pair(wx);
        // park y and z BEFORE any push can re-hand their slots
        self.asm.sd(y.0, S0, s + U256WS_STAGE_Y);
        self.asm.sd(y.1, S0, s + U256WS_STAGE_Y + 8);
        self.asm.sd(z.0, S0, s + U256WS_STAGE_Z);
        self.asm.sd(z.1, S0, s + U256WS_STAGE_Z + 8);
        // [x] — the lowest slots are x's own; zero traffic for a Pool x
        let xlo = self.push_vreg();
        let xhi = self.push_vreg();
        if xlo != x.0 {
            self.asm.mv(xlo, x.0);
        }
        if xhi != x.1 {
            self.asm.mv(xhi, x.1); // ZERO hi → writes the 0 half
        }
        // [y] then [z], reloading from the park
        let ylo = self.push_vreg();
        let yhi = self.push_vreg();
        self.asm.ld(ylo, S0, s + U256WS_STAGE_Y);
        self.asm.ld(yhi, S0, s + U256WS_STAGE_Y + 8);
        let zlo = self.push_vreg();
        let zhi = self.push_vreg();
        self.asm.ld(zlo, S0, s + U256WS_STAGE_Z);
        self.asm.ld(zhi, S0, s + U256WS_STAGE_Z + 8);
        Ok(((xlo, xhi), (ylo, yhi), (zlo, zhi)))
    }

    /// Consumer guards for a 128-bit compare: both operands are on the
    /// stack (x below y), `ex`/`ey` entries each, tags on the LOW entries.
    /// EQ/NEQ fault 90 iff exactly one side is Nil; ordered compares fault
    /// 90 on any Nil — identical to `nil_guard_int_pair`'s contract.
    pub(crate) fn nil_guard_u128_cmp(&mut self, op: Bytecode, lx: bool, ly: bool, ex: usize, ey: usize) {
        let len = self.vstack.len();
        match op {
            Bytecode::EQ | Bytecode::NEQ if lx && ly => {
                self.nil_guard_eq_tags(len - ey - ex, len - ey)
            }
            Bytecode::EQ | Bytecode::NEQ if lx => self.nil_guard_tag(len - ey - ex, 90),
            Bytecode::EQ | Bytecode::NEQ if ly => self.nil_guard_tag(len - ey, 90),
            _ => {
                if lx {
                    self.nil_guard_tag(len - ey - ex, 90);
                }
                if ly {
                    self.nil_guard_tag(len - ey, 90);
                }
            }
        }
    }

    /// `d = d + rhs` at 128 bits with the interpreter's same-width overflow
    /// face (Arithmetic 90 on any carry out of bit 127).
    pub(crate) fn emit_u128_add(&mut self, dlo: u8, dhi: u8, rlo: u8, rhi: u8) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        self.asm.add(dlo, dlo, rlo);
        self.asm.r(0, rlo, dlo, 3, sc1, 0b0110011); // sltu sc1, dlo', rlo — carry
        self.asm.add(dhi, dhi, rhi);
        self.asm.r(0, rhi, dhi, 3, sc2, 0b0110011); // carry out of the hi add
        self.asm.add(dhi, dhi, sc1); // fold the lo carry in
        self.asm.r(0, sc1, dhi, 3, sc1, 0b0110011); // the carry add wrapped?
        self.asm.or_(sc1, sc1, sc2);
        self.asm.bne(sc1, ZERO, self.fault_arith);
    }

    /// `d = d - rhs` at 128 bits, underflow (borrow out of bit 127) → 90.
    pub(crate) fn emit_u128_sub(&mut self, dlo: u8, dhi: u8, rlo: u8, rhi: u8) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        self.asm.r(0, rlo, dlo, 3, sc1, 0b0110011); // b = dlo < rlo — borrow
        self.asm.sub(dlo, dlo, rlo);
        self.asm.r(0, rhi, dhi, 3, sc2, 0b0110011); // dhi < rhi — hi borrow
        self.asm.sub(dhi, dhi, rhi);
        self.asm.r(0, sc1, dhi, 3, self.a0, 0b0110011); // t < b — fold wraps?
        self.asm.sub(dhi, dhi, sc1); // fold the borrow into the result
        self.asm.or_(self.a0, self.a0, sc2);
        self.asm.bne(self.a0, ZERO, self.fault_arith);
    }

    /// `d = d * rhs` at 128 bits; overflow iff any product bit above bit
    /// 127 is nonzero (the interpreter's u128 checked_mul). Register plan:
    /// temps sc1/sc2/a0/a1 (dead at every call site); `rhi`/`rlo`/`dhi` are
    /// overwritten only after their last operand read; `dlo` moves last.
    pub(crate) fn emit_u128_mul(&mut self, dlo: u8, dhi: u8, rlo: u8, rhi: u8) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        // P1 = dhi*rhi (must vanish: bits 128..255), P2 = dlo*rhi
        self.asm.r(1, rhi, dhi, 3, sc2, 0b0110011); // mulhu sc2, dhi, rhi — hh_hi
        self.asm.r(1, rhi, dlo, 3, sc1, 0b0110011); // mulhu sc1, dlo, rhi — lh_hi
        self.asm.r(1, rhi, dlo, 0, self.a0, 0b0110011); // mul a0, dlo, rhi — lh_lo
        self.asm.r(1, rhi, dhi, 0, rhi, 0b0110011); // mul rhi, dhi, rhi — hh_lo
        self.asm.or_(sc2, sc2, rhi);
        self.asm.bne(sc2, ZERO, self.fault_arith); // hh == 0
        self.asm.bne(sc1, ZERO, self.fault_arith); // lh_hi == 0
        // P3 = dhi*rlo (hi must vanish), P4 = dlo*rlo → the product
        self.asm.r(1, rlo, dhi, 3, sc2, 0b0110011); // mulhu sc2, dhi, rlo — hl_hi
        self.asm.bne(sc2, ZERO, self.fault_arith);
        self.asm.r(1, rlo, dhi, 0, dhi, 0b0110011); // mul dhi, dhi, rlo — hl_lo
        self.asm.r(1, rlo, dlo, 3, sc2, 0b0110011); // mulhu sc2, dlo, rlo — ll_hi
        self.asm.r(1, rlo, dlo, 0, rlo, 0b0110011); // mul rlo, dlo, rlo — ll_lo
        // hi = hl_lo + lh_lo + ll_hi with carries; lo = ll_lo
        self.asm.add(dhi, dhi, self.a0); // hl_lo + lh_lo
        self.asm.r(0, self.a0, dhi, 3, sc1, 0b0110011); // carry c1
        self.asm.add(dhi, dhi, sc2); // + ll_hi
        self.asm.r(0, sc2, dhi, 3, self.a1, 0b0110011); // carry c2
        self.asm.or_(sc1, sc1, self.a1);
        self.asm.bne(sc1, ZERO, self.fault_arith);
        self.asm.mv(dlo, rlo);
    }

    /// `d = d <op> rhs` at 128 bits for the bitwise family (no overflow
    /// face — the interpreter's u128 bitops never fault).
    pub(crate) fn emit_u128_bitop(&mut self, op: Bytecode, dlo: u8, dhi: u8, rlo: u8, rhi: u8) {
        let f3 = match op {
            Bytecode::BAND => 7u32,
            Bytecode::BOR => 6,
            _ => 4, // BXOR
        };
        self.asm.r(0, rlo, dlo, f3, dlo, 0b0110011);
        self.asm.r(0, rhi, dhi, f3, dhi, 0b0110011);
    }

    /// `d = (a CMP b)` as a 0/1 byte, numeric order (the interpreter's
    /// `lgc_ord_cmp`/`value_content_eq` — mixed uint widths compare by
    /// value, so both pairs are already zero-extended by the caller).
    pub(crate) fn emit_u128_cmp(&mut self, op: Bytecode, d: u8, a: (u8, u8), b: (u8, u8)) {
        let (alo, ahi) = a;
        let (blo, bhi) = b;
        let sc1 = self.asm.sc1;
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        match op {
            Bytecode::EQ | Bytecode::NEQ => {
                self.asm.xor(sc1, alo, blo);
                self.asm.xor(d, ahi, bhi);
                self.asm.or_(sc1, sc1, d);
                if op == Bytecode::EQ {
                    self.asm.i(1, sc1, 3, d, 0b0010011); // seqz d, sc1
                } else {
                    sltu(self, d, ZERO, sc1); // snez d, sc1
                }
            }
            Bytecode::LT | Bytecode::GE => {
                let l_done = self.asm.new_label();
                sltu(self, sc1, ahi, bhi); // a_hi < b_hi decides
                self.asm.bne(ahi, bhi, l_done);
                sltu(self, sc1, alo, blo); // hi equal → lo decides
                self.place(l_done);
                if op == Bytecode::LT {
                    self.asm.mv(d, sc1);
                } else {
                    self.asm.i(1, sc1, 3, d, 0b0010011); // seqz (invert for GE)
                }
            }
            _ => {
                // GT / LE: mirror the operand order, invert for LE
                let l_done = self.asm.new_label();
                sltu(self, sc1, bhi, ahi); // b_hi < a_hi decides
                self.asm.bne(bhi, ahi, l_done);
                sltu(self, sc1, blo, alo);
                self.place(l_done);
                if op == Bytecode::GT {
                    self.asm.mv(d, sc1);
                } else {
                    self.asm.i(1, sc1, 3, d, 0b0010011); // seqz (invert for LE)
                }
            }
        }
    }

    /// Register the shared soft-divmod routine label (lazily — the routine
    /// is emitted after the body only when some site actually calls it).
    pub(crate) fn u128_divmod_label(&mut self) -> usize {
        *self
            .divmod_lbl
            .get_or_insert_with(|| self.asm.new_label())
    }

    /// T2: the shared u128 mul_mod routine's label (lazy, same pattern as
    /// [`Self::u128_divmod_label`]).
    pub(crate) fn u128_mulmod_label(&mut self) -> usize {
        *self
            .mulmod_lbl
            .get_or_insert_with(|| self.asm.new_label())
    }

    /// The 128-bit form of the binary int arm. Comparisons accept mixed
    /// widths (the interpreter's ordered compare / content equality are
    /// numeric over u128); arithmetic requires BOTH operands statically
    /// u128 — the interpreter widens a narrower side, and a compile
    /// rejection is the iron-rule stance where the widened face cannot be
    /// reproduced one-to-one (pin the operand with `as u128`).
    pub(crate) fn emit_bin128(&mut self, op: Bytecode, n: &IRNodeDouble) -> Ret<()> {
        use Bytecode::*;
        let wx = self.int_width(&*n.subx);
        let wy = self.int_width(&*n.suby);
        match op {
            EQ | NEQ | LT | GT | LE | GE => {
                let (ex, ey) = (Self::entries_of_width(wx), Self::entries_of_width(wy));
                let (lx, ly) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_int(&*n.subx)?;
                self.eval_int(&*n.suby)?;
                if lx || ly {
                    self.nil_guard_u128_cmp(op, lx, ly, ex, ey);
                }
                let b = self.pop_u128_pair(wy);
                let a = self.pop_u128_pair(wx);
                let d = self.push_vreg();
                self.emit_u128_cmp(op, d, a, b);
                Ok(())
            }
            ADD | SUB | MUL | DIV | MOD | BAND | BOR | BXOR => {
                // T5 mixed widths: the interpreter's arithmetic_args2 widens
                // BOTH sides to the common width (here 128, since the caller
                // routes any u128 operand here) — zero-extending the narrow
                // side is faithful (uints are non-negative). Every operand
                // still needs a definite uint width: a bool-ish/widthless
                // side has no reproducible face (the interpreter's to_uint
                // faults 71 on bools) — that shape stays a compile reject.
                if !matches!(wx, Some(w) if (8..=128).contains(&w))
                    || !matches!(wy, Some(w) if (8..=128).contains(&w))
                {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                         (u128 arithmetic needs definite uint widths on both operands, got \
                         {wx:?} and {wy:?} — pin a widthless/bool operand with `as u128`; \
                         comparisons may stay mixed)"
                    );
                }
                let (lx, ly) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_int(&*n.subx)?;
                // a narrow LHS becomes a full writable (lo, 0) pool pair
                // BEFORE the rhs evaluates: the add/sub/mul/bit emit helpers
                // mutate the destination pair in place (a bank Ref or the
                // ZERO constant is not writable), and staging now keeps the
                // pair contiguous under the rhs (the XOP staging precedent).
                let ex = if wx == Some(128) {
                    2
                } else {
                    let _ = self.pop_own_replace();
                    let hi = self.push_vreg();
                    self.asm.addi(hi, ZERO, 0);
                    2
                };
                self.eval_int(&*n.suby)?;
                let ey = Self::entries_of_width(wy);
                let len = self.vstack.len();
                let code = 71u64; // to_uint before the op — CastFail(71)
                if lx {
                    self.nil_guard_tag(len - ey - ex, code);
                }
                if ly {
                    self.nil_guard_tag(len - ey, code);
                }
                let rhs = self.pop_u128_pair(wy);
                // the lhs is ALWAYS a full pair here (native u128 or staged
                // zero-extended above)
                let lhs = self.pop_u128_pair(Some(128));
                match op {
                    DIV => self.emit_u128_divmod(false, lhs.0, lhs.1, rhs.0, rhs.1),
                    MOD => self.emit_u128_divmod(true, lhs.0, lhs.1, rhs.0, rhs.1),
                    _ => {
                        match op {
                            ADD => self.emit_u128_add(lhs.0, lhs.1, rhs.0, rhs.1),
                            SUB => self.emit_u128_sub(lhs.0, lhs.1, rhs.0, rhs.1),
                            MUL => self.emit_u128_mul(lhs.0, lhs.1, rhs.0, rhs.1),
                            _ => self.emit_u128_bitop(op, lhs.0, lhs.1, rhs.0, rhs.1),
                        }
                        let out_lo = self.push_vreg();
                        let out_hi = self.push_vreg();
                        if out_lo != lhs.0 {
                            self.asm.mv(out_lo, lhs.0);
                        }
                        if out_hi != lhs.1 {
                            self.asm.mv(out_hi, lhs.1);
                        }
                        Ok(())
                    }
                }
            }
            // T6: the remaining int binops at u128. Every one rides the
            // interpreter's binop_arithmetic → arithmetic_args2 face: both
            // sides widen to the common width (128 here), Nil → 71 before
            // the op, and the op's own checked semantics fault 90.
            BSHL | BSHR | MAX | MIN | ABSDIFF | DIVUP | DIVEXACT | POW => {
                if !matches!(wx, Some(w) if (8..=128).contains(&w))
                    || !matches!(wy, Some(w) if (8..=128).contains(&w))
                {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                         at u128 width (operands need definite uint widths, got {wx:?} and \
                         {wy:?})"
                    );
                }
                let (lx, ly) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
                self.eval_int(&*n.subx)?;
                // POW's base pair is squared IN PLACE, so a narrow base
                // stages to a writable (lo, 0) pair first (same pattern as
                // the arithmetic arm); every other op here only READS the
                // lhs pair (a bank Ref / the ZERO half extend read-only).
                let ex = if wx == Some(128) || op != POW {
                    Self::entries_of_width(wx)
                } else {
                    let _ = self.pop_own_replace();
                    let hi = self.push_vreg();
                    self.asm.addi(hi, ZERO, 0);
                    2
                };
                self.eval_int(&*n.suby)?;
                // POW shifts the exponent down and squares the base, so a
                // narrow EXPONENT also stages to a writable pair (the
                // amount/exp regs of every other op here are read-only).
                let ey = if wy == Some(128) || op != POW {
                    Self::entries_of_width(wy)
                } else {
                    let _ = self.pop_own_replace();
                    let hi = self.push_vreg();
                    self.asm.addi(hi, ZERO, 0);
                    2
                };
                let len = self.vstack.len();
                if lx {
                    self.nil_guard_tag(len - ey - ex, 71);
                }
                if ly {
                    self.nil_guard_tag(len - ey, 71);
                }
                // POW register plan: the loop keeps base / exp / result in
                // three DISTINCT pairs. Pushing the result pair while both
                // operands are still live hands out fresh slots; pushing
                // after the pops would recycle the BASE's registers and the
                // `result = 1` init would destroy it.
                let pow_dst = if op == POW {
                    let dlo = self.push_vreg();
                    let dhi = self.push_vreg();
                    let _ = self.pop_vreg(); // dhi (register number kept)
                    let _ = self.pop_vreg(); // dlo (register number kept)
                    Some((dlo, dhi))
                } else {
                    None
                };
                let rhs = self.pop_u128_pair(if op == POW { Some(128) } else { wy });
                let lhs = self.pop_u128_pair(if op == POW { Some(128) } else { wx });
                match op {
                    MAX | MIN => {
                        // max/min_checked: the result IS one of the widened
                        // operands — select the whole pair by 128-bit order.
                        let out_lo = self.push_vreg();
                        let out_hi = self.push_vreg();
                        self.asm.mv(out_lo, lhs.0);
                        self.asm.mv(out_hi, lhs.1);
                        let sc1 = self.asm.sc1;
                        let l_decided = self.asm.new_label();
                        let l_take_b = self.asm.new_label();
                        let l_done = self.asm.new_label();
                        // r(0, rs2, rs1, 3, rd) → sltu rd, rs1, rs2
                        if op == MAX {
                            // take b iff b > a ⇔ a < b
                            self.asm.r(0, rhs.1, lhs.1, 3, sc1, 0b0110011);
                            self.asm.bne(lhs.1, rhs.1, l_decided);
                            self.asm.r(0, rhs.0, lhs.0, 3, sc1, 0b0110011);
                        } else {
                            // take b iff b < a
                            self.asm.r(0, lhs.1, rhs.1, 3, sc1, 0b0110011);
                            self.asm.bne(lhs.1, rhs.1, l_decided);
                            self.asm.r(0, lhs.0, rhs.0, 3, sc1, 0b0110011);
                        }
                        self.place(l_decided);
                        self.asm.bne(sc1, ZERO, l_take_b);
                        self.asm.jal_(ZERO, l_done); // keep a
                        self.place(l_take_b);
                        self.asm.mv(out_lo, rhs.0);
                        self.asm.mv(out_hi, rhs.1);
                        self.place(l_done);
                        Ok(())
                    }
                    ABSDIFF => {
                        // |a − b| at the common width; on each branch the
                        // minuend ≥ subtrahend, so the shared checked sub's
                        // borrow fault is dead by construction. The out pair
                        // recycles the lhs's registers (post-pop pushes) —
                        // each branch subtracts INTO the source that already
                        // holds its minuend and only then moves to out.
                        let out_lo = self.push_vreg();
                        let out_hi = self.push_vreg();
                        let sc1 = self.asm.sc1;
                        let l_decided = self.asm.new_label();
                        let l_a_ge = self.asm.new_label();
                        let l_done = self.asm.new_label();
                        self.asm.r(0, rhs.1, lhs.1, 3, sc1, 0b0110011); // sc1 = ahi < bhi
                        self.asm.bne(lhs.1, rhs.1, l_decided);
                        self.asm.r(0, rhs.0, lhs.0, 3, sc1, 0b0110011); // sc1 = alo < blo
                        self.place(l_decided);
                        self.asm.beq(sc1, ZERO, l_a_ge);
                        // a < b → b − a, computed in b's own registers
                        self.emit_u128_sub(rhs.0, rhs.1, lhs.0, lhs.1);
                        self.asm.mv(out_lo, rhs.0);
                        self.asm.mv(out_hi, rhs.1);
                        self.asm.jal_(ZERO, l_done);
                        self.place(l_a_ge);
                        // a ≥ b → a − b: out already holds a (lhs-aliased)
                        self.emit_u128_sub(out_lo, out_hi, rhs.0, rhs.1);
                        self.place(l_done);
                        Ok(())
                    }
                    BSHL | BSHR => self.emit_u128_shift(op, lhs, rhs),
                    DIVUP | DIVEXACT => {
                        // div_with_round_checked: z = 0 → 90 (inside the
                        // routine); quo/rem both load; DIVUP adds 1 on a
                        // nonzero remainder, DIVEXACT faults 90 on one.
                        self.emit_u128_divmod_qr(lhs.0, lhs.1, rhs.0, rhs.1)?;
                        let rhi = self.pop_reg();
                        let rlo = self.pop_reg();
                        let qhi = self.pop_reg();
                        let qlo = self.pop_reg();
                        let sc1 = self.asm.sc1;
                        self.asm.or_(sc1, rlo, rhi);
                        if op == DIVEXACT {
                            self.asm.bne(sc1, ZERO, self.fault_arith);
                        } else {
                            // quo + 1 when rem != 0. Overflow is provably
                            // unreachable: quo == 2^128−1 forces div == 1
                            // and rem == 0, so a nonzero rem bounds quo below
                            // the maximum — no overflow check is needed.
                            let l_no = self.asm.new_label();
                            self.asm.beq(sc1, ZERO, l_no);
                            self.asm.addi(qlo, qlo, 1);
                            // carry into hi ⟺ the lo add wrapped to 0
                            self.asm.addi(self.a0, ZERO, 1);
                            self.asm.r(0, self.a0, qlo, 3, self.a0, 0b0110011);
                            self.asm.add(qhi, qhi, self.a0);
                            self.place(l_no);
                        }
                        let out_lo = self.push_vreg();
                        let out_hi = self.push_vreg();
                        if out_lo != qlo {
                            self.asm.mv(out_lo, qlo);
                        }
                        if out_hi != qhi {
                            self.asm.mv(out_hi, qhi);
                        }
                        Ok(())
                    }
                    POW => {
                        let (dlo, dhi) = pow_dst.unwrap();
                        self.emit_u128_pow(lhs.0, lhs.1, rhs.0, rhs.1, dlo, dhi)?;
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
                    _ => unreachable!("T6 arm op set is closed"),
                }
            }
            _ => errf!(
                "rv64 lower: {op:?} is outside the translatable kernel subset at u128 width \
                 (the kernel computes u128 add/sub/mul/div/mod, bitwise, shifts, pow, \
                 min/max/absdiff, div_up/div_exact and comparisons)"
            ),
        }
    }

    /// CLAMP(x, lo, hi) with a u128 x (T6): the interpreter's
    /// `clamp_checked` after `arithmetic_args3` widened all three operands
    /// to the common width (128 — x itself is u128, so the result keeps the
    /// static CLAMP width = x's width). Faces: any Nil operand → 71
    /// (to_uint before the op), lo > hi → Arithmetic 90 BEFORE clamping
    /// (BUG-010), then out = min(max(x, lo), hi). The bounds may be any
    /// definite uint width (zero-extended, read-only); x is a native pair
    /// (u128 values are never banked, so mutating it in place is sound).
    pub(crate) fn emit_clamp128(&mut self, n: &IRNodeTriple) -> Ret<()> {
        let (wy, wz) = (self.int_width(&*n.suby), self.int_width(&*n.subz));
        if !matches!(wy, Some(w) if (8..=128).contains(&w))
            || !matches!(wz, Some(w) if (8..=128).contains(&w))
        {
            return errf!(
                "rv64 lower: CLAMP is outside the translatable kernel subset at u128 width \
                 (the bounds need definite uint widths, got {wy:?} and {wz:?})"
            );
        }
        let (mx, mlo, mhi) = (
            self.nil_val(&*n.subx),
            self.nil_val(&*n.suby),
            self.nil_val(&*n.subz),
        );
        self.eval_int(&*n.subx)?; // x — a native (lo, hi) pair
        self.eval_int(&*n.suby)?; // lo
        self.eval_int(&*n.subz)?; // hi
        let ey = Self::entries_of_width(wy);
        let ez = Self::entries_of_width(wz);
        let len = self.vstack.len();
        if mx {
            self.nil_guard_tag(len - ez - ey - 2, 71);
        }
        if mlo {
            self.nil_guard_tag(len - ez - ey, 71);
        }
        if mhi {
            self.nil_guard_tag(len - ez, 71);
        }
        let hi = self.pop_u128_pair(wz);
        let lo = self.pop_u128_pair(wy);
        let dxhi = self.pop_reg();
        let dxlo = self.pop_reg();
        let sc1 = self.asm.sc1;
        // lo > hi → Arithmetic 90 BEFORE clamping (hi < lo ⇔ lo > hi)
        let l_chk = self.asm.new_label();
        self.asm.r(0, lo.1, hi.1, 3, sc1, 0b0110011); // sltu sc1, hi.hi, lo.hi
        self.asm.bne(hi.1, lo.1, l_chk);
        self.asm.r(0, lo.0, hi.0, 3, sc1, 0b0110011); // sltu sc1, hi.lo, lo.lo
        self.place(l_chk);
        self.asm.bne(sc1, ZERO, self.fault_arith);
        // d = min(d, hi): take hi iff hi < d
        let l_min = self.asm.new_label();
        let l_min_take = self.asm.new_label();
        let l_min_done = self.asm.new_label();
        self.asm.r(0, dxhi, hi.1, 3, sc1, 0b0110011); // sltu sc1, hi.hi, d.hi
        self.asm.bne(hi.1, dxhi, l_min);
        self.asm.r(0, dxlo, hi.0, 3, sc1, 0b0110011); // sltu sc1, hi.lo, d.lo
        self.place(l_min);
        self.asm.bne(sc1, ZERO, l_min_take);
        self.asm.jal_(ZERO, l_min_done);
        self.place(l_min_take);
        self.asm.mv(dxlo, hi.0);
        self.asm.mv(dxhi, hi.1);
        self.place(l_min_done);
        // d = max(d, lo): take lo iff d < lo
        let l_max = self.asm.new_label();
        let l_max_take = self.asm.new_label();
        let l_max_done = self.asm.new_label();
        self.asm.r(0, lo.1, dxhi, 3, sc1, 0b0110011); // sltu sc1, d.hi, lo.hi
        self.asm.bne(lo.1, dxhi, l_max);
        self.asm.r(0, lo.0, dxlo, 3, sc1, 0b0110011); // sltu sc1, d.lo, lo.lo
        self.place(l_max);
        self.asm.bne(sc1, ZERO, l_max_take);
        self.asm.jal_(ZERO, l_max_done);
        self.place(l_max_take);
        self.asm.mv(dxlo, lo.0);
        self.asm.mv(dxhi, lo.1);
        self.place(l_max_done);
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        if out_lo != dxlo {
            self.asm.mv(out_lo, dxlo);
        }
        if out_hi != dxhi {
            self.asm.mv(out_hi, dxhi);
        }
        Ok(())
    }

    /// Call the shared routine for `D / V` (`want_mod` → `D % V`). Both
    /// operands are (lo, hi) pool pairs already popped off the vstack; the
    /// quotient/remainder pair lands in two fresh pool registers (lo, hi).
    /// Convention (mode-independent — in ImcRemap `self.a1` IS `sc2`, so a
    /// register handoff would clobber itself): D → frame s+0/s+8, V →
    /// s+16/s+24; the routine writes q to s+32/s+40 and r to s+48/s+56;
    /// the kernel-entry RA parks at s+128 around the `jal` (the routine
    /// returns via the post-jal RA, so without this the body's final `ret`
    /// would jump back into itself). Divide-by-zero faults 90 from inside
    /// the routine.
    pub(crate) fn emit_u128_divmod(&mut self, want_mod: bool, dlo: u8, dhi: u8, rlo: u8, rhi: u8) -> Ret<()> {
        let s = self.divmod_save_off;
        debug_assert!(s >= 0, "divmod call without a reserved save area");
        self.asm.sd(dlo, S0, s);
        self.asm.sd(dhi, S0, s + 8);
        self.asm.sd(rlo, S0, s + 16);
        self.asm.sd(rhi, S0, s + 24);
        let lbl = self.u128_divmod_label();
        self.asm.sd(RA, S0, s + 128);
        self.asm.jal_(RA, lbl);
        self.asm.ld(RA, S0, s + 128);
        let (qoff_lo, qoff_hi) = if want_mod { (s + 48, s + 56) } else { (s + 32, s + 40) };
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        self.asm.ld(out_lo, S0, qoff_lo);
        self.asm.ld(out_hi, S0, qoff_hi);
        Ok(())
    }

    /// The DIVUP/DIVEXACT companion of [`Self::emit_u128_divmod`]: run the
    /// shared routine and load BOTH the quotient pair (bottom two entries)
    /// and the remainder pair (top two) — the round policies need quo and
    /// rem together. Same frame convention; divide-by-zero faults 90 from
    /// inside the routine.
    pub(crate) fn emit_u128_divmod_qr(&mut self, dlo: u8, dhi: u8, vlo: u8, vhi: u8) -> Ret<()> {
        let s = self.divmod_save_off;
        debug_assert!(s >= 0, "divmod call without a reserved save area");
        self.asm.sd(dlo, S0, s);
        self.asm.sd(dhi, S0, s + 8);
        self.asm.sd(vlo, S0, s + 16);
        self.asm.sd(vhi, S0, s + 24);
        let lbl = self.u128_divmod_label();
        self.asm.sd(RA, S0, s + 128);
        self.asm.jal_(RA, lbl);
        self.asm.ld(RA, S0, s + 128);
        let qlo = self.push_vreg();
        let qhi = self.push_vreg();
        self.asm.ld(qlo, S0, s + 32);
        self.asm.ld(qhi, S0, s + 40);
        let rmlo = self.push_vreg();
        let rmhi = self.push_vreg();
        self.asm.ld(rmlo, S0, s + 48);
        self.asm.ld(rmhi, S0, s + 56);
        Ok(())
    }

    /// `d = d << s` / `d = d >> s` at 128 bits, the interpreter's
    /// `bit_shl`/`bit_shr` (Rust checked_shl/checked_shr on u128): any
    /// amount ≥ 128 faults 90 (the rhs > u32::MAX pre-check is subsumed —
    /// every such amount is ≥ 128); a legal amount TRUNCATES silently (no
    /// value-overflow face). `a` is the value pair (a narrow side arrives
    /// zero-extended, hi = ZERO), `s` the amount pair; both read-only.
    /// Temps: sc1/sc2 only.
    pub(crate) fn emit_u128_shift(&mut self, op: Bytecode, a: (u8, u8), s: (u8, u8)) -> Ret<()> {
        let (alo, ahi) = a;
        let (slo, shi) = s;
        let (t1, t2) = (self.asm.sc1, self.asm.sc2);
        self.asm.bne(shi, ZERO, self.fault_arith);
        self.asm.addi(t1, ZERO, 128);
        self.asm.bgeu(slo, t1, self.fault_arith);
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        self.asm.mv(out_lo, alo);
        self.asm.mv(out_hi, ahi);
        let l_hi = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.addi(t1, ZERO, 64);
        self.asm.bgeu(slo, t1, l_hi);
        // amount in [1, 63]
        self.asm.beq(slo, ZERO, l_done);
        self.asm.sub(t2, t1, slo); // 64 − s
        if op == Bytecode::BSHL {
            // lo' = lo << s; hi' = (hi << s) | (lo >> (64−s))
            self.asm.r(0, t2, alo, 5, t2, 0b0110011); // t2 = alo >> (64−s)
            self.asm.r(0, slo, out_hi, 1, out_hi, 0b0110011); // out_hi <<= s
            self.asm.or_(out_hi, out_hi, t2);
            self.asm.r(0, slo, out_lo, 1, out_lo, 0b0110011); // out_lo <<= s
        } else {
            // lo' = (lo >> s) | (hi << (64−s)); hi' = hi >> s
            self.asm.r(0, t2, ahi, 1, t2, 0b0110011); // t2 = ahi << (64−s)
            self.asm.r(0, slo, out_lo, 5, out_lo, 0b0110011); // out_lo >>= s
            self.asm.or_(out_lo, out_lo, t2);
            self.asm.r(0, slo, out_hi, 5, out_hi, 0b0110011); // out_hi >>= s
        }
        self.asm.jal_(ZERO, l_done);
        self.place(l_hi);
        // amount in [64, 127]
        self.asm.sub(t1, slo, t1); // s − 64
        if op == Bytecode::BSHL {
            self.asm.r(0, t1, alo, 1, out_hi, 0b0110011); // out_hi = alo << (s−64)
            self.asm.addi(out_lo, ZERO, 0);
        } else {
            self.asm.r(0, t1, ahi, 5, out_lo, 0b0110011); // out_lo = ahi >> (s−64)
            self.asm.addi(out_hi, ZERO, 0);
        }
        self.place(l_done);
        Ok(())
    }

    /// `d = d * b` at 128 bits with the interpreter's checked_mul face (90
    /// on any product bit above 127), PRESERVING the rhs pair — the POW
    /// loop keeps its base across iterations and squares it in place, so d
    /// and r may alias. Temps: sc1/sc2/a0 only (in ImcRemap `self.a1` IS
    /// `sc2`, so exactly three distinct scratches exist; every register is
    /// written only after its last read, under d == r aliasing too).
    pub(crate) fn emit_u128_mul_keep(&mut self, dlo: u8, dhi: u8, blo: u8, bhi: u8) {
        let (sc1, sc2, a0) = (self.asm.sc1, self.asm.sc2, self.a0);
        // hh = dhi*bhi must vanish entirely (bits ≥ 128)
        self.asm.r(1, bhi, dhi, 3, sc2, 0b0110011); // mulhu sc2 — hh_hi
        self.asm.r(1, bhi, dlo, 3, sc1, 0b0110011); // mulhu sc1 — lh_hi
        self.asm.r(1, bhi, dlo, 0, a0, 0b0110011); // mul a0 — lh_lo
        self.asm.bne(sc2, ZERO, self.fault_arith);
        self.asm.r(1, bhi, dhi, 0, sc2, 0b0110011); // mul sc2 — hh_lo
        self.asm.bne(sc2, ZERO, self.fault_arith);
        self.asm.bne(sc1, ZERO, self.fault_arith); // lh_hi
        // hl = dhi*blo (hi must vanish), ll = dlo*blo → the product
        self.asm.r(1, blo, dhi, 3, sc2, 0b0110011); // mulhu sc2 — hl_hi
        self.asm.bne(sc2, ZERO, self.fault_arith);
        self.asm.r(1, blo, dhi, 0, dhi, 0b0110011); // mul dhi — hl_lo
        self.asm.r(1, blo, dlo, 3, sc2, 0b0110011); // mulhu sc2 — ll_hi
        self.asm.r(1, blo, dlo, 0, sc1, 0b0110011); // mul sc1 — ll_lo
        // hi = hl_lo + lh_lo + ll_hi with carries; lo = ll_lo
        self.asm.add(dhi, dhi, a0);
        self.asm.r(0, a0, dhi, 3, a0, 0b0110011); // carry c1
        self.asm.add(dhi, dhi, sc2);
        self.asm.r(0, sc2, dhi, 3, sc2, 0b0110011); // carry c2
        self.asm.or_(a0, a0, sc2);
        self.asm.bne(a0, ZERO, self.fault_arith);
        self.asm.mv(dlo, sc1);
    }

    /// POW at 128 bits: the exponent must fit u32 (checked BEFORE any
    /// multiplication — base 0/1 with a huge exponent still faults 90, the
    /// interpreter's `exp_u32` face), then square-and-multiply with checked
    /// 128-bit muls. An intermediate overflow implies the mathematical b^e
    /// leaves u128 (squaring only runs while the remaining exponent needs
    /// it), so the checked face matches `u128::checked_pow` exactly.
    /// `blo/bhi` is the base pair, `elo/ehi` the exponent pair — both must
    /// be WRITABLE pool registers (the base is squared in place, the
    /// exponent shifts down), i.e. a narrow side stages to a pair first.
    pub(crate) fn emit_u128_pow(
        &mut self,
        blo: u8,
        bhi: u8,
        elo: u8,
        ehi: u8,
        dlo: u8,
        dhi: u8,
    ) -> Ret<()> {
        let t = self.asm.sc1;
        self.asm.srli_(t, elo, 32);
        self.asm.or_(t, t, ehi);
        self.asm.bne(t, ZERO, self.fault_arith);
        self.asm.addi(dlo, ZERO, 1); // result = 1 (e == 0 → 1)
        self.asm.addi(dhi, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_skip = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        // exp ≤ u32::MAX here, so exp == elo (ehi is provably zero)
        self.asm.beq(elo, ZERO, l_done);
        self.asm.andi(t, elo, 1);
        self.asm.beq(t, ZERO, l_skip);
        self.emit_u128_mul_keep(dlo, dhi, blo, bhi); // result *= base
        self.place(l_skip);
        self.asm.srli_(elo, elo, 1);
        self.asm.beq(elo, ZERO, l_done);
        self.emit_u128_mul_keep(blo, bhi, blo, bhi); // base *= base (d == r)
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        Ok(())
    }

    /// T7: `slot OP= k` on a u128 slot with an int constant rhs — the u128
    /// counterpart of the emit_arith_k guard absorption. The old pair is
    /// already staged in `slo/shi` (pool registers, values loaded) and the
    /// old-value Nil flag was guarded by the caller; the constant rhs
    /// cannot be Nil and cannot fault in to_uint, so only the op's own
    /// faces remain (ADD/SUB/MUL checked at 128 bits → 90; DIV by a zero
    /// constant → 90 unconditionally; a pow2 divisor is a plain shift, no
    /// routine call). The result stores back to the 16-byte cell.
    pub(crate) fn emit_xop128_k(&mut self, op: Bytecode, idx: u8, k: u64, slo: u8, shi: u8) -> Ret<()> {
        let pow2j = |k: u64| -> Option<u32> {
            (k != 0 && k.count_ones() == 1).then(|| k.trailing_zeros())
        };
        match op {
            Bytecode::ADD | Bytecode::SUB => {
                // the constant rides a POOL register: the shared emit
                // helpers use sc1/sc2/a0/a1 as their carry/product temps,
                // so a scratch temp would alias and clobber the constant
                // mid-sequence (pool homes are disjoint from scratches in
                // every mode)
                let t = self.push_vreg();
                self.asm.li(t, k);
                if op == Bytecode::ADD {
                    self.emit_u128_add(slo, shi, t, ZERO);
                } else {
                    self.emit_u128_sub(slo, shi, t, ZERO);
                }
                let _ = self.pop_vreg();
            }
            Bytecode::MUL => match k {
                0 => {
                    self.asm.addi(slo, ZERO, 0);
                    self.asm.addi(shi, ZERO, 0);
                }
                1 => {}
                _ => {
                    let t = self.push_vreg();
                    self.asm.li(t, k);
                    self.emit_u128_mul(slo, shi, t, ZERO);
                    let _ = self.pop_vreg();
                }
            },
            Bytecode::DIV => match k {
                // the interpreter's div_checked: a zero divisor faults 90
                0 => self.asm.jal_(ZERO, self.fault_arith),
                1 => {}
                _ if pow2j(k).is_some() => {
                    // x / 2^j at 128 bits is a right shift pair (j ≤ 63)
                    let j = pow2j(k).unwrap();
                    let (t1, t2) = (self.asm.sc1, self.asm.sc2);
                    self.asm.srli_(t1, slo, j);
                    self.asm.slli_(t2, shi, 64 - j);
                    self.asm.r(0, t2, t1, 6, slo, 0b0110011); // or slo, t1, t2
                    self.asm.srli_(shi, shi, j);
                }
                _ => {
                    // general constant divisor: the shared routine with
                    // V = (k, 0) — the reservation exists (LxOp::Div on a
                    // 128 slot, body_needs_divmod's XOP arm)
                    let s = self.divmod_save_off;
                    debug_assert!(s >= 0, "divmod call without a reserved save area");
                    self.asm.sd(slo, S0, s);
                    self.asm.sd(shi, S0, s + 8);
                    let t = self.asm.sc1;
                    self.asm.li(t, k);
                    self.asm.sd(t, S0, s + 16);
                    self.asm.sd(ZERO, S0, s + 24);
                    let lbl = self.u128_divmod_label();
                    self.asm.sd(RA, S0, s + 128);
                    self.asm.jal_(RA, lbl);
                    self.asm.ld(RA, S0, s + 128);
                    self.asm.ld(slo, S0, s + 32);
                    self.asm.ld(shi, S0, s + 40);
                }
            },
            other => {
                return errf!(
                    "rv64 lower: opcode XOP is outside the translatable kernel subset \
                     (compound {other:?} on a u128 slot)"
                )
            }
        }
        let off = self.slot_offset(idx);
        self.asm.sd(slo, S0, off);
        self.asm.sd(shi, S0, off + 8);
        self.fwd_int[idx as usize] = None;
        Ok(())
    }

    /// The shared 128/128 restoring-division routine: q = D / V, r = D % V
    /// in one 128-iteration shift-subtract loop. All operand and result
    /// traffic goes through the frame workspace at `divmod_save_off`
    /// (layout in new_mode_arena_bank): the caller `sd`s D to s+0/s+8 and
    /// V to s+16/s+24 and parks the kernel-entry RA at s+128 around the
    /// `jal`; the routine writes q to s+32/s+40 and r to s+48/s+56. This
    /// keeps the convention identical in all three modes — mandatory
    /// because in ImcRemap `self.a1` IS `sc2` (x31), so a register-level
    /// D/V handoff clobbers itself. Working registers (all mode-external,
    /// saved to s+64..128): D = T2:T1 (x7:x6), V = x24:x23, rem = a1:a0,
    /// q = T4:T3 (x29:x28), counter = x14, wrap carry = x22 (s6, a bank
    /// candidate in ImcRemap), transient = x17 (a7 — only the exit
    /// epilogue ever writes it). x23/x24 are BANK_NONWINDOW candidates,
    /// hence saved/restored. RA is never written here (internal jumps use
    /// rd=ZERO), so the post-jal RA the routine's `ret` needs stays intact.
    /// V == 0 → fault_arith (90, the interpreter's divide-by-zero face).
    pub(crate) fn emit_divmod_routine(&mut self) {
        let s = self.divmod_save_off;
        const VLO: u8 = 23;
        const VHI: u8 = 24;
        const CARRY: u8 = 22; // wrap carry (saved; s6 — never a mode scratch)
        const TMP: u8 = 17; // a7 — dead mid-body in every mode
        debug_assert!(s >= 0, "divmod routine without a reserved save area");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        let saved = [T1, T2, 14u8, VLO, VHI, CARRY, T3, T4];
        // save the working registers BEFORE the operand loads overwrite
        // T1/T2/VLO/VHI (they may be pool or bank homes in some modes)
        for (i, &r) in saved.iter().enumerate() {
            sd(self, r, s + 64 + (i as i32) * 8);
        }
        // fetch the operands from the frame workspace
        ld(self, T1, s);
        ld(self, T2, s + 8);
        ld(self, VLO, s + 16);
        ld(self, VHI, s + 24);
        // V == 0 → 90 (the interpreter's divide-by-zero face)
        self.asm.or_(TMP, VLO, VHI);
        self.asm.beq(TMP, ZERO, self.fault_arith);
        // rem = 0, q = 0, i = 128
        self.asm.addi(self.a0, ZERO, 0);
        self.asm.addi(self.a1, ZERO, 0);
        self.asm.addi(T3, ZERO, 0);
        self.asm.addi(T4, ZERO, 0);
        self.asm.addi(14, ZERO, 128);
        let l_loop = self.asm.new_label();
        let l_sub = self.asm.new_label();
        let l_hi = self.asm.new_label();
        let l_q0 = self.asm.new_label();
        let l_next = self.asm.new_label();
        self.place(l_loop);
        // c = msb(rem) BEFORE the doubling: a wrapped 2·rem+bit always
        // exceeds V and its wrapped residue is < V, so the plain compare
        // would misread it — the carry forces the subtract instead
        self.asm.srli_(CARRY, self.a1, 63);
        // rem = rem << 1 | msb(D)
        self.asm.srli_(TMP, self.a0, 63);
        self.asm.slli_(self.a1, self.a1, 1);
        self.asm.or_(self.a1, self.a1, TMP);
        self.asm.srli_(TMP, T2, 63); // D msb
        self.asm.slli_(self.a0, self.a0, 1);
        self.asm.or_(self.a0, self.a0, TMP);
        // D <<= 1
        self.asm.srli_(TMP, T1, 63);
        self.asm.slli_(T2, T2, 1);
        self.asm.or_(T2, T2, TMP);
        self.asm.slli_(T1, T1, 1);
        // decide: carry → subtract; else compare (rem, V) as 128-bit
        self.asm.bne(CARRY, ZERO, l_sub);
        self.asm.bne(self.a1, VHI, l_hi);
        sltu(self, TMP, self.a0, VLO); // hi equal: rem < V iff lo < vl
        self.asm.beq(TMP, ZERO, l_sub); // lo ≥ vl → subtract
        self.asm.jal_(ZERO, l_q0); // no subtract
        self.place(l_hi);
        sltu(self, TMP, VHI, self.a1); // hi differ: subtract iff vh < rh
        self.asm.beq(TMP, ZERO, l_q0); // vh > rh → no subtract
        self.place(l_sub);
        // rem -= V (128-bit, borrow chain)
        sltu(self, TMP, self.a0, VLO);
        self.asm.sub(self.a0, self.a0, VLO);
        self.asm.sub(self.a1, self.a1, VHI);
        self.asm.sub(self.a1, self.a1, TMP);
        // q = q << 1 | 1
        self.asm.srli_(TMP, T3, 63);
        self.asm.slli_(T4, T4, 1);
        self.asm.or_(T4, T4, TMP);
        self.asm.slli_(T3, T3, 1);
        self.asm.addi(T3, T3, 1);
        self.asm.jal_(ZERO, l_next);
        self.place(l_q0);
        // q = q << 1 (bit 0)
        self.asm.srli_(TMP, T3, 63);
        self.asm.slli_(T4, T4, 1);
        self.asm.or_(T4, T4, TMP);
        self.asm.slli_(T3, T3, 1);
        self.place(l_next);
        self.asm.addi(14, 14, -1);
        self.asm.bne(14, ZERO, l_loop);
        // park both results in the frame workspace before restoring
        sd(self, T3, s + 32); // quotient lo
        sd(self, T4, s + 40); // quotient hi
        sd(self, self.a0, s + 48); // remainder lo
        sd(self, self.a1, s + 56); // remainder hi
        for (i, &r) in saved.iter().enumerate() {
            ld(self, r, s + 64 + (i as i32) * 8);
        }
        self.asm.ret();
    }
}
