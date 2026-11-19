use super::super::*;

impl<'a> Lower<'a> {

    /* ---- T1/T9/T12 (pure-compute subset extension) ------------------- */

    /// T12: TNIL/TIS/TID/TLIST/TMAP — the interpreter peeks the operand's
    /// Value type and REPLACES the stack top with the answer (Bool, or the
    /// U8 type id for TID). The kernel's static type model answers the
    /// constant cases; a may-Nil operand reads its tag byte instead (Nil's
    /// type id is 0). The operand expression always evaluates FIRST — its
    /// fault faces are the interpreter's and fire before the type peek.
    pub(crate) fn emit_type_test(&mut self, op: Bytecode, node: &dyn IRNode) -> Ret<()> {
        use Bytecode::*;
        let sub: &dyn IRNode = match op {
            TIS => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeParam1Single>()
                    .ok_or("rv64 lower: TIS node shape unexpected")?;
                // the interpreter parses the type-id param before the peek
                // (InstParamsErr 25 on an unknown id); the bytecode verifier
                // rejects unknown ids even earlier, so such a program never
                // runs — fail fast with the same face
                if crate::value::parse_value_ty_param(n.para).is_err() {
                    return errf!(
                        "rv64 lower: opcode TIS is outside the translatable kernel subset \
                         (unknown type id {} — InstParamsErr(25))",
                        n.para
                    );
                }
                &*n.subx
            }
            _ => {
                let n = node
                    .as_any()
                    .downcast_ref::<IRNodeSingle>()
                    .ok_or("rv64 lower: type-test node shape unexpected")?;
                &*n.subx
            }
        };
        let ty = node_res_ty(sub, self.slot)?;
        // the operand's static type id (value/type.rs: Bool=1, U8..U128 =
        // 2..6, Bytes=8, Address=9); Tuple/Compo/Handle values never exist
        // in the kernel, so those ids fold to constants below
        let sid: u8 = match ty {
            Ty::Bytes => 8,
            // T13 integration: `as address` values exist now — the
            // interpreter's ValueTy::Address id (TID/`is 9` fold)
            Ty::Address => 9,
            Ty::List | Ty::Map => {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (a container value cannot be used as a value)"
                )
            }
            Ty::Int => match self.int_width(sub) {
                Some(1) => 1,
                Some(8) => 2,
                Some(16) => 3,
                Some(32) => 4,
                Some(64) => 5,
                Some(128) => 6,
                _ => {
                    return errf!(
                        "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                         (the operand's static type is unknown — a bool-ish mixed flow cannot \
                         answer a type test)"
                    )
                }
            },
        };
        let para: u8 = if op == TIS {
            node.as_any()
                .downcast_ref::<IRNodeParam1Single>()
                .expect("TIS shape checked above")
                .para
        } else {
            0
        };
        let mnil = self.nil_val(sub);
        // evaluate the operand first (identical fault order) — an Address
        // value lowers through the same 21-content-byte (ptr, len) pair
        // descriptor as bytes (T13; the stain fix's slot re-reads land here)
        if ty == Ty::Bytes || ty == Ty::Address {
            self.eval_bytes(sub)?;
        } else {
            self.eval_int(sub)?;
        }
        let entries = match ty {
            Ty::Bytes | Ty::Address => 2,
            _ => Self::entries_of_width(self.int_width(sub)),
        };
        let tag_d = match self.vstack.len().checked_sub(entries) {
            Some(d) => d,
            None => {
                return errf!(
                    "rv64 lower: expression nesting exceeds the 8-value register pool \
                     (kernel subset limit)"
                )
            }
        };
        // TLIST/TMAP: no kernel value is ever a compo (list escapes reject;
        // Nil is not a compo) — the constant answer is false, even may-Nil
        let needs_tag = mnil && matches!(op, TNIL | TID | TIS);
        if needs_tag {
            // the tag read must happen while the operand still occupies its
            // position (tags are keyed by vstack depth)
            let sc = self.asm.sc1;
            self.asm.lbu(sc, S0, self.tag_base + tag_d as i32);
            for _ in 0..entries {
                let _ = self.pop_vreg();
            }
            let d = self.push_vreg();
            match op {
                TNIL => {
                    self.asm.i(1, sc, 3, d, 0b0010011); // seqz d, sc (tag 0 = Nil)
                }
                TIS => {
                    if para == 0 {
                        // `is` the Nil type: same as TNIL
                        self.asm.i(1, sc, 3, d, 0b0010011); // seqz
                    } else if para == sid {
                        self.asm.r(0, sc, ZERO, 3, d, 0b0110011); // snez (non-Nil → this type)
                    } else {
                        self.asm.addi(d, ZERO, 0);
                    }
                }
                TID => {
                    // d = (tag != 0) ? sid : 0 — mask = -(tag != 0)
                    self.asm.r(0, sc, ZERO, 3, d, 0b0110011); // d = (sc != 0)
                    self.asm.sub(d, ZERO, d); // d = -(sc != 0)
                    if sid != 0 {
                        let sc2 = self.asm.sc2;
                        self.asm.addi(sc2, ZERO, sid as i32);
                        self.asm.and_(d, d, sc2);
                    }
                }
                _ => unreachable!("needs_tag covers TLIST/TMAP out"),
            }
        } else {
            // constant answer: TLIST/TMAP are always false in the kernel
            let k: u64 = match op {
                TNIL => 0,
                TIS => (para != 0 && para == sid) as u64,
                TID => sid as u64,
                _ => 0,
            };
            for _ in 0..entries {
                let _ = self.pop_vreg();
            }
            let d = self.push_vreg();
            self.asm.addi(d, ZERO, k as i32);
        }
        Ok(())
    }

    /// T1: SQRT (floor) / SQRTUP (ceil) — strict `extract_u128` on the
    /// interpreter (non-uint → CastParamFail 72), the result cast back to
    /// the operand's own width (`cast_uint_result1`, Arithmetic 90 on
    /// overflow — mathematically unreachable for a w-bit input but the
    /// check is emitted as the exact interpreter face). Software bitwise
    /// isqrt; the u128 operand runs the pair-domain variant.
    pub(crate) fn emit_sqrt(&mut self, op: Bytecode, n: &IRNodeSingle) -> Ret<()> {
        let w = match self.int_width(&*n.subx) {
            Some(w) if (8..=128).contains(&w) => w,
            _ => {
                return errf!(
                    "rv64 lower: opcode {op:?} is outside the translatable kernel subset \
                     (the operand must be a statically sized uint — the interpreter's strict \
                     extract_u128 faults CastParamFail(72) on anything else)"
                )
            }
        };
        let mnil = self.nil_val(&*n.subx);
        self.eval_int(&*n.subx)?;
        if mnil {
            // Nil → the strict extractor's CastParamFail(72)
            let depth = match self.vstack.len().checked_sub(Self::entries_of_width(Some(w))) {
                Some(d) => d,
                None => {
                    return errf!(
                        "rv64 lower: expression nesting exceeds the 8-value register pool \
                         (kernel subset limit)"
                    )
                }
            };
            self.nil_guard_tag(depth, 72);
        }
        if w == 128 {
            self.emit_sqrt128(op)?;
        } else {
            self.emit_sqrt64(op, w);
        }
        Ok(())
    }

    /// u64-domain bitwise isqrt: try each bit from 2^31 down, clearing it
    /// when r*r > n. r < 2^32 so the square fits one u64 mul exactly.
    /// Registers: d = n (kept for the ceil remainder), a0 = r, a1 = bit,
    /// sc1 = square temp — four distinct registers in every mode.
    pub(crate) fn emit_sqrt64(&mut self, op: Bytecode, w: u32) {
        let d = self.pop_own_replace();
        // A8: the gated form — the same body rides the shared routine, the
        // site stages sd(operand) + `jal a0` + ld(result) through the
        // sqrt frame workspace. The nil-72 guard (in emit_sqrt) and the
        // w<64 range check below stay at the site, in order.
        if match op {
            Bytecode::SQRTUP => self.sqrt64up_routine,
            _ => self.sqrt64_routine,
        } {
            let s = self.sqrt_save_off;
            debug_assert!(s >= 0, "sqrt64 site without a reserved save area");
            self.asm.sd(d, S0, s);
            let lbl = if op == Bytecode::SQRTUP {
                *self.sqrt64up_lbl.get_or_insert_with(|| self.asm.new_label())
            } else {
                *self.sqrt64_lbl.get_or_insert_with(|| self.asm.new_label())
            };
            self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
            self.asm.ld(d, S0, s + 16);
            if w < 64 {
                self.emit_range_check(d, w);
            }
            return;
        }
        let (a0, a1) = (self.a0, self.a1);
        let sc1 = self.asm.sc1;
        self.asm.addi(a0, ZERO, 0);
        self.asm.li(a1, 1u64 << 31);
        let l_loop = self.asm.new_label();
        let l_keep = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(a1, ZERO, l_done);
        self.asm.or_(a0, a0, a1);
        self.asm.r(1, a0, a0, 0, sc1, 0b0110011); // mul sc1, r, r
        // A5 (survey arith.md): tail branch inverted — `keep` is the taken
        // branch (n >= r*r) and the clear path is the fall-through into the
        // xor, dropping the unconditional jal. Pure control-flow inversion:
        // the taken/not-taken sets per input are unchanged (bltu d,sc1 ==
        // complement of bgeu d,sc1), no fault or value face moves.
        self.asm.bgeu(d, sc1, l_keep); // n >= r*r → keep the bit
        self.asm.xor(a0, a0, a1);
        self.place(l_keep);
        self.asm.srli_(a1, a1, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // a0 = isqrt(n)
        if op == Bytecode::SQRTUP {
            // ceil tail: RECOMPUTE r*r fresh — the loop's last mul belongs
            // to the TESTED value (r|bit), which is stale by one step when
            // the final bit was cleared (the founding-era ceil64
            // divergence: even perfect squares 0/4/16/2^62 returned
            // floor+1; odd roots kept their last bit so sc1 was fresh and
            // agreed). Same shape as the u128 variant, which always
            // recomputed. rem = n - r*r is borrow-safe on the fresh square
            // (r*r <= n) and the interpreter's sqrt_up is the minimal r
            // with r*r >= n (instruction.rs sqrt_up_checked: f·f==n → f,
            // else f+1).
            self.asm.r(1, a0, a0, 0, sc1, 0b0110011); // mul sc1, r, r (fresh)
            self.asm.sub(d, d, sc1); // d = n - r*r (borrow-safe: r*r <= n)
            self.asm.r(0, d, ZERO, 3, sc1, 0b0110011); // snez sc1, d
            self.asm.add(a0, a0, sc1); // ceil
        }
        self.asm.mv(d, a0);
        if w < 64 {
            // cast_uint_result1's overflow face (never taken for a w-bit
            // input — isqrt/ceilsqrt of 2^w-1 stays below 2^w — but kept
            // as the exact interpreter fault shape)
            self.emit_range_check(d, w);
        }
    }

    /// The u128 pair-domain variant. The operand pair stays pushed while
    /// two work registers (r, bit) ride above it; r < 2^64 so r*r needs
    /// mul+mulhu. sc1/sc2 hold the square halves (never pool positions).
    pub(crate) fn emit_sqrt128(&mut self, op: Bytecode) -> Ret<()> {
        let base = match self.vstack.len().checked_sub(2) {
            Some(b) => b,
            None => {
                return errf!(
                    "rv64 lower: expression nesting exceeds the 8-value register pool \
                     (kernel subset limit)"
                )
            }
        };
        let dlo = loc_reg(self.vstack[base]);
        let dhi = loc_reg(self.vstack[base + 1]);
        // A8: the gated form — the site stages the pair through the sqrt
        // frame workspace (sd lo/hi + `jal a0` + pop both entries + push a
        // fresh result pair + ld both halves), the same stack handback
        // shape as the inline form (pop the operand entries, push the
        // result pair) without the work registers riding above the pair.
        if match op {
            Bytecode::SQRTUP => self.sqrt128up_routine,
            _ => self.sqrt128_routine,
        } {
            let s = self.sqrt_save_off;
            debug_assert!(s >= 0, "sqrt128 site without a reserved save area");
            self.asm.sd(dlo, S0, s);
            self.asm.sd(dhi, S0, s + 8);
            let lbl = if op == Bytecode::SQRTUP {
                *self.sqrt128up_lbl.get_or_insert_with(|| self.asm.new_label())
            } else {
                *self.sqrt128_lbl.get_or_insert_with(|| self.asm.new_label())
            };
            self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
            let _ = self.pop_vreg(); // dhi entry
            let _ = self.pop_vreg(); // dlo entry
            let rlo = self.push_vreg();
            let rhi = self.push_vreg();
            self.asm.ld(rlo, S0, s + 16);
            self.asm.ld(rhi, S0, s + 24);
            return Ok(());
        }
        let rr = self.push_vreg(); // r
        let kk = self.push_vreg(); // bit
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        self.asm.addi(rr, ZERO, 0);
        self.asm.li(kk, 1u64 << 63);
        let l_loop = self.asm.new_label();
        let l_clear = self.asm.new_label();
        let l_keep = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(kk, ZERO, l_done);
        self.asm.or_(rr, rr, kk);
        self.asm.r(1, rr, rr, 0, sc1, 0b0110011); // mul sc1, r, r
        self.asm.r(1, rr, rr, 3, sc2, 0b0110011); // mulhu sc2, r, r
        // sq > n iff sc2 > dhi, or sc2 == dhi && sc1 > dlo
        self.asm.bltu(dhi, sc2, l_clear);
        self.asm.bne(dhi, sc2, l_keep);
        // A5 (survey arith.md): equal-high tail inverted — `keep` is the
        // taken branch (dlo >= sc1) and the clear path is the fall-through
        // into the xor, dropping the unconditional jal. bltu dlo,sc1 is the
        // exact complement of bgeu dlo,sc1, so the taken/not-taken sets per
        // input are unchanged; the first bltu still targets l_clear.
        self.asm.bgeu(dlo, sc1, l_keep);
        self.place(l_clear);
        self.asm.xor(rr, rr, kk);
        self.place(l_keep);
        self.asm.srli_(kk, kk, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // result halves: (r, 0) for the floor; the ceil's +1 can wrap the
        // low half exactly at n = 2^128-1 (ceil = 2^64), so the carry goes
        // to the high half (sc1 = sum lo, sc2 = carry)
        let (lo_src, hi_src) = if op == Bytecode::SQRTUP {
            // recompute r*r — the loop's last mul happened BEFORE the final
            // bit was (maybe) cleared, so sc1/sc2 may hold a stale square
            self.asm.r(1, rr, rr, 0, sc1, 0b0110011); // mul sc1, r, r
            self.asm.r(1, rr, rr, 3, sc2, 0b0110011); // mulhu sc2, r, r
            // rem = n - r*r; kk is dead after the loop — reuse as the borrow
            self.asm.r(0, sc1, dlo, 3, kk, 0b0110011); // sltu kk, dlo, sc1 (borrow)
            self.asm.sub(sc1, dlo, sc1); // rem_lo
            self.asm.sub(sc2, dhi, sc2);
            self.asm.sub(sc2, sc2, kk); // rem_hi
            self.asm.or_(kk, sc1, sc2);
            self.asm.r(0, kk, ZERO, 3, kk, 0b0110011); // snez kk, kk (=1 if rem)
            self.asm.add(sc1, rr, kk); // sum = r + rem!=0 (may wrap)
            self.asm.r(0, rr, sc1, 3, sc2, 0b0110011); // sltu sc2, sc1, rr (carry)
            (sc1, sc2)
        } else {
            (rr, ZERO)
        };
        let _ = self.pop_vreg(); // kk
        let _ = self.pop_vreg(); // rr
        let _ = self.pop_vreg(); // dhi entry
        let _ = self.pop_vreg(); // dlo entry
        let rlo = self.push_vreg();
        let rhi = self.push_vreg();
        if rlo != lo_src {
            self.asm.mv(rlo, lo_src);
        }
        if rhi != hi_src {
            self.asm.mv(rhi, hi_src);
        }
        Ok(())
    }

    /* ---- A8: the shared sqrt routine bodies --------------------------- */

    /// A8: the shared u64-domain sqrt routine — the SAME bitwise-isqrt
    /// body as the inline `emit_sqrt64` (bit scan from 2^31 down, the A5
    /// inverted tail, the ceil tail inside for the SQRTUP variant), with
    /// the operand/result riding the 64-byte frame workspace at
    /// `sqrt_save_off` instead of the pool register d. Workers: n = T1
    /// (x6), r = T2 (x7), bit = T3 (x28) — all pool members in Im/Imc and
    /// pool-or-bank homes in ImcRemap, so they are saved/restored around
    /// the compute (the mul128 routine's D4 save-set discipline); the
    /// square temp sc1 is dead at every call site (the A9 premise).
    /// `a0` is never written — the `jr a0` return reads the caller's D1
    /// link directly. The fault paths don't exist inside (isqrt never
    /// faults); the routine's own faces are the site's range check and
    /// nil guard, which stay at the site.
    pub(crate) fn emit_sqrt64_routine(&mut self, op: Bytecode) {
        let s = self.sqrt_save_off;
        debug_assert!(s >= 0, "sqrt64 routine without a reserved save area");
        const N: u8 = 6; // T1
        const R: u8 = 7; // T2
        const BIT: u8 = 28; // T3
        let sc1 = self.asm.sc1;
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        sd(self, N, s + 32);
        sd(self, R, s + 40);
        sd(self, BIT, s + 48);
        ld(self, N, s); // n
        self.asm.addi(R, ZERO, 0); // r = 0
        self.asm.li(BIT, 1u64 << 31); // bit = 2^31
        let l_loop = self.asm.new_label();
        let l_keep = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(BIT, ZERO, l_done);
        self.asm.or_(R, R, BIT);
        self.asm.r(1, R, R, 0, sc1, 0b0110011); // mul sc1, r, r
        // A5 inverted tail, same as the inline body (keep is the taken
        // branch; clear is the fall-through into the xor)
        self.asm.bgeu(N, sc1, l_keep); // n >= r*r → keep the bit
        self.asm.xor(R, R, BIT);
        self.place(l_keep);
        self.asm.srli_(BIT, BIT, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // R = isqrt(n); the ceil tail is part of the routine (the same
        // FIXED shape as the inline body: recompute R*R fresh — the
        // loop's last square belongs to the TESTED value R|bit, stale by
        // one step when the final bit was cleared, which used to return
        // floor+1 on the even perfect squares; the fresh square makes
        // rem = n - R*R borrow-safe and exact)
        if op == Bytecode::SQRTUP {
            self.asm.r(1, R, R, 0, sc1, 0b0110011); // mul sc1, r, r (fresh)
            self.asm.sub(N, N, sc1); // n - r*r (borrow-safe: r*r <= n)
            self.asm.r(0, N, ZERO, 3, sc1, 0b0110011); // snez sc1, n-r*r
            self.asm.add(R, R, sc1); // ceil
        }
        sd(self, R, s + 16); // result
        ld(self, N, s + 32);
        ld(self, R, s + 40);
        ld(self, BIT, s + 48);
        self.asm.jr(self.a0); // D1: return through the link register
    }

    /// A8: the shared u128 pair-domain sqrt routine — the SAME loop as the
    /// inline `emit_sqrt128` (bit scan from 2^63 down, mul+mulhu square,
    /// the A5 inverted equal-high tail, the SQRTUP ceil tail with the
    /// recomputed square and the 128-bit remainder subtract inside), with
    /// the operand/result pair on the frame workspace. Workers: rr = T1
    /// (x6), kk = T2 (x7), dlo = T3 (x28), dhi = T4 (x29) — the mul128
    /// routine's save-set class (all pool members in Im/Imc; pool-or-bank
    /// homes in ImcRemap), saved/restored around the compute; sc1/sc2 are
    /// dead at every call site. a0 is never written (the D1 link).
    pub(crate) fn emit_sqrt128_routine(&mut self, op: Bytecode) {
        let s = self.sqrt_save_off;
        debug_assert!(s >= 0, "sqrt128 routine without a reserved save area");
        const RR: u8 = 6; // T1
        const KK: u8 = 7; // T2
        const DLO: u8 = 28; // T3
        const DHI: u8 = 29; // T4
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        sd(self, RR, s + 32);
        sd(self, KK, s + 40);
        sd(self, DLO, s + 48);
        sd(self, DHI, s + 56);
        ld(self, DLO, s); // n lo
        ld(self, DHI, s + 8); // n hi
        self.asm.addi(RR, ZERO, 0); // r = 0
        self.asm.li(KK, 1u64 << 63); // bit = 2^63
        let l_loop = self.asm.new_label();
        let l_clear = self.asm.new_label();
        let l_keep = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(KK, ZERO, l_done);
        self.asm.or_(RR, RR, KK);
        self.asm.r(1, RR, RR, 0, sc1, 0b0110011); // mul sc1, r, r
        self.asm.r(1, RR, RR, 3, sc2, 0b0110011); // mulhu sc2, r, r
        // sq > n iff sc2 > dhi, or sc2 == dhi && sc1 > dlo — the A5
        // inverted equal-high tail, same as the inline body
        self.asm.bltu(DHI, sc2, l_clear);
        self.asm.bne(DHI, sc2, l_keep);
        self.asm.bgeu(DLO, sc1, l_keep);
        self.place(l_clear);
        self.asm.xor(RR, RR, KK);
        self.place(l_keep);
        self.asm.srli_(KK, KK, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        // result halves: (r, 0) for the floor; the ceil's +1 may wrap the
        // low half at n = 2^128-1, so the carry goes to the high half —
        // the exact inline tail (the loop's last mul predates the final
        // bit clear, so the square is recomputed)
        if op == Bytecode::SQRTUP {
            self.asm.r(1, RR, RR, 0, sc1, 0b0110011); // mul sc1, r, r
            self.asm.r(1, RR, RR, 3, sc2, 0b0110011); // mulhu sc2, r, r
            self.asm.r(0, sc1, DLO, 3, KK, 0b0110011); // sltu kk, dlo, sc1 (borrow)
            self.asm.sub(sc1, DLO, sc1); // rem_lo
            self.asm.sub(sc2, DHI, sc2); // rem_hi
            self.asm.sub(sc2, sc2, KK); // - borrow
            self.asm.or_(KK, sc1, sc2);
            self.asm.r(0, KK, ZERO, 3, KK, 0b0110011); // snez kk, kk (=1 if rem)
            self.asm.add(sc1, RR, KK); // sum = r + rem!=0 (may wrap)
            self.asm.r(0, RR, sc1, 3, sc2, 0b0110011); // sltu sc2, sc1, rr (carry)
            sd(self, sc1, s + 16); // result lo
            sd(self, sc2, s + 24); // result hi (carry)
        } else {
            sd(self, RR, s + 16); // result lo
            sd(self, ZERO, s + 24); // result hi = 0
        }
        ld(self, RR, s + 32);
        ld(self, KK, s + 40);
        ld(self, DLO, s + 48);
        ld(self, DHI, s + 56);
        self.asm.jr(self.a0); // D1: return through the link register
    }

    /// T9: XLG — `local[idx] OP rhs` with the Bool result replacing the
    /// rhs on the stack. mark>>5 selects And/Or/Eq/Ne/Gt/Ge/Lt/Le, the low
    /// 5 bits the local slot. And/Or use the interpreter's truthiness
    /// (uint≠0, Nil→false — the kernel's may-Nil payload is always 0, so
    /// `payload != 0` is exact); Eq/Ne use `value_content_eq` (numeric
    /// across uint widths, Bool×Bool; exactly-one-Nil → Arithmetic 90);
    /// the ordered four are uint-only (any non-uint, Nil included → 90).
    pub(crate) fn emit_xlg(&mut self, n: &IRNodeParam1Single, whole: &dyn IRNode) -> Ret<()> {
        use LxLg::*;
        let (lg, idx) = decode_local_logic_mark(n.para);
        let wl = self.slot_w.get(idx as usize).copied().flatten();
        let wr = self.int_width(&*n.subx);
        let rhs_bytes = node_res_ty(&*n.subx, self.slot)? == Ty::Bytes;
        // the lhs old value's may-Nil state (recorded by nil_plan like
        // XOP's, under the same xop_old field)
        let lhs_nil = self
            .nil_occ
            .get(&node_addr(whole))
            .map(|o| o.xop_old)
            .unwrap_or(false);
        let rhs_nil = self.nil_val(&*n.subx);
        match lg {
            And | Or => {
                if rhs_bytes {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (bytes truthiness scans the content — unsupported for the local \
                         logic ops)"
                    );
                }
                self.eval_int(&*n.subx)?;
                self.xlg_load_lhs(idx, wl);
                let truthy = |lw: &mut Self, r: u8| {
                    lw.asm.r(0, r, ZERO, 3, r, 0b0110011); // sltu r, zero, r
                };
                let d = match (wl == Some(128), wr == Some(128)) {
                    (false, false) => {
                        let rb = self.pop_reg(); // lhs
                        let d = self.pop_own_replace(); // rhs
                        truthy(self, d);
                        truthy(self, rb);
                        if lg == And {
                            self.asm.and_(d, d, rb);
                        } else {
                            self.asm.or_(d, d, rb);
                        }
                        d
                    }
                    (true, false) => {
                        let lhi = self.pop_reg();
                        let llo = self.pop_reg();
                        self.asm.or_(llo, llo, lhi);
                        truthy(self, llo);
                        let d = self.pop_own_replace(); // rhs
                        truthy(self, d);
                        if lg == And {
                            self.asm.and_(d, d, llo);
                        } else {
                            self.asm.or_(d, d, llo);
                        }
                        d
                    }
                    (false, true) => {
                        let rb = self.pop_reg(); // lhs
                        truthy(self, rb);
                        let rhi = self.pop_reg();
                        let d = self.pop_own_replace(); // rhs lo
                        self.asm.or_(d, d, rhi);
                        truthy(self, d);
                        if lg == And {
                            self.asm.and_(d, d, rb);
                        } else {
                            self.asm.or_(d, d, rb);
                        }
                        d
                    }
                    (true, true) => {
                        let lhi = self.pop_reg();
                        let llo = self.pop_reg();
                        self.asm.or_(llo, llo, lhi);
                        truthy(self, llo);
                        let rhi = self.pop_reg();
                        let d = self.pop_own_replace(); // rhs lo
                        self.asm.or_(d, d, rhi);
                        truthy(self, d);
                        if lg == And {
                            self.asm.and_(d, d, llo);
                        } else {
                            self.asm.or_(d, d, llo);
                        }
                        d
                    }
                };
                let _ = d;
            }
            Eq | Ne => {
                // value_content_eq: uint×uint numeric (cross-width ok),
                // Bool×Bool; every other static pair faults 90 at run time
                // on the interpreter (different types) — fail fast
                if rhs_bytes {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (bytes content equality is not supported for the local logic ops)"
                    );
                }
                let lbool = wl == Some(1);
                let rbool = wr == Some(1);
                if wl.is_none() || wr.is_none() || lbool != rbool {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (value_content_eq faults Arithmetic(90) on mixed bool/uint types)"
                    );
                }
                self.eval_int(&*n.subx)?;
                // exactly-one-Nil → Arithmetic(90); Nil×Nil compares equal
                // (both payloads are 0 in the kernel)
                let er = Self::entries_of_width(wr);
                if lhs_nil || rhs_nil {
                    let t90 = self.nil_thunk(90);
                    let s1 = self.asm.sc1;
                    if lhs_nil && rhs_nil {
                        let s2 = self.asm.sc2;
                        self.asm.lbu(s1, S0, self.flag_off[idx as usize]);
                        let td = match self.vstack.len().checked_sub(er) {
                            Some(d) => d,
                            None => {
                                return errf!(
                                    "rv64 lower: expression nesting exceeds the 8-value \
                                     register pool (kernel subset limit)"
                                )
                            }
                        };
                        self.asm.lbu(s2, S0, self.tag_base + td as i32);
                        self.asm.bne(s1, s2, t90);
                    } else if lhs_nil {
                        self.asm.lbu(s1, S0, self.flag_off[idx as usize]);
                        self.asm.beq(s1, ZERO, t90);
                    } else {
                        let td = match self.vstack.len().checked_sub(er) {
                            Some(d) => d,
                            None => {
                                return errf!(
                                    "rv64 lower: expression nesting exceeds the 8-value \
                                     register pool (kernel subset limit)"
                                )
                            }
                        };
                        self.asm.lbu(s1, S0, self.tag_base + td as i32);
                        self.asm.beq(s1, ZERO, t90);
                    }
                }
                self.xlg_load_lhs(idx, wl);
                let cmp_op = if lg == Eq { Bytecode::EQ } else { Bytecode::NEQ };
                if wl == Some(128) || wr == Some(128) {
                    let b = self.pop_u128_pair(wr); // rhs
                    let a = self.pop_u128_pair(wl); // lhs
                    let d = self.push_vreg();
                    self.emit_u128_cmp(cmp_op, d, a, b);
                } else {
                    let rb = self.pop_reg(); // lhs
                    let d = self.pop_own_replace(); // rhs
                    self.asm.xor(d, d, rb);
                    if lg == Eq {
                        self.asm.i(1, d, 3, d, 0b0010011); // seqz
                    } else {
                        self.asm.r(0, d, ZERO, 3, d, 0b0110011); // snez
                    }
                }
            }
            Gt | Ge | Lt | Le => {
                // ordered compare: uint-only (lgc_ord_cmp faults
                // Arithmetic(90) on any non-uint, Nil included) — fail fast
                // on the statically non-uint faces, guard the may-Nil ones
                if rhs_bytes || wl == Some(1) || wr == Some(1) || wl.is_none() || wr.is_none() {
                    return errf!(
                        "rv64 lower: opcode XLG is outside the translatable kernel subset \
                         (the ordered local logic ops need uint operands — the interpreter \
                         faults Arithmetic(90) otherwise)"
                    );
                }
                self.eval_int(&*n.subx)?;
                let er = Self::entries_of_width(wr);
                let mut tag_depth = None;
                if rhs_nil {
                    tag_depth = Some(match self.vstack.len().checked_sub(er) {
                        Some(d) => d,
                        None => {
                            return errf!(
                                "rv64 lower: expression nesting exceeds the 8-value \
                                 register pool (kernel subset limit)"
                            )
                        }
                    });
                }
                if lhs_nil {
                    let t90 = self.nil_thunk(90);
                    let s1 = self.asm.sc1;
                    self.asm.lbu(s1, S0, self.flag_off[idx as usize]);
                    self.asm.beq(s1, ZERO, t90);
                }
                if let Some(td) = tag_depth {
                    let t90 = self.nil_thunk(90);
                    let s1 = self.asm.sc1;
                    self.asm.lbu(s1, S0, self.tag_base + td as i32);
                    self.asm.beq(s1, ZERO, t90);
                }
                self.xlg_load_lhs(idx, wl);
                let cmp_op = match lg {
                    Gt => Bytecode::GT,
                    Ge => Bytecode::GE,
                    Lt => Bytecode::LT,
                    _ => Bytecode::LE,
                };
                if wl == Some(128) || wr == Some(128) {
                    let b = self.pop_u128_pair(wr); // rhs
                    let a = self.pop_u128_pair(wl); // lhs
                    let d = self.push_vreg();
                    self.emit_u128_cmp(cmp_op, d, a, b);
                } else {
                    // a = lhs (rb), b = rhs (d); sltu computes a < b
                    let rb = self.pop_reg(); // lhs
                    let d = self.pop_own_replace(); // rhs
                    match lg {
                        Lt => self.asm.r(0, d, rb, 3, d, 0b0110011), // sltu d, a, b
                        Gt => self.asm.r(0, rb, d, 3, d, 0b0110011), // sltu d, b, a
                        Le => {
                            self.asm.r(0, rb, d, 3, d, 0b0110011); // b < a
                            self.asm.i(1, d, 4, d, 0b0010011); // xori (invert)
                        }
                        Ge => {
                            self.asm.r(0, d, rb, 3, d, 0b0110011); // a < b
                            self.asm.i(1, d, 4, d, 0b0010011); // xori (invert)
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        Ok(())
    }

    /// Load local slot `idx` (XLG's lhs) onto the vstack as owned pool
    /// entries — a (lo, hi) pair for a u128 slot, one register otherwise.
    pub(crate) fn xlg_load_lhs(&mut self, idx: u8, wl: Option<u32>) {
        if wl == Some(128) {
            // u128 slots are never banked — always a 16-byte frame cell
            let off = self.slot_offset(idx);
            let lo = self.push_vreg();
            let hi = self.push_vreg();
            self.asm.ld(lo, S0, off);
            self.asm.ld(hi, S0, off + 8);
        } else if let Some(r) = self.bank_int_read(idx) {
            let l = self.push_vreg();
            self.asm.mv(l, r);
        } else {
            let off = self.slot_offset(idx);
            let l = self.push_vreg();
            self.asm.ld(l, S0, off);
        }
    }
}
