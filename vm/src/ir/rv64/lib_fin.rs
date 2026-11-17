use super::*;

impl<'a> Lower<'a> {

    /* ================================================================== */
    /* ---- T26: the FIN* family lowering -------------------------------- */
    /*                                                                    */
    /* Emit twins of the interpreter's fin kernels (instruction.rs         */
    /* fin2/3/4/p3/p4/pow3_checked + their helpers): every operand goes    */
    /* through to_uint (a non-uint/Nil operand faults CastFail 71 — the   */
    /* guards below), the operands zero-extend to their common width      */
    /* (arithmetic_argsN — the staging below materializes full writable   */
    /* (lo, hi) pairs), the kernel computes through u256 intermediates    */
    /* on the P1 library, and the u128 result casts back to that common   */
    /* width (an overflow is Arithmetic 90 — the fit-back check).         */
    /* FIN3/u64_mad keeps the ORIGINAL base width instead of the common   */
    /* one (triop_fin's special case). FINP3/FINP4 return Bool.           */
    /*                                                                    */
    /* Register discipline: after the operand pops the pool slots hold    */
    /* the operand pairs; every kernel consumes them in an order that     */
    /* keeps each register live until its last read. Values that must     */
    /* survive a divmod call (whose fresh result pushes land in the low   */
    /* pool slots) park in the FIN workspace slots A..F behind the RA     */
    /* park — the P1 helpers never touch those bytes.                     */

    /// Park a u128 register pair into a FIN workspace slot.
    pub(crate) fn fin_park(&mut self, slot: i32, p: (u8, u8)) {
        let s = self.u256_save_off + slot;
        self.asm.sd(p.0, S0, s);
        self.asm.sd(p.1, S0, s + 8);
    }

    /// Load a FIN workspace slot into an explicit register pair (the
    /// caller guarantees both registers are dead; plain loads, no push).
    pub(crate) fn fin_load(&mut self, slot: i32, p: (u8, u8)) {
        let s = self.u256_save_off + slot;
        self.asm.ld(p.0, S0, s);
        self.asm.ld(p.1, S0, s + 8);
    }

    /// Evaluate one FIN operand and stage it as a full OWNED (lo, hi)
    /// register pair: `emit_u256_mul_wide` repurposes all four operand
    /// registers as the product limbs, so neither a ZERO hi limb (narrow
    /// values) nor a read-only bank Ref is admissible — narrow operands
    /// get a pushed zero hi, Refs copy into fresh pool registers (the
    /// T5 staging precedent).
    pub(crate) fn eval_fin_arg(&mut self, node: &dyn IRNode, w: Option<u32>) -> Ret<()> {
        self.eval_int(node)?;
        // A may-Nil operand's tag rides the LOW entry of its staged pair;
        // the re-push below moves the value to new vstack positions, so the
        // tag byte must move with it or the FIN 71 guard would read a stale
        // position (and a definite `nil` would not fault).
        let src_tag = if self.nil_val(node) && self.tag_base >= 0 {
            Some(self.vstack.len() - if w == Some(128) { 2 } else { 1 })
        } else {
            None
        };
        if w == Some(128) {
            let hi = self.pop_vreg();
            let lo = self.pop_vreg();
            let plo = self.push_vreg();
            if plo != loc_reg(lo) {
                self.asm.mv(plo, loc_reg(lo));
            }
            let phi = self.push_vreg();
            if phi != loc_reg(hi) {
                self.asm.mv(phi, loc_reg(hi));
            }
        } else {
            let lo = self.pop_vreg();
            let plo = self.push_vreg();
            if plo != loc_reg(lo) {
                self.asm.mv(plo, loc_reg(lo));
            }
            let phi = self.push_vreg();
            self.asm.addi(phi, ZERO, 0);
        }
        if let Some(src) = src_tag {
            self.nil_copy_tag(src, self.vstack.len() - 2);
        }
        Ok(())
    }

