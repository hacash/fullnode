use super::*;

/// A2/A4 (survey arith.md): a full-range u128 literal. `int_literal`
/// (web.rs) caps at u64::MAX — the right bound for the u64 paths, but the
/// 128-bit operand paths have legal constants above it (`x / 2^100`).
/// Same node family as `int_literal` (pure int literals; cast-wrapped
/// forms fall through and keep the general lowering).
pub(crate) fn u128_literal(node: &dyn IRNode) -> Option<u128> {
    crate::lang::ir_literal_value(node)
        .ok()
        .flatten()
        .and_then(|v| v.extract_u128().ok())
}

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
        if self.mul128_routine {
            // A9: the shared routine (the site count reached the
            // amortization threshold; mul128_call preserves this
            // function's d-mutation contract)
            self.mul128_call(dlo, dhi, rlo, rhi);
            return;
        }
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

    /// D8 (survey fin-control.md): the branch-fused u128 compare — branch
    /// to `label` iff `(a CMP b) == when_true`, with NO 0/1 byte
    /// materialized (the `emit_u128_cmp` + `bne/beq` pair loses both the
    /// byte's vreg and 1-2 instructions per conditional site).
    ///
    /// Faces are the same numeric-order / content-equality semantics as
    /// [`Self::emit_u128_cmp`] (both pairs already zero-extended by the
    /// caller); only the branch shape differs. The ordered form mirrors
    /// the u64 arm's (f3, swap) mapping and splits the decision on the
    /// high halves' equality — three instructions:
    /// ```text
    /// bne    x_hi, y_hi, l_hi      // halves differ → the hi compare decides
    /// branch f3,  x_lo, y_lo, lbl  // hi equal → the lo compare decides
    /// l_hi: bltu <hi order>, lbl   // (see below)
    /// ```
    /// The `l_hi` arm is ALWAYS a `bltu` (f3 = 6), with the operand order
    /// picked by the effective decision: for f3 = 6 ("branch when x < y")
    /// it is `bltu x_hi, y_hi`; for f3 = 7 ("branch when x >= y") it is
    /// `bltu y_hi, x_hi` (x >= y with differing halves ⇔ x_hi > y_hi ⇔
    /// bltu y_hi, x_hi). The bltu-only tail is load-bearing: the
    /// hi-equal path whose lo compare is NOT taken falls THROUGH into
    /// `l_hi` (no room for a skip jump in the 3-slot shape), and a bltu
    /// on EQUAL halves is false — exactly the "overall false" verdict
    /// that fall-through must produce. A bgeu tail would read as TRUE on
    /// the spurious entry (equal halves) and misbranch — caught by the
    /// d8 differential corpus before it shipped.
    ///
    /// EQ/NEQ split the same way: unequal on ANY half → not equal, so the
    /// "branch when different" form is two `bne`s straight to the label
    /// and the "branch when equal" form falls through two `bne`s into a
    /// `jal` to the label.
    ///
    /// (The survey's other D8 half — direct-writing the second `sltu`
    /// into `d` inside [`Self::emit_u128_cmp`] to drop its tail `mv` —
    /// is NOT taken: `d` is handed positionally after the operand pops,
    /// so it can recycle a popped operand register (typically a's lo);
    /// the direct-write form reads operand halves in later instructions
    /// after `d` was already clobbered, which is unsound without a
    /// d-distinctness guarantee the shared callers (fin.rs, lib_u256.rs
    /// pass fixed registers) do not provide. The fused form has no `d`
    /// at all and reaches −2/−3 per conditional site — strictly more
    /// than the −1 the direct-write could offer.)
    pub(crate) fn emit_u128_cmp_branch(
        &mut self,
        op: Bytecode,
        when_true: bool,
        label: usize,
        a: (u8, u8),
        b: (u8, u8),
    ) {
        let (alo, ahi) = a;
        let (blo, bhi) = b;
        match op {
            Bytecode::EQ | Bytecode::NEQ => {
                if (op == Bytecode::EQ) == when_true {
                    // branch when EQUAL: any half differing skips it
                    let l_ne = self.asm.new_label();
                    self.asm.bne(alo, blo, l_ne);
                    self.asm.bne(ahi, bhi, l_ne);
                    self.asm.jal_(ZERO, label);
                    self.place(l_ne);
                } else {
                    // branch when DIFFERENT: either half differing branches
                    self.asm.bne(alo, blo, label);
                    self.asm.bne(ahi, bhi, label);
                }
            }
            _ => {
                // ordered: (f3, swap) = the branch-when-TRUE form, the
                // same mapping as the u64 arm (f3 ^ 1 inverts for
                // when_false)
                let (f3, swap) = match op {
                    Bytecode::LT => (6u32, false), // bltu a, b
                    Bytecode::GT => (6, true),     // a > b  <=>  bltu b, a
                    Bytecode::LE => (7, true),     // a <= b <=>  bgeu b, a
                    _ => (7, false),               // a >= b <=>  bgeu a, b
                };
                let f3 = if when_true { f3 } else { f3 ^ 1 };
                let (xlo, xhi, ylo, yhi) = if swap {
                    (blo, bhi, alo, ahi)
                } else {
                    (alo, ahi, blo, bhi)
                };
                let l_hi = self.asm.new_label();
                self.asm.bne(xhi, yhi, l_hi);
                self.asm.branch(f3, xlo, ylo, label);
                self.place(l_hi);
                // hi differ (or the spurious hi-equal fall-through, which
                // any bltu-on-equal verdicts false): bltu only, operands
                // per the effective decision — see the doc comment
                if f3 == 6 {
                    self.asm.branch(6, xhi, yhi, label); // x < y
                } else {
                    self.asm.branch(6, yhi, xhi, label); // x_hi > y_hi
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

    /// A9: the shared u128 multiply routine's label (lazy, same pattern as
    /// [`Self::u128_divmod_label`] — emitted after the body only when a
    /// gated site staged through it).
    pub(crate) fn u128_mul_label(&mut self) -> usize {
        *self.mul128_lbl.get_or_insert_with(|| self.asm.new_label())
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
                // A2 (survey arith.md): constant DIV/MOD divisors absorb
                // before the rhs evaluates — the u128 route fires before
                // the u64 constant-absorption path, and the XOP arm
                // (emit_xop128_k) already folds; this repairs the
                // expression-path asymmetry. Zero → 90, 1 → identity,
                // pow2 → shift/mask; other constants keep the routine.
                if matches!(op, DIV | MOD) {
                    if let Some(k) =
                        u128_literal(&*n.suby).filter(|&k| k == 0 || k == 1 || k.is_power_of_two())
                    {
                        let lx = self.nil_val(&*n.subx);
                        self.eval_int(&*n.subx)?;
                        let len = self.vstack.len();
                        if lx {
                            // the same 71 face on the same entry (the
                            // lhs sits alone on the stack; a literal rhs
                            // never evaluates and never carries a tag)
                            self.nil_guard_tag(len - Self::entries_of_width(wx), 71);
                        }
                        let lhs = self.pop_u128_pair(wx);
                        return self.emit_u128_divmod_k(op, lhs, k);
                    }
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
                // A4 (survey arith.md): a constant shift amount specializes
                // before the rhs evaluates (the eval.rs u128 route fires
                // BEFORE the u64 constant-absorption path — this is the
                // 128-bit counterpart of the arith_k BSHL/BSHR arms).
                if matches!(op, BSHL | BSHR) {
                    if let Some(k) = u128_literal(&*n.suby) {
                        let lx = self.nil_val(&*n.subx);
                        self.eval_int(&*n.subx)?;
                        let len = self.vstack.len();
                        if lx {
                            // the same 71 face on the same entry: the lhs
                            // sits alone on the stack (a literal rhs never
                            // evaluates and never carries a tag, so the
                            // general path's ly guard is unreachable here)
                            self.nil_guard_tag(len - Self::entries_of_width(wx), 71);
                        }
                        let a = self.pop_u128_pair(wx);
                        return self.emit_u128_shift_k(op, a, k);
                    }
                }
                // A2: constant DIVUP/DIVEXACT divisors — zero → 90,
                // 1 → identity, pow2 → the shift+mask round forms; the
                // shared routine call (and its 66-slot body) drops away
                // when this is the body's only division site.
                if matches!(op, DIVUP | DIVEXACT) {
                    if let Some(k) = u128_literal(&*n.suby)
                        .filter(|&k| k == 0 || k == 1 || k.is_power_of_two())
                    {
                        let lx = self.nil_val(&*n.subx);
                        self.eval_int(&*n.subx)?;
                        let len = self.vstack.len();
                        if lx {
                            self.nil_guard_tag(len - Self::entries_of_width(wx), 71);
                        }
                        let lhs = self.pop_u128_pair(wx);
                        return self.emit_u128_divup_exact_k(op, lhs, k);
                    }
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
    /// s+16/s+24; the routine writes q to s+32/s+40 and r to s+48/s+56.
    /// D1: the call links through the mode scratch a0 (`jal a0` — dead at
    /// every routine call site in every mode; the routines clobber it from
    /// entry anyway) and the routine parks/restores its own return address
    /// at s+128, so the call site saves/restores nothing and RA is never
    /// touched (−2 instructions, −8B per call site). Divide-by-zero faults
    /// 90 from inside the routine — the fault path skips the routine's
    /// restores exactly like the old call-site `ld RA` was skipped: the
    /// kernel exits whole, same observable face.
    pub(crate) fn emit_u128_divmod(&mut self, want_mod: bool, dlo: u8, dhi: u8, rlo: u8, rhi: u8) -> Ret<()> {
        let s = self.divmod_save_off;
        debug_assert!(s >= 0, "divmod call without a reserved save area");
        self.asm.sd(dlo, S0, s);
        self.asm.sd(dhi, S0, s + 8);
        self.asm.sd(rlo, S0, s + 16);
        self.asm.sd(rhi, S0, s + 24);
        let lbl = self.u128_divmod_label();
        self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
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
        self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
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

    /// A2 (survey arith.md): `x / k` / `x % k` at 128 bits with a CONSTANT
    /// divisor — the expression-path counterpart of the already-folded XOP
    /// arm ([`Self::emit_xop128_k`]; the eval.rs u128 route fires before
    /// the u64 constant absorption, so `x / 1000u128` paid the full
    /// routine call + the 66-slot shared routine for no reason). Faces,
    /// per the interpreter's `div_checked`/`rem_checked` (`<u128>::
    /// checked_div/rem` — their ONLY fault is divide-by-zero → 90):
    /// · k == 0 → Arithmetic 90 unconditionally, after the lhs's to_uint
    ///   71 guard (order preserved; the routine's runtime V == 0 check
    ///   faults on exactly the same inputs);
    /// · k != 0 → NO fault face at all (a constant divisor is nonzero by
    ///   proof), and the u128 division/mask is exact — pow2 shift/mask
    ///   semantics are identical to the interpreter's u128 division.
    /// Non-pow2 k >= 2 is NOT intercepted (the caller filters): the
    /// general routine call stays (A3's magic-reciprocal is a loss).
    /// `lhs` is the value pair (a narrow side arrives zero-extended,
    /// read-only); the result pair is pushed fresh.
    pub(crate) fn emit_u128_divmod_k(&mut self, op: Bytecode, lhs: (u8, u8), k: u128) -> Ret<()> {
        let (alo, ahi) = lhs;
        let (t1, _) = (self.asm.sc1, self.asm.sc2);
        if k == 0 {
            // zero constant divisor → 90 (no value side to compute)
            self.asm.jal_(ZERO, self.fault_arith);
            // dead result for the vstack balance (never executed — the
            // same dead-code-after-jal shape as emit_xop128_k's k == 0)
            let out_lo = self.push_vreg();
            let out_hi = self.push_vreg();
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if out_hi != ahi {
                self.asm.mv(out_hi, ahi);
            }
            return Ok(());
        }
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        if op == Bytecode::DIV {
            if k == 1 {
                // x / 1 = x — identity, no face
                if out_lo != alo {
                    self.asm.mv(out_lo, alo);
                }
                if out_hi != ahi {
                    self.asm.mv(out_hi, ahi);
                }
                return Ok(());
            }
            // x / 2^j: the shift pair (j <= 127 — k < 2^128)
            let j = k.trailing_zeros();
            if j < 64 {
                // lo' = (lo >> j) | (hi << (64−j)); hi' = hi >> j
                self.asm.slli_(t1, ahi, (64 - j) as u32);
                self.asm.srli_(out_lo, alo, j);
                self.asm.or_(out_lo, out_lo, t1);
                self.asm.srli_(out_hi, ahi, j);
            } else {
                // lo' = hi >> (j−64); hi' = 0
                self.asm.srli_(out_lo, ahi, (j - 64) as u32);
                self.asm.addi(out_hi, ZERO, 0);
            }
            return Ok(());
        }
        // MOD
        if k == 1 {
            // x % 1 = 0
            self.asm.addi(out_lo, ZERO, 0);
            self.asm.addi(out_hi, ZERO, 0);
            return Ok(());
        }
        let j = k.trailing_zeros();
        if j < 64 {
            // lo' = lo & (2^j − 1); hi' = 0 — the mask rides `andi` only
            // when it fits the 12-bit signed window (the same three-way
            // fix as the u64 MOD pow2 arm: a truncated mask silently
            // became −1 / an identity)
            let mask = (k - 1) as u64;
            match Self::i12_mask(mask) {
                Some(m) => self.asm.andi(out_lo, alo, m),
                None => {
                    // asm.li clobbers sc1/sc2 internally (its own
                    // expansion temps) — the mask materializes into a
                    // POOL register (the li contract every other site
                    // follows)
                    let t = self.push_vreg();
                    self.asm.li(t, mask);
                    self.asm.and_(out_lo, alo, t);
                    let _ = self.pop_vreg();
                }
            }
            self.asm.addi(out_hi, ZERO, 0);
        } else {
            // x % 2^j keeps bits 0..j−1: lo unchanged, hi = hi & (2^(j−64) − 1)
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if j == 64 {
                self.asm.addi(out_hi, ZERO, 0);
            } else {
                let mask = ((k >> 64) - 1) as u64;
                match Self::i12_mask(mask) {
                    Some(m) => self.asm.andi(out_hi, ahi, m),
                    None => {
                        let t = self.push_vreg();
                        self.asm.li(t, mask);
                        self.asm.and_(out_hi, ahi, t);
                        let _ = self.pop_vreg();
                    }
                }
            }
        }
        Ok(())
    }

    /// A2: the DIVUP/DIVEXACT companion of [`Self::emit_u128_divmod_k`] —
    /// constant divisors on the round-policy forms. Faces, per the
    /// interpreter's `div_with_round_checked`:
    /// · k == 0 → 90 (the z == 0 face, before any rounding);
    /// · k == 1 → identity (DIVUP's bump sees rem == 0, DIVEXACT's exact
    ///   check passes — no face);
    /// · k = 2^j → the shift+mask round forms: DIVUP adds 1 on a nonzero
    ///   remainder (the +1 cannot leave 128 bits: a quotient of 2^128 − 1
    ///   with a nonzero remainder would need x > u128::MAX — only the lo
    ///   carry into hi needs handling), DIVEXACT faults 90 on one;
    /// · other constants keep the shared routine (caller filters).
    pub(crate) fn emit_u128_divup_exact_k(&mut self, op: Bytecode, lhs: (u8, u8), k: u128) -> Ret<()> {
        let (alo, ahi) = lhs;
        let (t1, t2) = (self.asm.sc1, self.asm.sc2);
        if k == 0 {
            // zero constant divisor → 90
            self.asm.jal_(ZERO, self.fault_arith);
            let out_lo = self.push_vreg();
            let out_hi = self.push_vreg();
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if out_hi != ahi {
                self.asm.mv(out_hi, ahi);
            }
            return Ok(());
        }
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        if k == 1 {
            // div_up(x, 1) = div_exact(x, 1) = x — identity, no face
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if out_hi != ahi {
                self.asm.mv(out_hi, ahi);
            }
            return Ok(());
        }
        let j = k.trailing_zeros(); // k = 2^j, j <= 127
        if j < 64 {
            // r = lo & (2^j − 1) (the mask fits the low half); q = x >> j
            let mask = (k - 1) as u64;
            match Self::i12_mask(mask) {
                Some(m) => self.asm.andi(t1, alo, m),
                None => {
                    let t = self.push_vreg();
                    self.asm.li(t, mask);
                    self.asm.and_(t1, alo, t);
                    let _ = self.pop_vreg();
                }
            }
            self.asm.slli_(t2, ahi, (64 - j) as u32);
            self.asm.srli_(out_lo, alo, j);
            self.asm.or_(out_lo, out_lo, t2);
            self.asm.srli_(out_hi, ahi, j);
            let l_no = self.asm.new_label();
            if op == Bytecode::DIVEXACT {
                // div_exact: a nonzero remainder faults 90
                self.asm.bne(t1, ZERO, self.fault_arith);
            } else {
                // div_up: q += 1 when r != 0, with the lo wrap's carry
                // into hi (out_lo all-ones → +1 wraps to 0)
                self.asm.beq(t1, ZERO, l_no);
                self.asm.addi(out_lo, out_lo, 1);
                self.asm.addi(self.a0, ZERO, 1);
                self.asm.r(0, self.a0, out_lo, 3, self.a0, 0b0110011); // sltu a0, out_lo, 1
                self.asm.add(out_hi, out_hi, self.a0);
                self.place(l_no);
            }
        } else {
            // j >= 64: the remainder flag computes FIRST — out_lo recycles
            // alo's register (the pop/push slot handback), so every alo
            // read must precede the quotient shift's write
            if j == 64 {
                // mask = 0: the remainder is exactly alo
                if op == Bytecode::DIVEXACT {
                    self.asm.bne(alo, ZERO, self.fault_arith);
                } else {
                    self.asm.r(0, alo, ZERO, 3, t1, 0b0110011); // sltu t1, ZERO, alo
                }
            } else {
                let mask = ((k >> 64) - 1) as u64;
                match Self::i12_mask(mask) {
                    Some(m) => self.asm.andi(t1, ahi, m),
                    None => {
                        let t = self.push_vreg();
                        self.asm.li(t, mask);
                        self.asm.and_(t1, ahi, t);
                        let _ = self.pop_vreg();
                    }
                }
                self.asm.or_(t1, alo, t1); // nonzero iff r != 0
            }
            // q = hi >> (j−64) < 2^64 (the DIVUP bump cannot carry)
            self.asm.srli_(out_lo, ahi, (j - 64) as u32);
            self.asm.addi(out_hi, ZERO, 0);
            if op == Bytecode::DIVEXACT {
                // div_exact: a nonzero remainder faults 90
                if j > 64 {
                    self.asm.bne(t1, ZERO, self.fault_arith);
                }
            } else {
                // div_up: q += 1 when r != 0 (q < 2^64 — no carry)
                let l_no = self.asm.new_label();
                self.asm.beq(t1, ZERO, l_no);
                self.asm.addi(out_lo, out_lo, 1);
                self.place(l_no);
            }
        }
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

    /// A4 (survey arith.md): `x << k` / `x >> k` at 128 bits with a
    /// CONSTANT amount — [`Self::emit_u128_shift`]'s face with the amount
    /// checks folded away. Face per the interpreter's `bit_shl`/`bit_shr`
    /// (Rust checked_shl/checked_shr on u128):
    /// · k >= 128 → Arithmetic 90 unconditionally. The amount checks are a
    ///   PURE face: the variable path's `shi != 0` / `slo >= 128` guards
    ///   fire on exactly the inputs a constant k >= 128 names, so the 90
    ///   keeps its code and its order (after the lhs's to_uint 71 guard).
    /// · k < 128 → no fault face at all; the value side truncates
    ///   silently with NO truncation instruction — the full 128-bit pair
    ///   IS the widened shift (the variable path emits none either).
    /// `a` is the value pair (a narrow side arrives zero-extended, hi =
    /// ZERO — every form below reads the zero hi correctly). The result
    /// pair is pushed FRESH: the lhs registers stay read-only (a bank
    /// Ref / the ZERO half-extend is never written).
    pub(crate) fn emit_u128_shift_k(&mut self, op: Bytecode, a: (u8, u8), k: u128) -> Ret<()> {
        let (alo, ahi) = a;
        let t1 = self.asm.sc1;
        if k >= 128 {
            // constant amount ≥ width: the interpreter's checked shift
            // faults unconditionally — no value side to compute
            self.asm.jal_(ZERO, self.fault_arith);
            // dead result for the vstack balance (never executed — the
            // fault thunk exits the kernel; the same dead-code-after-jal
            // shape as the emit_xop128_k zero-divisor site)
            let out_lo = self.push_vreg();
            let out_hi = self.push_vreg();
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if out_hi != ahi {
                self.asm.mv(out_hi, ahi);
            }
            return Ok(());
        }
        let out_lo = self.push_vreg();
        let out_hi = self.push_vreg();
        if k == 0 {
            // amount 0: the checked shift succeeds, result = x
            if out_lo != alo {
                self.asm.mv(out_lo, alo);
            }
            if out_hi != ahi {
                self.asm.mv(out_hi, ahi);
            }
            return Ok(());
        }
        let k = k as u64; // k < 128 here
        if k < 64 {
            // 0 < k < 64: the direct shift pair — the variable path's
            // `64 − s` subtraction, the >= 64 branch and the k == 0
            // branch all fold away (4 instructions vs 17)
            if op == Bytecode::BSHL {
                // lo' = lo << k; hi' = (hi << k) | (lo >> (64−k))
                self.asm.srli_(t1, alo, (64 - k) as u32);
                self.asm.slli_(out_hi, ahi, k as u32);
                self.asm.or_(out_hi, out_hi, t1);
                self.asm.slli_(out_lo, alo, k as u32);
            } else {
                // lo' = (lo >> k) | (hi << (64−k)); hi' = hi >> k
                self.asm.slli_(t1, ahi, (64 - k) as u32);
                self.asm.srli_(out_lo, alo, k as u32);
                self.asm.or_(out_lo, out_lo, t1);
                self.asm.srli_(out_hi, ahi, k as u32);
            }
        } else if op == Bytecode::BSHL {
            // 64 <= k <= 127: out_hi = alo << (k−64), out_lo = 0
            self.asm.slli_(out_hi, alo, (k - 64) as u32);
            self.asm.addi(out_lo, ZERO, 0);
        } else {
            // 64 <= k <= 127: out_lo = ahi >> (k−64), out_hi = 0
            self.asm.srli_(out_lo, ahi, (k - 64) as u32);
            self.asm.addi(out_hi, ZERO, 0);
        }
        Ok(())
    }

    /// `d = d * b` at 128 bits with the interpreter's checked_mul face (90
    /// on any product bit above 127), PRESERVING the rhs pair — the POW
    /// loop keeps its base across iterations and squares it in place, so d
    /// and r may alias. Temps: sc1/sc2/a0 only (in ImcRemap `self.a1` IS
    /// `sc2`, so exactly three distinct scratches exist; every register is
    /// written only after its last read, under d == r aliasing too).
    pub(crate) fn emit_u128_mul_keep(&mut self, dlo: u8, dhi: u8, blo: u8, bhi: u8) {
        if self.mul128_routine {
            // A9: the shared routine — the frame handoff snapshots both
            // pairs, so the d == b aliasing (POW squaring) and the
            // preserved-rhs contract hold by construction
            self.mul128_call(dlo, dhi, blo, bhi);
            return;
        }
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

    /// A9: stage one u128 multiply site through the shared routine (the
    /// frame-workspace convention of [`Self::emit_u128_divmod`]): the
    /// caller `sd`s D to s+0/s+8 and B to s+16/s+24 and jumps with
    /// `jal a0` (D1: the mode scratch as the link register — a0 is dead at
    /// every routine call site); the routine writes the 128-bit product to
    /// s+32/s+40 and the site reloads it INTO the D pair registers,
    /// preserving `emit_u128_mul`'s result-in-d contract (`emit_u128_mul_keep`'s
    /// d == b aliasing holds by construction: the frame copies snapshot
    /// both operands before the compute reads them, and B's registers are
    /// never written). The gate (`mul128_routine`) is construction-time —
    /// below the threshold the callers emit the historical inline form
    /// byte-for-byte and this helper never runs.
    fn mul128_call(&mut self, dlo: u8, dhi: u8, rlo: u8, rhi: u8) {
        let s = self.divmod_save_off;
        debug_assert!(s >= 0, "mul128 call without a reserved save area");
        self.asm.sd(dlo, S0, s);
        self.asm.sd(dhi, S0, s + 8);
        self.asm.sd(rlo, S0, s + 16);
        self.asm.sd(rhi, S0, s + 24);
        let lbl = self.u128_mul_label();
        self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
        self.asm.ld(dlo, S0, s + 32);
        self.asm.ld(dhi, S0, s + 40);
    }

    /// A9: the shared 128×128 checked-multiply routine — the merged
    /// `emit_u128_mul` / `emit_u128_mul_keep` body (both are 19-slot
    /// checked_mul lowerings differing only in register-reuse order; this
    /// form follows the mul_keep 3-temp shape). Convention: D rides the
    /// frame at s+0/s+8, B at s+16/s+24; the product lands at s+32/s+40;
    /// the call site links through a0 (`jal a0`, D1) and a0 is NEVER
    /// written inside — no park, the `jr a0` return reads the caller's
    /// link directly.
    ///
    /// Working registers (saved/restored around the compute — they may be
    /// pool homes in Im/Imc and bank homes in ImcRemap, the same D4 audit
    /// class as `emit_divmod_routine`'s T-set): D = T1:T2 (x6:x7), B =
    /// T3:T4 (x28:x29). Temps: sc1/sc2 (x15/x16, x12/x13 or x30/x31 per
    /// mode) and x17 (a7 — excluded from the ImcRemap bank candidate list,
    /// P2-A27 audit 3103669; `self.a1` is unusable as a distinct worker
    /// because in ImcRemap it IS sc2). The fault paths (mulhu ≠ 0 → 90)
    /// skip the restores exactly like the divmod routine's: the kernel
    /// exits whole, same observable face.
    ///
    /// 90-face equivalence: the interpreter's `<u128>::checked_mul` faults
    /// iff any product bit above bit 127 is nonzero — checked here as
    /// hh_hi/hh_lo (dhi·bhi), lh_hi, hl_hi and the two hi-add carries,
    /// exactly the inline forms' condition set (fault order inside the
    /// site is unobservable — every path is the same exit-90).
    pub(crate) fn emit_mul128_routine(&mut self) {
        let s = self.divmod_save_off;
        const DLO: u8 = T1; // x6
        const DHI: u8 = T2; // x7
        const BLO: u8 = T3; // x28
        const BHI: u8 = T4; // x29
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        const X17: u8 = 17; // a7 — the unsaved scratch (see the audit note)
        debug_assert!(s >= 0, "mul128 routine without a reserved save area");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        let mulhu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(1, rs2, rs1, 3, rd, 0b0110011);
        };
        let mul = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(1, rs2, rs1, 0, rd, 0b0110011);
        };
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        // save the worker registers BEFORE the operand loads overwrite
        // them (pool/bank homes in every mode — the divmod routine's D4
        // audit covers this same class; no mode-specialization is possible
        // because T1..T4 are pool members in Im/Imc too)
        for (i, &r) in [DLO, DHI, BLO, BHI].iter().enumerate() {
            sd(self, r, s + 64 + (i as i32) * 8);
        }
        ld(self, DLO, s);
        ld(self, DHI, s + 8);
        ld(self, BLO, s + 16);
        ld(self, BHI, s + 24);
        // hh = dhi·bhi must vanish entirely (bits ≥ 128)
        mulhu(self, sc1, DHI, BHI); // hh_hi
        mulhu(self, sc2, DLO, BHI); // lh_hi
        mul(self, X17, DLO, BHI); // lh_lo
        self.asm.bne(sc1, ZERO, self.fault_arith); // hh_hi
        mul(self, sc1, DHI, BHI); // hh_lo — BHI's last read
        self.asm.bne(sc1, ZERO, self.fault_arith);
        self.asm.bne(sc2, ZERO, self.fault_arith); // lh_hi
        // hl = dhi·blo (hi must vanish), ll = dlo·blo → the product
        mulhu(self, sc1, DHI, BLO); // hl_hi
        self.asm.bne(sc1, ZERO, self.fault_arith);
        mul(self, DHI, DHI, BLO); // hl_lo — DHI repurposed
        mulhu(self, sc1, DLO, BLO); // ll_hi
        mul(self, sc2, DLO, BLO); // ll_lo — DLO's last read
        // hi = hl_lo + lh_lo + ll_hi with carries; lo = ll_lo
        self.asm.add(DHI, DHI, X17);
        sltu(self, X17, DHI, X17); // carry c1
        self.asm.add(DHI, DHI, sc1); // + ll_hi
        sltu(self, sc1, DHI, sc1); // carry c2
        self.asm.or_(X17, X17, sc1);
        self.asm.bne(X17, ZERO, self.fault_arith);
        // park both result halves before restoring
        sd(self, sc2, s + 32); // product lo
        sd(self, DHI, s + 40); // product hi
        for (i, &r) in [DLO, DHI, BLO, BHI].iter().enumerate() {
            ld(self, r, s + 64 + (i as i32) * 8);
        }
        self.asm.jr(self.a0); // D1: return through the link register
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
                // A11 (survey arith.md): a large constant rides the literal
                // pool via li_k (smaller ones fall back to the plain li —
                // never worse); t is a POOL register, disjoint from the
                // sc1/sc2/a0/a1 carry/product temps the helpers use.
                self.li_k(t, k);
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
                    // A11: literal-pool materialization for large constants
                    self.li_k(t, k);
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
                    // A11: literal-pool materialization for large constants;
                    // the pool base register (x2, ImcRemap only) is untouched
                    // by the routine, and the ld happens before the jal.
                    self.li_k(t, k);
                    self.asm.sd(t, S0, s + 16);
                    self.asm.sd(ZERO, S0, s + 24);
                    let lbl = self.u128_divmod_label();
                    self.asm.jal_(self.a0, lbl); // D1: a0-linked call — RA stays intact
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

    /// The shared 128/128 division routine: q = D / V, r = D % V in one
    /// call. All operand and result traffic goes through the frame
    /// workspace at `divmod_save_off` (layout in new_mode_arena_bank): the
    /// caller `sd`s D to s+0/s+8 and V to s+16/s+24 and jumps with
    /// `jal a0` (D1: the mode scratch as the link register — the routine
    /// parks/restores it at s+128 and returns through it, so call sites
    /// save nothing and RA is never touched); the routine writes q to
    /// s+32/s+40 and r to s+48/s+56. This keeps the convention identical
    /// in all three modes — mandatory because in ImcRemap `self.a1` IS
    /// `sc2` (x31), so a register-level D/V handoff clobbers itself.
    /// Working registers (all mode-external, saved to s+64..128): the
    /// dividend low half shifts in T1 (x6), rem = a1:T2 (x11/x31 : x7),
    /// V = x24:x23, q = T4:T3 (x29:x28), counter = x14, wrap carry = x22
    /// (s6, a bank candidate in ImcRemap), transient = x17 (a7 — dead
    /// mid-body because the ImcRemap bank candidate list excludes it, see
    /// the TMP comment in `emit_divmod_routine`). The saved set are pool/bank candidates in
    /// some mode, hence saved/restored — mode-specialized per D4 (audit
    /// in `emit_divmod_routine`): Im/Imc save only the pool members,
    /// ImcRemap the full set. RA is never written here (internal
    /// jumps use rd=ZERO and the return rides the parked a0 link), so the
    /// body's kernel-entry RA survives every routine call untouched.
    ///
    /// A6 (docs.loc/fitsh-rv64/survey/arith.md): the monolithic
    /// 128-iteration shift loop is now a two-stage form —
    ///
    ///   stage 1 (consume dhi):   q_hi = dhi / V,   r1 = dhi % V
    ///     · VHI == 0 (V = vlo ∈ [1, 2^64-1]): one hardware divu + remu;
    ///     · VHI != 0 (V ≥ 2^64 > dhi):        q_hi = 0, r1 = dhi.
    ///   stage 2: ONE shared 64-iteration restoring loop over dlo's bits,
    ///     starting from rem = (0, r1) with q_hi parked in T4.
    ///
    /// 90-face equivalence — the death criterion, per input (also proven
    /// by the model test `a6_divmod_models_agree` and the end-to-end
    /// differential `a6_divmod_routine_matches_interpreter` below):
    /// · The interpreter's u128 DIV/MOD are `<u128>::checked_div/rem`
    ///   (vm/src/interpreter/instruction.rs, ahmtdo!) — their ONLY fault
    ///   is divide-by-zero → Arithmetic 90. A 128/128 quotient always fits
    ///   128 bits, so there is no quotient-overflow face to reproduce.
    /// · The pre-A6 loop faulted exactly on V == 0 (the or+beq below,
    ///   kept verbatim) and returned the exact (q, r) otherwise. This
    ///   routine keeps exactly that face: divu/remu never trap and run
    ///   only with vlo != 0 (V != 0 ∧ VHI == 0); the loop adds no fault.
    /// · In particular VHI == 0 ∧ dhi ≥ vlo (quotient ≥ 2^64, e.g.
    ///   D = 2^64, V = 1 → q = 2^64) must NOT fault. The survey's sketch
    ///   proposed faulting 90 there by analogy with the u256 routine's
    ///   N_hi ≥ V face, but that face exists because the u256 routine
    ///   divides a 256-bit dividend into a 128-bit quotient — here the
    ///   quotient is 128-bit for a 128-bit dividend and cannot overflow.
    ///   The old loop returns q = (dhi/vlo)·2^64 + … for those inputs, so
    ///   a fault would be a NEW face (consensus break); stage 1's divu
    ///   computes that high quotient exactly instead.
    ///
    /// Bit-exactness vs the 128-iteration loop, per input: the old loop's
    /// first 64 iterations ARE a restoring division of dhi by V (they
    /// leave rem = dhi % V and accumulate floor(dhi/V) into q's top
    /// bits); stage 1 computes those same two values directly (divu/remu
    /// when V fits one register, or trivially q_hi = 0 / r1 = dhi when
    /// V ≥ 2^64 > dhi). Stage 2 is then the old loop's remaining 64
    /// iterations verbatim: the invariant rem < V holds at every loop
    /// top (r1 < V by construction; rem' = 2·rem + bit ≤ 2V − 1 < 2^129,
    /// and subtracting V restores rem < V), the wrap of rem' across bit
    /// 128 is caught by CARRY = old msb(a1) which forces the subtract
    /// (a wrapped 2·rem + bit ≥ 2^128 > V), the pair compare/subtract
    /// chains are the old ones, and q_lo = N' / V < 2^64 never overflows
    /// T3 because N' = r1·2^64 + dlo < V·2^64. Hence q and r are
    /// bit-identical to the old loop for every (D, V) pair, and the fault
    /// sets are identical (exactly V == 0).
    pub(crate) fn emit_divmod_routine(&mut self) {
        let s = self.divmod_save_off;
        const VLO: u8 = 23;
        const VHI: u8 = 24;
        const CARRY: u8 = 22; // wrap carry (saved; s6 — never a mode scratch)
        const TMP: u8 = 17; // a7 — unsaved scratch. DEAD mid-body ONLY
        // because x17 is excluded from the ImcRemap bank candidate list
        // (mod.rs P2-A27 — audit 3103669): this routine writes it on every
        // call (the `or TMP, VLO, VHI` V==0 check below) and throughout the
        // slow loop. Never re-add 17 to the bank candidates without adding
        // it to this routine's save set as well.
        debug_assert!(s >= 0, "divmod routine without a reserved save area");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        // D4: mode-specialized save set. VLO/VHI/CARRY (x23/x24/x22) are
        // bank-candidate registers — live slot homes in ImcRemap only. In
        // Im/Imc NOTHING outside this routine ever writes them:
        //   * the local bank (the only writer class that can target them)
        //     is built exclusively under `mode == Rv64Mode::ImcRemap` in
        //     the lower pipeline, so `plan.homes` is empty and every
        //     PUT/XOP/bank path stores through the frame instead;
        //   * the Im/Imc value pool is VSTACK_REGS = x5,x6,x7,x28,x29,
        //     x30,x31,x14 and the scratches are x15/x16 (Im) or x12/x13
        //     (Imc) plus a0/a1 = x10/x11 — no overlap;
        //   * a grep audit over the whole emit corpus (emit/*.rs,
        //     lib_*.rs, asm.rs, finish.rs) finds x3/x4/x22..x25 written
        //     ONLY as the routines' own working constants; the GT-a copy
        //     thunks write `li a0` only, `li`'s temp is sc1/sc2, the
        //     literal-pool base is x2, and finish.rs relaxation rewrites
        //     encodings (same rd), never register assignments.
        // So the saves/restores of those three registers are dead in
        // Im/Imc and dropped (−6 instructions); ImcRemap keeps the full
        // set. Cross-routine review: no routine calls another (all
        // internal jumps are rd=ZERO; the only calls are the body-side
        // `jal a0` sites), and the shared fault thunks write A0 only — a
        // dropped save here cannot break any other routine's assumptions.
        let saved: &[u8] = if self.saves_bank_candidate_regs {
            &[T1, T2, 14, VLO, VHI, CARRY, T3, T4]
        } else {
            &[T1, T2, 14, T3, T4]
        };
        // D1: park the link register FIRST — the caller's `jal a0` saved
        // nothing. The slot (s+128, the old caller RA park) is dead for
        // the routine's whole lifetime (its own saves run s+64..s+128 and
        // nothing else touches it). The fault path (trap 90 below) skips
        // the restore, exactly like the old call-site `ld RA` was skipped:
        // the kernel exits whole, same observable face.
        sd(self, self.a0, s + 128);
        // save the working registers BEFORE the operand loads overwrite
        // T1/T2/VLO/VHI (they may be pool or bank homes in some modes)
        for (i, &r) in saved.iter().enumerate() {
            sd(self, r, s + 64 + (i as i32) * 8);
        }
        // fetch the operands from the frame workspace: T1 = dlo, T2 = dhi
        ld(self, T1, s);
        ld(self, T2, s + 8);
        ld(self, VLO, s + 16);
        ld(self, VHI, s + 24);
        // V == 0 → 90 (the interpreter's divide-by-zero face — the ONLY
        // fault this routine ever raises, either version)
        self.asm.or_(TMP, VLO, VHI);
        self.asm.beq(TMP, ZERO, self.fault_arith);
        // ---- stage 1: consume dhi (T2) ----
        // V ≥ 2^64 > dhi → q_hi = 0 and r1 = dhi (T2 already holds it).
        // The divu/remu pair below runs only for VHI == 0, where
        // vlo != 0 (V != 0) — hardware division never traps.
        self.asm.addi(T4, ZERO, 0);
        let l_common = self.asm.new_label();
        self.asm.bne(VHI, ZERO, l_common);
        self.asm.r(1, VLO, T2, 5, T4, 0b0110011); // divu q_hi, dhi, vlo
        self.asm.r(1, VLO, T2, 7, T2, 0b0110011); // remu r1, dhi, vlo (T2 = rem lo from here)
        self.place(l_common);
        // ---- stage 2: the shared 64-iteration restoring loop ----
        self.asm.addi(self.a1, ZERO, 0); // rem hi = 0 (rem = a1:T2 = (0, r1))
        self.asm.addi(T3, ZERO, 0); // q lo = 0 (q hi parked in T4)
        self.asm.addi(14, ZERO, 64);
        let l_loop = self.asm.new_label();
        let l_sub = self.asm.new_label();
        let l_hi = self.asm.new_label();
        let l_next = self.asm.new_label();
        self.place(l_loop);
        // c = msb(rem) BEFORE the doubling: a wrapped 2·rem+bit always
        // exceeds V and its wrapped residue is < V, so the plain compare
        // would misread it — the carry forces the subtract instead
        self.asm.srli_(CARRY, self.a1, 63);
        // rem = rem << 1 | msb(dlo) — the pair chain lifts T2's msb into
        // a1 exactly like the old loop's rem pair
        self.asm.srli_(TMP, T2, 63);
        self.asm.slli_(self.a1, self.a1, 1);
        self.asm.or_(self.a1, self.a1, TMP);
        self.asm.srli_(TMP, T1, 63); // next dividend bit
        self.asm.slli_(T2, T2, 1);
        self.asm.or_(T2, T2, TMP);
        self.asm.slli_(T1, T1, 1); // dlo <<= 1
        // q = q << 1 — hoisted ahead of the decision (q's bit 0 is
        // tentatively 0); the subtract arm just sets it with addi 1,
        // replacing the old two-arm pair-shift blocks
        self.asm.slli_(T3, T3, 1);
        // decide: carry → subtract; else compare (rem, V) as 128-bit
        self.asm.bne(CARRY, ZERO, l_sub);
        self.asm.bne(self.a1, VHI, l_hi);
        sltu(self, TMP, T2, VLO); // hi equal: rem < V iff lo < vl
        self.asm.beq(TMP, ZERO, l_sub); // lo ≥ vl → subtract
        // hi equal ∧ lo < vl → no subtract: the fall-through runs the
        // l_hi compare redundantly, which is exact — a1 == VHI here, so
        // sltu(VHI, a1) = 0 and the following beq branches to l_next.
        // (The old loop needed a dedicated jal to skip the subtract arm;
        // this benign re-compare replaces it — same trick as A5.)
        self.place(l_hi);
        sltu(self, TMP, VHI, self.a1); // hi differ: subtract iff vh < rh
        self.asm.beq(TMP, ZERO, l_next); // vh ≥ rh → no subtract
        self.place(l_sub);
        // rem -= V (128-bit, borrow chain)
        sltu(self, TMP, T2, VLO);
        self.asm.sub(T2, T2, VLO);
        self.asm.sub(self.a1, self.a1, VHI);
        self.asm.sub(self.a1, self.a1, TMP);
        self.asm.addi(T3, T3, 1); // q |= 1
        self.place(l_next);
        self.asm.addi(14, 14, -1);
        self.asm.bne(14, ZERO, l_loop);
        // park both results in the frame workspace before restoring:
        // q = (T3, T4), r = (T2, a1). In the VHI == 0 arm the exit
        // remainder is < vlo so a1 is provably 0 (any transient carry
        // bit lifted into a1 was subtracted back off — a no-subtract
        // step requires the wrapped rem < V < 2^64, i.e. a1 == 0), and
        // T4 holds divu's q_hi.
        sd(self, T3, s + 32); // quotient lo
        sd(self, T4, s + 40); // quotient hi
        sd(self, T2, s + 48); // remainder lo
        sd(self, self.a1, s + 56); // remainder hi
        for (i, &r) in saved.iter().enumerate() {
            ld(self, r, s + 64 + (i as i32) * 8);
        }
        ld(self, self.a0, s + 128); // D1: return through the link register
        self.asm.jr(self.a0);
    }
}

#[cfg(test)]
mod a9_mul_routine_tests {
    //! A9 verification: the shared u128 multiply routine (the merged
    //! emit_u128_mul / emit_u128_mul_keep body) must agree with the
    //! interpreter's `<u128>::checked_mul` on every input — values AND the
    //! 90 overflow face — through the real pipeline (Lower → finish →
    //! consensus verify → calc run) in all three modes, with the gate
    //! forced ON and two sites per kernel (the amortized shape).

    use super::*;
    use crate::calc::container::{CalcContainer, CODE_BASE, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC};
    use crate::calc::interp::{run, CalcAbort};
    use crate::calc::verify::verify_container_with_cap;
    use crate::rt::SpaceCap;
    use std::collections::HashMap;

    const CYCLES: u64 = 20_000_000;

    /// Build/verify/run a kernel whose body multiplies consecutive 16-byte
    /// (D, B) records; gate forced ON. `Ok` = halt output; `Err(code)` =
    /// guest exit (90 = the checked_mul overflow face).
    fn run_mul_kernel(mode: Rv64Mode, input: &[u8], out_len: usize) -> Result<Vec<u8>, u64> {
        let mut long_ids = std::collections::HashSet::new();
        let mut wide_ids = std::collections::HashSet::new();
        let code = loop {
            let lists = ListPlanData::default();
            let nilp = NilPlan { tracked: vec![], occ: HashMap::new(), tag_bytes: false };
            let mut lw = Lower::new_mode_arena_bank(
                &[],
                &[],
                mode,
                ARENA_SIZE,
                8,
                WebPlan {
                    banked: vec![],
                    homes: vec![],
                    init_zero: vec![],
                    segs_debug: vec![],
                    no_split_debug: vec![],
                },
                None,
                &[],
                &nilp,
                &lists,
                &[],
                &[],
                true, // the mul routine shares the divmod frame workspace
                false,
                0,
                None,
                false,
                (false, false, false, true, false, false, false, false), // A9 gate forced ON
            );
            lw.asm.long_branch_ids = long_ids.clone();
            lw.asm.wide_c_ids = wide_ids.clone();
            lw.asm.addi(S0, SP, -(lw.frame_bytes as i32));
            lw.asm.mv(S2, A0);
            lw.asm.mv(S3, 12);
            // one mul site per 32-byte record: two u128 mul sites per case
            // (d·b, then the aliased square d·d — the POW aliasing shape)
            let n = input.len() / 32;
            for i in 0..n {
                let in_off = (i as i32) * 32;
                let dlo = lw.push_vreg();
                lw.asm.ld(dlo, S2, in_off);
                let dhi = lw.push_vreg();
                lw.asm.ld(dhi, S2, in_off + 8);
                let blo = lw.push_vreg();
                lw.asm.ld(blo, S2, in_off + 16);
                let bhi = lw.push_vreg();
                lw.asm.ld(bhi, S2, in_off + 24);
                lw.pop_vreg();
                lw.pop_vreg();
                lw.pop_vreg();
                lw.pop_vreg();
                lw.emit_u128_mul(dlo, dhi, blo, bhi);
                // the mul call lands the product IN the d pair (in-place
                // contract) — the caller pushes the result pair (the
                // pop/push handback usually recycles d's registers, so
                // the mvs elide)
                let out_lo = lw.push_vreg();
                let out_hi = lw.push_vreg();
                if out_lo != dlo {
                    lw.asm.mv(out_lo, dlo);
                }
                if out_hi != dhi {
                    lw.asm.mv(out_hi, dhi);
                }
                let hi = lw.pop_vreg();
                let lo = lw.pop_vreg();
                let out_off = (i as i32) * 16;
                lw.asm.sd(loc_reg(lo), S3, out_off);
                lw.asm.sd(loc_reg(hi), S3, out_off + 8);
            }
            lw.asm.addi(A0, ZERO, 0);
            lw.asm.li(A1, out_len as u64);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            if let Some(l) = lw.mul128_lbl {
                lw.asm.place(l);
                lw.emit_mul128_routine();
            }
            lw.asm.place(lw.fault_arith);
            lw.asm.li(A0, EXIT_ARITH);
            lw.asm.jal_(ZERO, lw.trap_label);
            lw.asm.place(lw.trap_label);
            lw.asm.addi(A1, ZERO, 0);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            match lw.asm.finish() {
                Ok(code) => break code,
                Err(FinishErr::BranchOverflow(sites)) => {
                    let before = long_ids.len();
                    long_ids.extend(sites);
                    if long_ids.len() == before {
                        panic!("a9 test kernel: branch relaxation did not converge");
                    }
                }
                Err(FinishErr::WideCOverflow(sites)) => {
                    let before = wide_ids.len();
                    wide_ids.extend(sites);
                    if wide_ids.len() == before {
                        panic!("a9 test kernel: c-slot relaxation did not converge");
                    }
                }
                Err(FinishErr::Msg(m)) => panic!("a9 test kernel did not assemble: {m}"),
            }
        };
        let profile = match mode {
            Rv64Mode::Im => CALC_PROFILE_RV64IM,
            _ => CALC_PROFILE_RV64IMC,
        };
        let c = CalcContainer::from_code(
            profile,
            16,
            CODE_BASE as u32,
            code,
            input.len().min(u16::MAX as usize) as u16,
            out_len.min(u16::MAX as usize) as u16,
        );
        let module = verify_container_with_cap(&c, &SpaceCap::new(0))
            .unwrap_or_else(|e| panic!("consensus verify rejected the a9 test kernel: {e}"));
        match run(&module, &c, input, CYCLES, None) {
            Ok(r) => Ok(r.output),
            Err((CalcAbort::GuestExit(code), _)) => Err(code),
            Err((a, _)) => panic!("a9 test kernel aborted: {a:?}"),
        }
    }

    fn limbs128(v: u128) -> Vec<u8> {
        let mut b = (v as u64).to_le_bytes().to_vec();
        b.extend_from_slice(&((v >> 64) as u64).to_le_bytes());
        b
    }

    fn out128(b: &[u8]) -> u128 {
        (u64::from_le_bytes(b[0..8].try_into().unwrap()) as u128)
            | ((u64::from_le_bytes(b[8..16].try_into().unwrap()) as u128) << 64)
    }

    /// Deterministic splitmix64 (the a6 corpus generator).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn u128(&mut self) -> u128 {
            ((self.next() as u128) << 64) | (self.next() as u128)
        }
    }

    const BATCH: usize = 64;

    #[test]
    fn a9_mul_routine_matches_interpreter() {
        // boundaries around the overflow seam + random pairs; the batched
        // kernel must not fault (any 90 exit kills the whole batch), so
        // the overflowing cases run one per kernel for the 90 face
        let mut cases: Vec<(u128, u128)> = vec![
            (0, 0),
            (0, u128::MAX),
            (1, u128::MAX),
            (u128::MAX, 1),
            ((1u128 << 64) - 1, (1u128 << 64) - 1), // 2^128 − 2^65 + 1 fits
            ((1u128 << 64) - 1, (1u128 << 64)),     // 2^128 − 2^64 fits
            ((1u128 << 127), 1), // fits
            (1u128 << 64, 0),    // 2^64 · 0 fits
            ((1u128 << 64) + 1, (1u128 << 63)), // 2^127 + 2^63 fits
        ];
        let mut rng = Rng(0xA9_0001);
        for _ in 0..4000 {
            cases.push((rng.u128(), rng.u128()));
        }
        let overflow_cases: Vec<(u128, u128)> = vec![
            (u128::MAX, 2),                         // → 90
            (1u128 << 64, 1u128 << 64),             // 2^128 → 90
            ((1u128 << 127), (1u128 << 1)),         // → 90
            ((1u128 << 127) + 1, 3),                // → 90
            (u128::MAX, u128::MAX),                 // → 90
        ];
        let safe: Vec<(u128, u128)> = cases
            .into_iter()
            .filter(|&(d, b)| d.checked_mul(b).is_some())
            .collect();
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for chunk in safe.chunks(BATCH) {
                let mut input = Vec::new();
                for &(d, b) in chunk {
                    input.extend(limbs128(d));
                    input.extend(limbs128(b));
                }
                let out_len = chunk.len() * 16;
                let out = run_mul_kernel(mode, &input, out_len)
                    .unwrap_or_else(|c| panic!("must not fault mode {mode:?} exit {c}"));
                for (i, &(d, b)) in chunk.iter().enumerate() {
                    assert_eq!(
                        out128(&out[i * 16..i * 16 + 16]),
                        d.checked_mul(b).unwrap(),
                        "product mismatch d={d} b={b} mode {mode:?}"
                    );
                }
            }
            // the 90 face: one overflowing pair per kernel
            for &(d, b) in &overflow_cases {
                let mut input = limbs128(d);
                input.extend(limbs128(b));
                let got = run_mul_kernel(mode, &input, 16);
                assert_eq!(
                    got,
                    Err(90),
                    "overflow must exit 90 d={d} b={b} mode {mode:?}"
                );
            }
        }
    }

    #[test]
    fn a9_gate_label_laziness() {
        // gate OFF: a site stays inline — no label, no routine
        let nilp = NilPlan { tracked: vec![], occ: HashMap::new(), tag_bytes: false };
        let lists = ListPlanData::default();
        let mut lw = Lower::new_mode_arena_bank(
            &[], &[], Rv64Mode::Im, ARENA_SIZE, 8,
            WebPlan { banked: vec![], homes: vec![], init_zero: vec![], segs_debug: vec![], no_split_debug: vec![] },
            None, &[], &nilp, &lists, &[], &[], true, false, 0, None, false,
            (false, false, false, false, false, false, false, false),
        );
        lw.asm.addi(S0, SP, -(lw.frame_bytes as i32));
        lw.emit_u128_mul(5, 6, 7, 8);
        assert!(lw.mul128_lbl.is_none(), "gate OFF must stay inline");
        // gate ON: the site stages through the routine label
        let mut lw = Lower::new_mode_arena_bank(
            &[], &[], Rv64Mode::Im, ARENA_SIZE, 8,
            WebPlan { banked: vec![], homes: vec![], init_zero: vec![], segs_debug: vec![], no_split_debug: vec![] },
            None, &[], &nilp, &lists, &[], &[], true, false, 0, None, false,
            (false, false, false, true, false, false, false, false),
        );
        lw.asm.addi(S0, SP, -(lw.frame_bytes as i32));
        lw.emit_u128_mul(5, 6, 7, 8);
        assert!(lw.mul128_lbl.is_some(), "gate ON must stage through the routine");
        // mul_keep rides the same routine
        lw.emit_u128_mul_keep(5, 6, 7, 8);
        assert!(lw.mul128_lbl.is_some());
    }
}

#[cfg(test)]
mod a6_divmod_tests {
    //! A6 verification: the shared u128 divmod routine's two-stage form
    //! (hardware divu/remu stage 1 + 64-iteration stage 2) must be
    //! bit-identical to the old 128-iteration shift loop on every input,
    //! with the 90 fault face per-input identical (exactly V == 0).
    //!
    //! Two layers:
    //! 1. `a6_divmod_models_agree` — faithful register-level Rust models
    //!    of BOTH routine versions (including the wrap-carry decision and
    //!    the wrapping 128-bit pair arithmetic), diffed against each other
    //!    and against Rust's native u128 division (the interpreter's own
    //!    `<u128>::checked_div/rem` ground truth) over ≥10^5 random plus
    //!    boundary inputs.
    //! 2. `a6_divmod_routine_matches_interpreter` — the REAL emitted
    //!    routine through the full pipeline (Lower → finish → consensus
    //!    verify → calc run) in all three modes, all three call shapes
    //!    (`emit_u128_divmod_qr` = DIVUP/DIVEXACT, `emit_u128_divmod`
    //!    q/r = DIV/MOD and the MULMOD/ADDMOD mod_pass), 10^4 random plus
    //!    boundary cases, and the V == 0 → exit 90 face.

    use super::*;
    use crate::calc::container::{CalcContainer, CODE_BASE, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC};
    use crate::calc::interp::{run, CalcAbort};
    use crate::calc::verify::verify_container_with_cap;
    use crate::rt::SpaceCap;
    use std::collections::HashMap;

    const CYCLES: u64 = 20_000_000;

    /* ---- register-level models ---- */

    /// The PRE-A6 routine: 128-iteration restoring shift loop with the
    /// wrap-carry decision, simulated with the machine's wrapping u64
    /// pair arithmetic (`None` = the fault-90 path).
    fn model_old_loop(d: u128, v: u128) -> Option<(u128, u128)> {
        if v == 0 {
            return None;
        }
        let mut dd = d;
        let mut rem: u128 = 0;
        let mut q: u128 = 0;
        for _ in 0..128 {
            let carry = rem >> 127; // msb before the doubling
            rem = rem.wrapping_shl(1) | (dd >> 127);
            dd <<= 1;
            q <<= 1; // q ≤ D/V < 2^128 — never overflows
            if carry == 1 || rem >= v {
                rem = rem.wrapping_sub(v);
                q |= 1;
            }
        }
        Some((q, rem))
    }

    /// The A6 routine: stage 1 (divu/remu or trivial, consuming dhi) +
    /// stage 2 (the 64-iteration shared loop over dlo with rem = a1:T2),
    /// simulated with the machine's wrapping u64 arithmetic.
    fn model_new_routine(d: u128, v: u128) -> Option<(u128, u128)> {
        if v == 0 {
            return None;
        }
        let dlo = d as u64;
        let dhi = (d >> 64) as u64;
        let vlo = v as u64;
        let vhi = (v >> 64) as u64;
        // stage 1
        let (q_hi, r1) = if vhi == 0 { (dhi / vlo, dhi % vlo) } else { (0u64, dhi) };
        // stage 2
        let mut a1: u64 = 0; // rem hi
        let mut t2: u64 = r1; // rem lo
        let mut t1: u64 = dlo; // shifting dividend low half
        let mut q_lo: u64 = 0;
        for _ in 0..64 {
            let carry = a1 >> 63;
            let t2_msb = t2 >> 63;
            a1 = (a1 << 1) | t2_msb;
            let t1_msb = t1 >> 63;
            t2 = t2.wrapping_shl(1) | t1_msb;
            t1 <<= 1;
            q_lo <<= 1; // q_lo = N'/V < 2^64 — never overflows
            if carry != 0 || (a1 == vhi && t2 >= vlo) || (a1 != vhi && vhi < a1) {
                let borrow = (t2 < vlo) as u64;
                t2 = t2.wrapping_sub(vlo);
                a1 = a1.wrapping_sub(vhi).wrapping_sub(borrow);
                q_lo |= 1;
            }
        }
        Some((((q_hi as u128) << 64) | q_lo as u128, ((a1 as u128) << 64) | t2 as u128))
    }

    /* ---- case corpus (deterministic) ---- */

    /// Deterministic splitmix64.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn u128(&mut self) -> u128 {
            ((self.next() as u128) << 64) | (self.next() as u128)
        }
    }

    fn pack(hi: u64, lo: u64) -> u128 {
        ((hi as u128) << 64) | lo as u128
    }

    /// Interesting u64 halves: the fast/slow-path decision boundaries.
    fn halves() -> Vec<u64> {
        vec![
            0,
            1,
            2,
            3,
            5,
            63,
            64,
            65,
            0xFF,
            0x1_0000,
            0xFFFF_FFFF,
            1u64 << 32,
            (1u64 << 32) - 1,
            (1u64 << 32) + 1,
            (1u64 << 63) - 1,
            1u64 << 63,
            (1u64 << 63) + 1,
            u64::MAX - 1,
            u64::MAX,
        ]
    }

    /// Boundary corpus: VHI == 0 cross products (dhi vs vlo equality,
    /// dhi = vlo−1/+1, dlo extremes), VHI != 0 crosses, 2^64-ish
    /// divisors, all-F, zero dividend/divisor corners.
    fn boundary_cases() -> Vec<(u128, u128)> {
        let hs = halves();
        let mut cases: Vec<(u128, u128)> = Vec::new();
        // fast path (VHI == 0): full cross of dhi × dlo × vlo
        for &vlo in &hs {
            if vlo == 0 {
                // V == 0 → the 90 face (kept in the corpus for the model
                // layer; the e2e layer splits them out)
                cases.push((0, 0));
                cases.push((u128::MAX, 0));
                continue;
            }
            for &dhi in &hs {
                for &dlo in &hs {
                    cases.push((pack(dhi, dlo), pack(0, vlo)));
                }
            }
            // the would-have-faulted region: dhi around and above vlo
            for &dlo in &hs {
                for k in 0..3u64 {
                    cases.push((pack(vlo.wrapping_add(k), dlo), pack(0, vlo)));
                    cases.push((pack(vlo.wrapping_sub(k + 1), dlo), pack(0, vlo)));
                    cases.push((pack(vlo.wrapping_mul(2).wrapping_add(k), dlo), pack(0, vlo)));
                }
            }
        }
        // slow path (VHI != 0): crosses with fixed extreme dividends
        let divs = [0u128, 1, 2, u64::MAX as u128, 1u128 << 64, (1u128 << 64) + 1, u128::MAX - 1, u128::MAX];
        for &vhi in &hs {
            for &vlo in &hs {
                if vhi == 0 && vlo == 0 {
                    continue;
                }
                for &d in &divs {
                    cases.push((d, pack(vhi, vlo)));
                }
                cases.push((pack(vhi, vlo), pack(vhi, vlo))); // D == V → q = 1
                cases.push((pack(vhi, vlo).wrapping_sub(1), pack(vhi, vlo)));
                cases.push((pack(vhi, vlo).wrapping_add(1), pack(vhi, vlo)));
            }
        }
        cases
    }

    /// Random corpus biased at the fast/slow seam: V < 2^64 (fast), V with
    /// the high half set (slow), and the dhi ≥ vlo quotient-≥-2^64 region.
    fn random_cases(n: usize, seed: u64) -> Vec<(u128, u128)> {
        let mut rng = Rng(seed);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let d = rng.u128();
            let v = match rng.next() % 10 {
                0..=5 => rng.next() as u128,                    // fast path
                6..=7 => ((rng.next() as u128) << 64).max(1),   // power-of-2-ish slow
                _ => rng.u128().max(1),                          // general slow
            };
            let d = if rng.next() % 4 == 0 && v < 1u128 << 64 {
                // hammer dhi ≥ vlo (quotient ≥ 2^64 — must NOT fault)
                pack(rng.next(), ((d as u64) ^ rng.next()) | (v as u64))
            } else {
                d
            };
            out.push((d, v.max(1)));
        }
        out
    }

    #[test]
    fn a6_divmod_models_agree() {
        let mut cases = boundary_cases();
        cases.extend(random_cases(100_000, 0xA6_0001));
        assert!(cases.len() >= 100_000);
        for &(d, v) in &cases {
            let native = if v == 0 {
                None
            } else {
                Some((d / v, d % v))
            };
            assert_eq!(
                model_old_loop(d, v),
                native,
                "old shift loop diverged from native division at d={d:#034x} v={v:#034x}"
            );
            assert_eq!(
                model_new_routine(d, v),
                native,
                "new two-stage routine diverged from native division at d={d:#034x} v={v:#034x}"
            );
        }
    }

    /* ---- end-to-end differential through the real pipeline ---- */

    fn limbs128(v: u128) -> Vec<u8> {
        let mut b = (v as u64).to_le_bytes().to_vec();
        b.extend_from_slice(&((v >> 64) as u64).to_le_bytes());
        b
    }

    fn out128(b: &[u8]) -> u128 {
        (u64::from_le_bytes(b[0..8].try_into().unwrap()) as u128)
            | ((u64::from_le_bytes(b[8..16].try_into().unwrap()) as u128) << 64)
    }

    /// Build/verify/run a synthetic kernel whose body emits one divmod
    /// call per case (all three call shapes) — the same pipeline the
    /// production lowering uses. `Ok` = halt output; `Err(code)` = guest
    /// exit (90 = fault_arith).
    fn run_kernel_out(
        mode: Rv64Mode,
        input: &[u8],
        out_len: usize,
        body: impl Fn(&mut Lower),
    ) -> Result<Vec<u8>, u64> {
        let mut long_ids = std::collections::HashSet::new();
        let mut wide_ids = std::collections::HashSet::new();
        let code = loop {
            let lists = ListPlanData::default();
            let nilp = NilPlan { tracked: vec![], occ: HashMap::new(), tag_bytes: false };
            let mut lw = Lower::new_mode_arena_bank(
                &[],
                &[],
                mode,
                ARENA_SIZE,
                8,
                WebPlan {
                    banked: vec![],
                    homes: vec![],
                    init_zero: vec![],
                    segs_debug: vec![],
                    no_split_debug: vec![],
                },
                None,
                &[],
                &nilp,
                &lists,
                &[],
                &[],
                true,  // needs_divmod — reserve the routine workspace
                false,
                0,
                None,
                false,
                (false, false, false, false, false, false, false, false),
            );
            lw.asm.long_branch_ids = long_ids.clone();
            lw.asm.wide_c_ids = wide_ids.clone();
            // frame base + IO shadows (S2 = input base, S3 = output base —
            // the same roles the real prologue gives them)
            lw.asm.addi(S0, SP, -(lw.frame_bytes as i32));
            lw.asm.mv(S2, A0);
            lw.asm.mv(S3, 12);
            body(&mut lw);
            // success halt: a0 = 0 (exit code), a1 = out_len, a7 = 0
            lw.asm.addi(A0, ZERO, 0);
            lw.asm.li(A1, out_len as u64);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            // the shared u128 divmod routine, then the fault thunks + trap
            // epilogue (the exact epilogue shape of the production
            // lowering — exit 90 rides A0 through the shared trap ecall)
            if let Some(l) = lw.divmod_lbl {
                lw.asm.place(l);
                lw.emit_divmod_routine();
            }
            lw.asm.place(lw.fault_arith);
            lw.asm.li(A0, EXIT_ARITH);
            lw.asm.jal_(ZERO, lw.trap_label);
            lw.asm.place(lw.trap_label);
            lw.asm.addi(A1, ZERO, 0);
            lw.asm.li(A7, 0);
            lw.asm.ecall();
            match lw.asm.finish() {
                Ok(code) => break code,
                Err(FinishErr::BranchOverflow(sites)) => {
                    let before = long_ids.len();
                    long_ids.extend(sites);
                    if long_ids.len() == before {
                        panic!("a6 test kernel: branch relaxation did not converge");
                    }
                }
                Err(FinishErr::WideCOverflow(sites)) => {
                    let before = wide_ids.len();
                    wide_ids.extend(sites);
                    if wide_ids.len() == before {
                        panic!("a6 test kernel: c-slot relaxation did not converge");
                    }
                }
                Err(FinishErr::Msg(m)) => panic!("a6 test kernel did not assemble: {m}"),
            }
        };
        let profile = match mode {
            Rv64Mode::Im => CALC_PROFILE_RV64IM,
            _ => CALC_PROFILE_RV64IMC,
        };
        let c = CalcContainer::from_code(
            profile,
            16,
            CODE_BASE as u32,
            code,
            input.len().min(u16::MAX as usize) as u16,
            out_len.min(u16::MAX as usize) as u16,
        );
        let module = verify_container_with_cap(&c, &SpaceCap::new(0))
            .unwrap_or_else(|e| panic!("consensus verify rejected the a6 test kernel: {e}"));
        match run(&module, &c, input, CYCLES, None) {
            Ok(r) => Ok(r.output),
            Err((CalcAbort::GuestExit(code), _)) => Err(code),
            Err((a, _)) => panic!("a6 test kernel aborted: {a:?}"),
        }
    }

    /// One `emit_u128_divmod_qr` site (the DIVUP/DIVEXACT shape): reads a
    /// 32-byte (D, V) record, writes a 32-byte (q, r) record.
    fn qr_site(lw: &mut Lower, in_off: i32, out_off: i32) {
        let dlo = lw.push_vreg();
        lw.asm.ld(dlo, S2, in_off);
        let dhi = lw.push_vreg();
        lw.asm.ld(dhi, S2, in_off + 8);
        let vlo = lw.push_vreg();
        lw.asm.ld(vlo, S2, in_off + 16);
        let vhi = lw.push_vreg();
        lw.asm.ld(vhi, S2, in_off + 24);
        lw.pop_vreg();
        lw.pop_vreg();
        lw.pop_vreg();
        lw.pop_vreg();
        lw.emit_u128_divmod_qr(dlo, dhi, vlo, vhi).unwrap();
        let rmhi = loc_reg(lw.pop_vreg());
        let rmlo = loc_reg(lw.pop_vreg());
        let qhi = loc_reg(lw.pop_vreg());
        let qlo = loc_reg(lw.pop_vreg());
        lw.asm.sd(qlo, S3, out_off);
        lw.asm.sd(qhi, S3, out_off + 8);
        lw.asm.sd(rmlo, S3, out_off + 16);
        lw.asm.sd(rmhi, S3, out_off + 24);
    }

    /// One `emit_u128_divmod` site (the DIV/MOD and MULMOD/ADDMOD
    /// mod_pass shape): 32-byte (D, V) in, 16-byte `want_mod ? r : q` out.
    fn divmod_site(lw: &mut Lower, want_mod: bool, in_off: i32, out_off: i32) {
        let dlo = lw.push_vreg();
        lw.asm.ld(dlo, S2, in_off);
        let dhi = lw.push_vreg();
        lw.asm.ld(dhi, S2, in_off + 8);
        let vlo = lw.push_vreg();
        lw.asm.ld(vlo, S2, in_off + 16);
        let vhi = lw.push_vreg();
        lw.asm.ld(vhi, S2, in_off + 24);
        lw.pop_vreg();
        lw.pop_vreg();
        lw.pop_vreg();
        lw.pop_vreg();
        lw.emit_u128_divmod(want_mod, dlo, dhi, vlo, vhi).unwrap();
        let hi = loc_reg(lw.pop_vreg());
        let lo = loc_reg(lw.pop_vreg());
        lw.asm.sd(lo, S3, out_off);
        lw.asm.sd(hi, S3, out_off + 8);
    }

    /// Sites per kernel: the raw `ld`/`sd` immediates over the record
    /// window must stay inside the i-type ±2047-byte range.
    const BATCH: usize = 64;

    #[test]
    fn a6_divmod_routine_matches_interpreter() {
        let mut cases: Vec<(u128, u128)> = boundary_cases();
        cases.retain(|&(_, v)| v != 0); // the 90 face has its own test
        cases.extend(random_cases(10_000, 0xA6_0002));
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for chunk in cases.chunks(BATCH) {
                let mut input = Vec::new();
                for &(d, v) in chunk {
                    input.extend(limbs128(d));
                    input.extend(limbs128(v));
                }
                let out_len = chunk.len() * 32;
                let out = run_kernel_out(mode, &input, out_len, |lw| {
                    for (i, _) in chunk.iter().enumerate() {
                        qr_site(lw, (i as i32) * 32, (i as i32) * 32);
                    }
                })
                .unwrap_or_else(|c| panic!("routine must not fault (exit {c}) mode {mode:?}"));
                for (i, &(d, v)) in chunk.iter().enumerate() {
                    let (q, r) = (d / v, d % v);
                    assert_eq!(
                        out128(&out[i * 32..i * 32 + 16]),
                        q,
                        "q mismatch d={d:#034x} v={v:#034x} mode {mode:?}"
                    );
                    assert_eq!(
                        out128(&out[i * 32 + 16..i * 32 + 32]),
                        r,
                        "r mismatch d={d:#034x} v={v:#034x} mode {mode:?}"
                    );
                }
            }
            // the DIV/MOD + mod_pass call shapes on a boundary + random
            // subset (the shape reuse of MULMOD/ADDMOD/DIV/MOD)
            let mut shape_cases: Vec<(u128, u128)> =
                boundary_cases().into_iter().filter(|&(_, v)| v != 0).collect();
            shape_cases.extend(random_cases(1_000, 0xA6_0003));
            for chunk in shape_cases.chunks(BATCH) {
                let mut input = Vec::new();
                for &(d, v) in chunk {
                    input.extend(limbs128(d));
                    input.extend(limbs128(v));
                }
                let out_len = chunk.len() * 32;
                let out = run_kernel_out(mode, &input, out_len, |lw| {
                    for (i, _) in chunk.iter().enumerate() {
                        // q site then r site — two routine calls per case
                        divmod_site(lw, false, (i as i32) * 32, (i as i32) * 32);
                        divmod_site(lw, true, (i as i32) * 32, (i as i32) * 32 + 16);
                    }
                })
                .unwrap_or_else(|c| panic!("routine must not fault (exit {c}) mode {mode:?}"));
                for (i, &(d, v)) in chunk.iter().enumerate() {
                    assert_eq!(out128(&out[i * 32..i * 32 + 16]), d / v, "DIV shape {mode:?}");
                    assert_eq!(out128(&out[i * 32 + 16..i * 32 + 32]), d % v, "MOD shape {mode:?}");
                }
            }
        }
    }

    #[test]
    fn a6_divmod_zero_divisor_is_90() {
        // V == 0 (both halves) → the routine's only fault, in every mode,
        // with the DIV/MOD/DIVUP shapes and extreme dividends
        for &mode in &[Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            for d in [0u128, 1, u64::MAX as u128, 1u128 << 64, u128::MAX] {
                let mut input = limbs128(d);
                input.extend(limbs128(0));
                for (name, body) in [
                    ("qr", &(|lw: &mut Lower| qr_site(lw, 0, 0)) as &dyn Fn(&mut Lower)),
                    ("div", &|lw: &mut Lower| divmod_site(lw, false, 0, 0)),
                    ("mod", &|lw: &mut Lower| divmod_site(lw, true, 0, 0)),
                ] {
                    let got = run_kernel_out(mode, &input, 32, |lw| body(lw));
                    assert_eq!(
                        got,
                        Err(90),
                        "zero divisor must exit 90 (shape {name}, d={d}, mode {mode:?})"
                    );
                }
            }
        }
    }
}