    /// Nil guards for the staged operand pairs: every operand's to_uint
    /// faults CastFail 71 before any kernel face (the tag rides the LOW
    /// entry of each pair). BUG-021 hardening: a shrunken-pool body bails
    /// with the depth reject.
    pub(crate) fn fin_nil_guards(&mut self, flags: &[bool]) -> Ret<()> {
        let total = flags.len() * 2;
        let base = match self.vstack.len().checked_sub(total) {
            Some(b) => b,
            None => {
                return errf!(
                    "rv64 lower: expression nesting exceeds the 8-value register pool (kernel subset limit)"
                );
            }
        };
        for (i, &f) in flags.iter().enumerate() {
            if f {
                self.nil_guard_tag(base + i * 2, 71);
            }
        }
        Ok(())
    }

    /// Pop one staged operand pair (the hi entry first).
    pub(crate) fn fin_pop_pair(&mut self) -> (u8, u8) {
        let hi = self.pop_reg();
        let lo = self.pop_reg();
        (lo, hi)
    }

    /// Stage a FIN node's arguments. With no may-Nil operand the arguments
    /// are evaluated in source order and parked immediately into this
    /// node's nesting-level area, then reloaded as owned pairs right before
    /// the kernel — the value stack never holds more than one argument pair,
    /// so a NESTED FIN argument sees the full 8-entry pool (the historical
    /// all-on-stack staging would need 4 + 6 = 10 entries for
    /// `u64_mad(s, 1, mul_div(...))`). A may-Nil operand's tag lives at its
    /// vstack position and must not move into the untagged park slots, so
    /// those bodies keep the plain on-stack staging (and its pool limit; a
    /// deeper form rejects cleanly rather than miscompiling).
    pub(crate) fn fin_stage_args(&mut self, args: &[(&dyn IRNode, u32)], may_nil: bool) -> Ret<()> {
        if may_nil {
            for &(node, w) in args {
                self.eval_fin_arg(node, Some(w))?;
            }
            return Ok(());
        }
        debug_assert!(self.fin_depth < self.fin_park_levels.max(1));
        let base = FIN_PARK_BASE + self.fin_depth as i32 * FIN_PARK_LEVEL_BYTES;
        self.fin_depth += 1;
        for (i, &(node, w)) in args.iter().enumerate() {
            self.eval_fin_arg(node, Some(w))?;
            let p = self.fin_pop_pair();
            self.fin_park(base + i as i32 * 16, p);
        }
        for i in 0..args.len() {
            let lo = self.push_vreg();
            let hi = self.push_vreg();
            self.fin_load(base + i as i32 * 16, (lo, hi));
        }
        self.fin_depth -= 1;
        Ok(())
    }

    /// A FIN operand's static width: bool-ish/widthless (None / Some(1))
    /// and sub-byte widths have no interpreter face the kernel subset can
    /// reproduce (the interpreter's to_uint faults 71 on bools — the
    /* pinned D5 family is deliberately NOT inherited here: FIN rejects. */
    pub(crate) fn fin_arg_width(&self, w: Option<u32>, name: &str) -> Ret<u32> {
        match w {
            Some(w) if (8..=128).contains(&w) && w != 1 => Ok(w),
            _ => errf!(
                "rv64 lower: fin kernel {name} is outside the translatable kernel subset \
                 (operands need static uint widths, got {w:?})"
            ),
        }
    }

    /// The fit-back face + vstack publication of a u128 kernel result: the
    /// interpreter casts back to the common width `w` (cast_uint_result*
    /// → Arithmetic 90 when out ≥ 2^w), then the value takes 1 entry (w <
    /// 128) or the (lo, hi) pair (w == 128).
    pub(crate) fn emit_fin_publish(&mut self, lo: u8, hi: u8, w: u32) {
        if w < 128 {
            self.asm.bne(hi, ZERO, self.fault_arith);
        }
        if w < 64 {
            self.emit_range_check(lo, w);
        }
        if w == 128 {
            let plo = self.push_vreg();
            if plo != lo {
                self.asm.mv(plo, lo);
            }
            let phi = self.push_vreg();
            if phi != hi {
                self.asm.mv(phi, hi);
            }
        } else {
            let plo = self.push_vreg();
            if plo != lo {
                self.asm.mv(plo, lo);
            }
        }
    }


    /// `floor = isqrt(n)` of the 4-limb value parked at `slot` (4
    /// consecutive u64 limbs, low first) — the emit twin of
    /// `isqrt_u256_floor`'s digit-by-digit loop (bit from 1<<127 down).
    /// Working state parks behind it (OUT/BIT/CAND at slot+32/+48/+64);
    /// ALL 8 pool registers are used as scratch (the caller runs this
    /// with the op's operands consumed). Returns the floor as a plain
    /// (lo, hi) pair in the two lowest pool registers.
    pub(crate) fn emit_fin_isqrt256(&mut self, slot: i32) -> Ret<(u8, u8)> {
        let s = self.u256_save_off;
        let out = slot + 32;
        let bit = slot + 48;
        let cand = slot + 64;
        // out = 0; bit = 1 << 127
        self.asm.sd(ZERO, S0, s + out);
        self.asm.sd(ZERO, S0, s + out + 8);
        let sc1 = self.asm.sc1;
        self.asm.addi(sc1, ZERO, 0);
        self.asm.sd(sc1, S0, s + bit);
        self.asm.li(sc1, 0x8000_0000_0000_0000);
        self.asm.sd(sc1, S0, s + bit + 8);
        // eight scratch pool registers (the caller consumed the op's
        // operands; the floor pair stays as the two lowest entries)
        let t0 = self.push_vreg();
        let t1 = self.push_vreg();
        let t2 = self.push_vreg();
        let t3 = self.push_vreg();
        let t4 = self.push_vreg();
        let t5 = self.push_vreg();
        let t6 = self.push_vreg();
        let t7 = self.push_vreg();
        let l_loop = self.asm.new_label();
        let l_accept = self.asm.new_label();
        let l_skip = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        // bit != 0 ?
        self.asm.ld(t2, S0, s + bit);
        self.asm.ld(t3, S0, s + bit + 8);
        self.asm.or_(sc1, t2, t3);
        self.asm.beq(sc1, ZERO, l_done);
        // cand = out | bit
        self.asm.ld(t0, S0, s + out);
        self.asm.ld(t1, S0, s + out + 8);
        self.asm.or_(t0, t0, t2);
        self.asm.or_(t1, t1, t3);
        self.asm.sd(t0, S0, s + cand);
        self.asm.sd(t1, S0, s + cand + 8);
        // sq = cand * cand (limbs land in [t0,t1,t4,t5])
        self.asm.mv(t4, t0);
        self.asm.mv(t5, t1);
        self.emit_u256_mul_wide((t0, t1), (t4, t5))?;
        // n limbs -> (t2, t3, t6, t7); accept iff sq <= n
        self.asm.ld(t2, S0, s + slot);
        self.asm.ld(t3, S0, s + slot + 8);
        self.asm.ld(t6, S0, s + slot + 16);
        self.asm.ld(t7, S0, s + slot + 24);
        let l_eq = self.asm.new_label();
        self.emit_u256_branch3(
            &[t0, t1, t4, t5],
            &[t2, t3, t6, t7],
            l_accept,
            l_eq,
            l_skip,
        );
        // sq < n -> accept; sq == n -> accept (the interpreter's <=)
        self.place(l_eq);
        self.place(l_accept);
        self.asm.ld(t0, S0, s + cand);
        self.asm.ld(t1, S0, s + cand + 8);
        self.asm.sd(t0, S0, s + out);
        self.asm.sd(t1, S0, s + out + 8);
        self.place(l_skip);
        // bit >>= 1 (128-bit: t3 = high limb, t2 = low limb). The bit
        // crossing the limb boundary is the HIGH limb's LSB, which becomes
        // the LOW limb's MSB (getting this backwards drops every low-half
        // digit after 64 shifts and makes the root read as 0).
        self.asm.ld(t2, S0, s + bit);
        self.asm.ld(t3, S0, s + bit + 8);
        let sc2 = self.asm.sc2;
        self.asm.andi(sc2, t3, 1);
        self.asm.slli_(sc2, sc2, 63);
        self.asm.srli_(t3, t3, 1);
        self.asm.srli_(t2, t2, 1);
        self.asm.or_(t2, t2, sc2);
        self.asm.sd(t2, S0, s + bit);
        self.asm.sd(t3, S0, s + bit + 8);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // floor -> (t0, t1); release the six upper scratches
        self.asm.ld(t0, S0, s + out);
        self.asm.ld(t1, S0, s + out + 8);
        let _ = self.pop_reg(); // t7
        let _ = self.pop_reg(); // t6
        let _ = self.pop_reg(); // t5
        let _ = self.pop_reg(); // t4
        let _ = self.pop_reg(); // t3
        let _ = self.pop_reg(); // t2
        Ok((t0, t1))
    }

    /// The owned register pair of the two vstack entries at `pos`.
    pub(crate) fn fin_pair_at(&self, pos: usize) -> (u8, u8) {
        (
            loc_reg(*self.vstack.get(pos).expect("fin operand entry")),
            loc_reg(*self.vstack.get(pos + 1).expect("fin operand entry")),
        )
    }

    /// The wavg2 rounding tail (the interpreter's
    /// `div_u256_by_u129_with_round` call with `base` as the half-even
    /// parity offset). Specialized — instead of calling the P1
    /// `emit_u129_round_quot` — because all eight pool slots hold live
    /// values at this point (q, rem, and the five-entry push ceiling);
    /// the divisor and the parity base load lazily from their FIN parks
    /// (`den` lo pair + bit, `base`), the tie branch reusing the dead
    /// divisor registers for the base's low limb.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_wavg2_round(
        &mut self,
        policy: FinRoundPolicy,
        q: (u8, u8),
        r_lo: (u8, u8),
        rh: u8,
        scratch: [u8; 3],
    ) -> Ret<()> {
        let a0 = self.a0;
        let l_done = self.asm.new_label();
        let round_up = |lw: &mut Self| {
            let t = lw.a0;
            lw.asm.addi(t, ZERO, 1);
            lw.emit_u128_add(q.0, q.1, t, ZERO); // checked → 90 on carry
        };
        match policy {
            FinRoundPolicy::Floor => {}
            FinRoundPolicy::Exact => {
                self.asm.or_(a0, r_lo.0, r_lo.1);
                self.asm.or_(a0, a0, rh);
                self.asm.bne(a0, ZERO, self.fault_arith);
            }
            FinRoundPolicy::Ceil => {
                self.asm.or_(a0, r_lo.0, r_lo.1);
                self.asm.or_(a0, a0, rh);
                self.asm.beq(a0, ZERO, l_done);
                round_up(self);
            }
            FinRoundPolicy::HalfUp | FinRoundPolicy::HalfEven => {
                // dbl = rem + rem (a 129-bit value doubled in place —
                // cannot overflow the three limbs)
                let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                self.asm.srli_(sc1, r_lo.1, 63);
                self.asm.srli_(sc2, r_lo.0, 63);
                self.asm.slli_(rh, rh, 1);
                self.asm.or_(rh, rh, sc1);
                self.asm.slli_(r_lo.1, r_lo.1, 1);
                self.asm.or_(r_lo.1, r_lo.1, sc2);
                self.asm.slli_(r_lo.0, r_lo.0, 1);
                // divisor (dh, d1, d0) from the parks into the scratch
                let s = self.u256_save_off;
                let (t0, t1, t2) = (scratch[0], scratch[1], scratch[2]);
                self.asm.ld(t0, S0, s + U256WS_FIND);
                self.asm.ld(t1, S0, s + U256WS_FIND + 8);
                self.asm.ld(t2, S0, s + U256WS_FINE);
                if policy == FinRoundPolicy::HalfUp {
                    let l_up = self.asm.new_label();
                    self.emit_u129v_branch3((rh, r_lo.1, r_lo.0), (t2, t1, t0), l_up, l_up, l_done);
                    self.place(l_up);
                    round_up(self);
                } else {
                    let l_up = self.asm.new_label();
                    let l_tie = self.asm.new_label();
                    self.emit_u129v_branch3((rh, r_lo.1, r_lo.0), (t2, t1, t0), l_up, l_tie, l_done);
                    self.place(l_tie);
                    // tie → against the FINAL result's parity:
                    // (quo & 1) != (base & 1) → up. The divisor registers
                    // are dead here — reuse t0 for base's low limb.
                    self.asm.ld(t0, S0, s + U256WS_FINF);
                    self.asm.xor(sc1, q.0, t0);
                    self.asm.andi(sc1, sc1, 1);
                    self.asm.beq(sc1, ZERO, l_done);
                    self.place(l_up);
                    round_up(self);
                }
            }
        }
        self.place(l_done);
        Ok(())
    }
}
