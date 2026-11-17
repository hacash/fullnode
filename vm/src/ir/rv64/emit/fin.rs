use super::super::*;

impl<'a> Lower<'a> {

    /// FIN2 lowering (div_* / sqrt_mul_* / quantize_* / sat_add /
    /// sat_sub — instruction.rs fin2_checked).
    pub(crate) fn emit_fin2(&mut self, n: &IRNodeParam1Double) -> Ret<()> {
        let spec = fin_spec_lookup(Bytecode::FIN2, n.para).map_err(|e| format!("rv64 lower: {e}"))?;
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let w = wx.max(wy);
        let (lx, ly) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby));
        self.fin_stage_args(&[(&*n.subx, wx), (&*n.suby, wy)], lx || ly)?;
        self.fin_nil_guards(&[lx, ly])?;
        let base = self.vstack.len() - 4;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        match spec.kernel {
            FinKernel::Div => {
                // div_with_round_checked: plain u128÷u128 division with the
                // policy tail — y == 0 faults inside the routine (90). The
                // divisor parks across the divmod (its result pushes land
                // on the operand registers), then reloads into the dead
                // zero-limb registers for the policy tail.
                self.fin_park(U256WS_FINA, y);
                let t0 = self.push_vreg();
                self.asm.addi(t0, ZERO, 0);
                let t1 = self.push_vreg();
                self.asm.addi(t1, ZERO, 0);
                let _ = self.pop_reg(); // t1
                let _ = self.pop_reg(); // t0
                let _ = self.fin_pop_pair(); // y (registers keep the value)
                let _ = self.fin_pop_pair(); // x
                let n256 = [x.0, x.1, t0, t1];
                self.emit_u256_divmod_u128(true, &n256, y)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.fin_load(U256WS_FINA, (t0, t1)); // the divisor
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, (t0, t1))?;
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair();
                self.emit_fin_publish(q0, q1, w);
            }
            FinKernel::Quantize => {
                // quantize_with_round_checked: step = y (0 -> 90);
                // Floor: quo·step (checked mul); Ceil: exact -> value,
                // else (quo+1)·step with both adds checked. Value and step
                // park across the divmod (same register plan as Div).
                self.fin_park(U256WS_FINA, x); // the Ceil exact path's value
                self.fin_park(U256WS_FINB, y); // the step
                let t0 = self.push_vreg();
                self.asm.addi(t0, ZERO, 0);
                let t1 = self.push_vreg();
                self.asm.addi(t1, ZERO, 0);
                let _ = self.pop_reg(); // t1
                let _ = self.pop_reg(); // t0
                let _ = self.fin_pop_pair(); // y (registers keep the value)
                let _ = self.fin_pop_pair(); // x
                let n256 = [x.0, x.1, t0, t1];
                self.emit_u256_divmod_u128(true, &n256, y)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.fin_load(U256WS_FINB, (t0, t1)); // the step
                let (q0, q1) = (q.0, q.1);
                if spec.round_or_exact() == FinRoundPolicy::Ceil {
                    let l_exact = self.asm.new_label();
                    let l_end = self.asm.new_label();
                    self.asm.or_(self.a0, r.0, r.1);
                    self.asm.beq(self.a0, ZERO, l_exact);
                    self.asm.addi(self.a0, ZERO, 1);
                    self.emit_u128_add(q0, q1, self.a0, ZERO); // quo+1 checked
                    self.emit_u128_mul(q0, q1, t0, t1); // ·step, checked
                    self.asm.jal_(ZERO, l_end);
                    self.place(l_exact);
                    self.fin_load(U256WS_FINA, (q0, q1)); // out = value
                    self.place(l_end);
                } else {
                    self.emit_u128_mul(q0, q1, t0, t1); // checked (cannot trip)
                }
                let _ = self.fin_pop_pair(); // r
                let (q0r, q1r) = self.fin_pop_pair(); // q -> plain regs
                self.emit_fin_publish(q0r, q1r, w);
            }
            FinKernel::SqrtMul => {
                // sqrtmul_with_round_checked: floor = isqrt_u256(x·y);
                // Ceil adds 1 when floor² != x·y (the checked +1 -> 90).
                let a0 = self.a0;
                let s = self.u256_save_off;
                let limbs = self.emit_u256_mul_wide(x, y)?;
                // the 4-limb product is the isqrt input (FINA..FINA+24)
                for (i, &r) in limbs.iter().enumerate() {
                    self.asm.sd(r, S0, s + U256WS_FINA + (i as i32) * 8);
                }
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                let (f0, f1) = self.emit_fin_isqrt256(U256WS_FINA)?;
                if spec.round_or_exact() == FinRoundPolicy::Ceil {
                    // park the floor, square it, compare with the product
                    self.fin_park(U256WS_FINF, (f0, f1));
                    let c0 = self.push_vreg();
                    let c1 = self.push_vreg();
                    self.asm.mv(c0, f0);
                    self.asm.mv(c1, f1);
                    let sq = self.emit_u256_mul_wide((f0, f1), (c0, c1))?;
                    // the product limbs into four fresh slots above the
                    // square (peak 8 — the probe sizes the pool the same)
                    let n0 = self.push_vreg();
                    let n1 = self.push_vreg();
                    let n2 = self.push_vreg();
                    let n3 = self.push_vreg();
                    self.asm.ld(n0, S0, s + U256WS_FINA);
                    self.asm.ld(n1, S0, s + U256WS_FINA + 8);
                    self.asm.ld(n2, S0, s + U256WS_FINA + 16);
                    self.asm.ld(n3, S0, s + U256WS_FINA + 24);
                    let l_up = self.asm.new_label();
                    let l_eq = self.asm.new_label();
                    let l_done = self.asm.new_label();
                    // sq < n or sq > n -> +1; sq == n -> keep the floor
                    self.emit_u256_branch3(&sq, &[n0, n1, n2, n3], l_up, l_eq, l_up);
                    for _ in 0..4 {
                        let _ = self.pop_vreg(); // n limbs
                    }
                    let _ = self.fin_pop_pair(); // c pair
                    let _ = self.fin_pop_pair(); // floor pair (regs keep)
                    self.place(l_eq);
                    self.fin_load(U256WS_FINF, (f0, f1));
                    self.asm.jal_(ZERO, l_done);
                    self.place(l_up);
                    self.fin_load(U256WS_FINF, (f0, f1));
                    self.asm.addi(a0, ZERO, 1);
                    self.emit_u128_add(f0, f1, a0, ZERO); // checked -> 90
                    self.place(l_done);
                } else {
                    // Floor: release the isqrt result entries (the registers
                    // keep the value) before publishing.
                    let _ = self.fin_pop_pair();
                }
                self.emit_fin_publish(f0, f1, w);
            }
            FinKernel::SatAdd => {
                // saturating_uint_add on the promoted pair: saturate at the
                // common width w.
                let d0 = self.push_vreg();
                let d1 = self.push_vreg();
                let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                self.asm.add(d0, x.0, y.0);
                self.asm.r(0, y.0, d0, 3, sc1, 0b0110011); // carry
                self.asm.add(d1, x.1, y.1);
                self.asm.r(0, y.1, d1, 3, sc2, 0b0110011);
                self.asm.add(d1, d1, sc1);
                self.asm.r(0, sc1, d1, 3, sc1, 0b0110011);
                self.asm.or_(sc2, sc2, sc1); // carry out of bit 127
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                let l_end = self.asm.new_label();
                if w == 128 {
                    let l_no = self.asm.new_label();
                    self.asm.beq(sc2, ZERO, l_no);
                    self.asm.addi(d0, ZERO, -1);
                    self.asm.addi(d1, ZERO, -1);
                    self.place(l_no);
                } else {
                    let l_sat = self.asm.new_label();
                    self.asm.bne(d1, ZERO, l_sat);
                    if w < 64 {
                        self.asm.li(self.a0, 1u64 << w);
                        self.asm.bgeu(d0, self.a0, l_sat);
                    }
                    self.asm.jal_(ZERO, l_end);
                    self.place(l_sat);
                    let sat = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
                    self.asm.li(d0, sat);
                    self.asm.addi(d1, ZERO, 0);
                }
                self.place(l_end);
                let _ = self.fin_pop_pair(); // d pair → plain regs
                self.emit_fin_publish(d0, d1, w);
            }
            FinKernel::SatSub => {
                // saturating_uint_sub: x ≥ y ? x-y : 0 (no fault face).
                let d0 = self.push_vreg();
                let d1 = self.push_vreg();
                self.asm.mv(d0, x.0);
                self.asm.mv(d1, x.1);
                self.emit_u128_cmp(Bytecode::GE, self.a0, x, y);
                let l_zero = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.asm.beq(self.a0, ZERO, l_zero);
                self.emit_u128_sub(d0, d1, y.0, y.1);
                self.asm.jal_(ZERO, l_end);
                self.place(l_zero);
                self.asm.addi(d0, ZERO, 0);
                self.asm.addi(d1, ZERO, 0);
                self.place(l_end);
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                let _ = self.fin_pop_pair(); // d pair → plain regs
                self.emit_fin_publish(d0, d1, w);
            }
            _ => {
                return errf!(
                    "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                    spec.name
                );
            }
        }
        Ok(())
    }

    /// FIN4 lowering (mul_add|sub_div_* / mul3_div_* / wavg2_* / lerp_* —
    /// instruction.rs fin4_checked).
    pub(crate) fn emit_fin4(&mut self, n: &IRNodeParam1Quad) -> Ret<()> {
        let spec =
            fin_spec_lookup(Bytecode::FIN4, n.para).map_err(|e| format!("rv64 lower: {e}"))?;
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let wz = self.fin_arg_width(self.int_width(&*n.subz), spec.name)?;
        let ww = self.fin_arg_width(self.int_width(&*n.subw), spec.name)?;
        let w = wx.max(wy).max(wz).max(ww);
        let (lx, ly, lz, lw) = (
            self.nil_val(&*n.subx),
            self.nil_val(&*n.suby),
            self.nil_val(&*n.subz),
            self.nil_val(&*n.subw),
        );
        self.fin_stage_args(
            &[
                (&*n.subx, wx),
                (&*n.suby, wy),
                (&*n.subz, wz),
                (&*n.subw, ww),
            ],
            lx || ly || lz || lw,
        )?;
        self.fin_nil_guards(&[lx, ly, lz, lw])?;
        let base = self.vstack.len() - 8;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        let z = self.fin_pair_at(base + 4);
        let wd = self.fin_pair_at(base + 6);
        match spec.kernel {
            FinKernel::MulAddDiv | FinKernel::MulSubDiv => {
                // mul_xy_addsub_z_div_u128: n = x·y ± z at 256 bits (the
                // add/sub carry faces -> 90), then the rounded division.
                let limbs = self.emit_u256_mul_wide(x, y)?;
                if spec.kernel == FinKernel::MulAddDiv {
                    self.emit_u256_add_u128(&limbs, z);
                } else {
                    self.emit_u256_sub_u128(&limbs, z);
                }
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.emit_u256_divmod_u128(true, &limbs, wd)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, wd)?;
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair();
                self.emit_fin_publish(q0, q1, w);
            }
            FinKernel::Mul3Div => {
                // mul3div_checked: n = (x·y)·z through
                // mul_u256_u128_to_u256_checked (hi_hi != 0 or the checked
                // hi add -> 90), then the rounded division.
                self.fin_park(U256WS_FINA, wd); // the divisor
                self.fin_park(U256WS_FINB, z); // the third factor
                let limbs = self.emit_u256_mul_wide(x, y)?; // lo=(x), hi=(y)
                // loz = lo · z (z reloads into the dead w regs)
                self.fin_load(U256WS_FINB, wd);
                let loz = self.emit_u256_mul_wide((limbs[0], limbs[1]), wd)?;
                self.fin_park(U256WS_FINC, (loz[2], loz[3])); // loz hi pair
                // hiz = hi · z (z reloads again)
                self.fin_load(U256WS_FINB, wd);
                let hiz = self.emit_u256_mul_wide((limbs[2], limbs[3]), wd)?;
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.asm.or_(self.a0, hiz[2], hiz[3]);
                self.asm.bne(self.a0, ZERO, self.fault_arith); // hi_hi != 0
                // n_hi = hiz_lo + loz_hi checked (carry -> 90)
                self.fin_load(U256WS_FINC, wd); // loz hi -> w regs
                self.emit_u128_add(hiz[0], hiz[1], wd.0, wd.1);
                // n = [loz_lo, hiz_lo/hi] = [loz[0], loz[1], hiz[0], hiz[1]]
                self.fin_load(U256WS_FINA, wd); // the divisor -> w regs
                let n256 = [loz[0], loz[1], hiz[0], hiz[1]];
                self.emit_u256_divmod_u128(true, &n256, wd)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, wd)?;
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair();
                self.emit_fin_publish(q0, q1, w);
            }
            FinKernel::Wavg2 => {
                // wavg2_checked: den = wx + wy (a u129; zero -> 90), lhs ==
                // rhs -> lhs, else base + round(|lhs-rhs|·dw / den) with
                // the half-even tie broken against the FINAL parity
                // (base's bit 0).
                let s = self.u256_save_off;
                self.fin_park(U256WS_FINA, z); // rhs
                self.fin_park(U256WS_FINB, y); // wx
                self.fin_park(U256WS_FINC, wd); // wy
                // den limbs = [wx regs, zeroed z regs] += wy (w regs)
                self.asm.addi(z.0, ZERO, 0);
                self.asm.addi(z.1, ZERO, 0);
                let den = [y.0, y.1, z.0, z.1];
                self.emit_u256_add_u128(&den, wd);
                self.asm.sd(y.0, S0, s + U256WS_FIND); // den lo
                self.asm.sd(y.1, S0, s + U256WS_FIND + 8);
                self.asm.sd(z.0, S0, s + U256WS_FINE); // den bit 128
                self.asm.or_(self.a0, y.0, y.1);
                self.asm.or_(self.a0, self.a0, z.0);
                self.asm.beq(self.a0, ZERO, self.fault_arith); // den == 0
                let l_eq = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.fin_load(U256WS_FINA, wd); // rhs -> w regs
                self.emit_u128_cmp(Bytecode::EQ, self.a0, x, wd);
                self.asm.bne(self.a0, ZERO, l_eq);
                // park the default base candidate (lhs) before consuming it;
                // the lhs > rhs arm overwrites the park with rhs below (the
                // interpreter's base is the SMALLER operand)
                self.fin_park(U256WS_FINF, x);
                self.emit_u128_cmp(Bytecode::LT, self.a0, x, wd);
                let l_lt = self.asm.new_label();
                let l_wmul = self.asm.new_label();
                self.asm.bne(self.a0, ZERO, l_lt);
                // lhs > rhs: diff = lhs - rhs (x regs), dw = wx, base = rhs
                self.emit_u128_sub(x.0, x.1, wd.0, wd.1);
                self.fin_park(U256WS_FINF, wd); // base = rhs
                self.fin_load(U256WS_FINB, y); // wx -> y regs
                self.asm.jal_(ZERO, l_wmul);
                self.place(l_lt);
                // lhs < rhs: diff = rhs - lhs (w regs -> x regs), dw = wy
                self.emit_u128_sub(wd.0, wd.1, x.0, x.1);
                self.fin_load(U256WS_FINC, y); // wy -> y regs
                self.asm.mv(x.0, wd.0);
                self.asm.mv(x.1, wd.1);
                self.place(l_wmul);
                let limbs = self.emit_u256_mul_wide(x, y)?; // diff · dw
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                // den lo -> the dead z regs; den bit 128 -> the dead w lo
                self.fin_load(U256WS_FIND, z);
                self.asm.ld(wd.0, S0, s + U256WS_FINE);
                self.emit_u256_divmod_u129(true, &limbs, z, wd.0)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                let rh = loc_reg(*self.vstack.get(base + 4).expect("fin rh entry"));
                // three plain scratch registers above the live set
                self.emit_wavg2_round(spec.round_or_exact(), q, r, rh, [z.1, wd.0, wd.1])?;
                let _ = self.pop_reg(); // rh
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair(); // q -> plain regs
                // out = base + part (checked -> 90), then the fit-back
                self.fin_load(U256WS_FINF, z);
                self.emit_u128_add(q0, q1, z.0, z.1);
                self.emit_fin_publish(q0, q1, w);
                self.asm.jal_(ZERO, l_end);
                self.place(l_eq);
                {
                    // lhs == rhs: the result is lhs (already in its regs).
                    // The fall-through arm's operand pops/publish are
                    // lowering-time bookkeeping only, so this runtime arm
                    // starts from the entries the previous arm published;
                    // discard them, then publish this arm's value into the
                    // same pool position.
                    for _ in 0..Self::entries_of_width(Some(w)) {
                        let _ = self.pop_vreg();
                    }
                    self.emit_fin_publish(x.0, x.1, w);
                }
                self.place(l_end);
            }
            FinKernel::Lerp => {
                // lerp_checked: base == 0 or weight > base -> 90; n =
                // start·(base-weight) + end·weight at 256 bits; q =
                // round(n / base).
                let a0 = self.a0;
                self.asm.or_(a0, wd.0, wd.1);
                self.asm.beq(a0, ZERO, self.fault_arith); // base == 0
                self.emit_u128_cmp(Bytecode::GT, a0, z, wd);
                self.asm.bne(a0, ZERO, self.fault_arith); // weight > base
                self.fin_park(U256WS_FINA, wd); // the base (divisor)
                self.emit_u128_sub(wd.0, wd.1, z.0, z.1); // left = base - weight
                let llimbs = self.emit_u256_mul_wide(x, wd)?; // start · left
                let rlimbs = self.emit_u256_mul_wide(y, z)?; // end · weight
                let n256 = [llimbs[0], llimbs[1], llimbs[2], llimbs[3]];
                let m256 = [rlimbs[0], rlimbs[1], rlimbs[2], rlimbs[3]];
                self.emit_u256_add_full(&n256, &m256); // carry -> 90
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.fin_load(U256WS_FINA, z); // the base -> z regs
                self.emit_u256_divmod_u128(true, &n256, z)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, z)?;
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair();
                self.emit_fin_publish(q0, q1, w);
            }
            _ => {
                return errf!(
                    "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                    spec.name
                );
            }
        }
        Ok(())
    }

    /// FINPOW3 lowering (rpow_half_up — instruction.rs finpow3_checked →
    /// rpow_checked): out = x^y rounded to z decimals with a half-up
    /// rounding after EVERY multiplication; z == 0 -> 90, y == 0 -> z.
    /// Square-and-multiply over y's bits (the interpreter's loop shape —
    /// ~log2(y) iterations, each one mul_wide + divmod + HalfUp).
    pub(crate) fn emit_finpow3(&mut self, n: &IRNodeParam1Triple) -> Ret<()> {
        let spec = fin_spec_lookup(Bytecode::FINPOW3, n.para)
            .map_err(|e| format!("rv64 lower: {e}"))?;
        if spec.kernel != FinKernel::RPow {
            return errf!(
                "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                spec.name
            );
        }
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let wz = self.fin_arg_width(self.int_width(&*n.subz), spec.name)?;
        let w = wx.max(wy).max(wz);
        let (lx, ly, lz) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby), self.nil_val(&*n.subz));
        self.fin_stage_args(
            &[(&*n.subx, wx), (&*n.suby, wy), (&*n.subz, wz)],
            lx || ly || lz,
        )?;
        self.fin_nil_guards(&[lx, ly, lz])?;
        let base = self.vstack.len() - 6;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        let z = self.fin_pair_at(base + 4);
        // base == 0 -> 90 (BEFORE the exponent's zero case)
        self.asm.or_(self.a0, z.0, z.1);
        self.asm.beq(self.a0, ZERO, self.fault_arith);
        // n == 0 -> result = base
        let l_n0 = self.asm.new_label();
        let l_out_done = self.asm.new_label();
        let l_rpow_done = self.asm.new_label();
        self.asm.or_(self.a0, y.0, y.1);
        self.asm.beq(self.a0, ZERO, l_n0);
        self.fin_park(U256WS_FINA, z); // the base/scale
        self.fin_park(U256WS_FINB, x); // bas
        // out = (n & 1) ? bas : base — parked at FINC
        let l_oe = self.asm.new_label();
        self.asm.andi(self.asm.sc1, y.0, 1);
        self.asm.beq(self.asm.sc1, ZERO, l_oe);
        self.fin_park(U256WS_FINC, x);
        self.asm.jal_(ZERO, l_out_done);
        self.place(l_oe);
        self.fin_park(U256WS_FINC, z);
        self.place(l_out_done);
        // release the operand entries; n rides the vstack through the loop
        let _ = self.fin_pop_pair(); // z (registers keep the values)
        let _ = self.fin_pop_pair(); // y
        let _ = self.fin_pop_pair(); // x
        let n0 = self.push_vreg();
        self.asm.mv(n0, y.0);
        let n1 = self.push_vreg();
        self.asm.mv(n1, y.1);
        let nb = self.vstack.len() - 2; // the n pair's entry base
        let nregs = self.fin_pair_at(nb);
        // The base divisor is a live pair for the whole loop. It rides the
        // vstack ABOVE the n pair as a real (pushed) entry pair — the probe
        // and the lowering then agree on the pool demand (8 slots), and the
        // shared divmod result pushes land above it.
        let bv0 = self.push_vreg();
        let bv1 = self.push_vreg();
        self.fin_load(U256WS_FINA, (bv0, bv1));
        let bdiv = (bv0, bv1);
        let l_head = self.asm.new_label();
        let l_body = self.asm.new_label();
        let l_end = self.asm.new_label();
        self.place(l_head);
        // while n > 1: n_hi != 0 or n_lo > 1
        self.asm.bne(nregs.1, ZERO, l_body);
        self.asm.addi(self.asm.sc1, ZERO, 1);
        self.asm.bltu(self.asm.sc1, nregs.0, l_body);
        self.asm.jal_(ZERO, l_end);
        self.place(l_body);
        {
            let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
            // n >>= 1 (in place)
            self.asm.srli_(sc2, nregs.0, 63);
            self.asm.srli_(nregs.1, nregs.1, 1);
            self.asm.or_(nregs.1, nregs.1, sc2);
            self.asm.srli_(nregs.0, nregs.0, 1);
            // bas = round_half_up(bas·bas / base)
            let b0 = self.push_vreg();
            let b1 = self.push_vreg();
            self.fin_load(U256WS_FINB, (b0, b1));
            let c0 = self.push_vreg();
            let c1 = self.push_vreg();
            self.asm.mv(c0, b0);
            self.asm.mv(c1, b1);
            self.emit_u256_mul_wide((b0, b1), (c0, c1))?; // limbs [b0,b1,c0,c1]
            let _ = self.fin_pop_pair(); // c pair
            let _ = self.fin_pop_pair(); // b pair -> q/r land there
            let limbs = [b0, b1, c0, c1];
            self.emit_u256_divmod_u128(true, &limbs, bdiv)?;
            let qb = self.vstack.len() - 4;
            let q = self.fin_pair_at(qb);
            let r = self.fin_pair_at(qb + 2);
            self.emit_u128_round_quot(FinRoundPolicy::HalfUp, q, r, bdiv)?;
            let _ = self.fin_pop_pair(); // r
            // park the new bas from the q registers
            self.fin_park(U256WS_FINB, q);
            let _ = self.fin_pop_pair(); // q
            // if n & 1: out = round_half_up(out·bas / base)
            self.asm.andi(sc1, nregs.0, 1);
            self.asm.beq(sc1, ZERO, l_head);
            let o0 = self.push_vreg();
            let o1 = self.push_vreg();
            self.fin_load(U256WS_FINC, (o0, o1));
            let p0 = self.push_vreg();
            let p1 = self.push_vreg();
            self.fin_load(U256WS_FINB, (p0, p1));
            self.emit_u256_mul_wide((o0, o1), (p0, p1))?; // limbs [o0,o1,p0,p1]
            let _ = self.fin_pop_pair(); // p pair
            let _ = self.fin_pop_pair(); // o pair
            let limbs2 = [o0, o1, p0, p1];
            self.emit_u256_divmod_u128(true, &limbs2, bdiv)?;
            let qb2 = self.vstack.len() - 4;
            let q2 = self.fin_pair_at(qb2);
            let r2 = self.fin_pair_at(qb2 + 2);
            self.emit_u128_round_quot(FinRoundPolicy::HalfUp, q2, r2, bdiv)?;
            let _ = self.fin_pop_pair(); // r
            self.fin_park(U256WS_FINC, q2);
            let _ = self.fin_pop_pair(); // q
            self.asm.jal_(ZERO, l_head);
        }
        self.place(l_end);
        let _ = self.fin_pop_pair(); // the base divisor pair
        let _ = self.fin_pop_pair(); // the n pair
        self.fin_load(U256WS_FINC, x);
        self.emit_fin_publish(x.0, x.1, w);
        self.asm.jal_(ZERO, l_rpow_done);
        // n == 0: the result is the base itself
        self.place(l_n0);
        {
            // The straight-line path's operand pops/publish are lowering-time
            // bookkeeping; this runtime arm enters with the entries the loop
            // arm published. Discard them, then publish the base value into
            // the same pool positions (z's registers still hold it: the
            // branch is taken before the loop clobbers them).
            for _ in 0..Self::entries_of_width(Some(w)) {
                let _ = self.pop_vreg();
            }
            self.emit_fin_publish(z.0, z.1, w);
        }
        self.place(l_rpow_done);
        Ok(())
    }

    /// FINP3 lowering (abs_diff_lte — instruction.rs finp3_checked):
    /// Bool(|x - y| <= tol); no fault faces beyond the operand guards.
    pub(crate) fn emit_finp3(&mut self, n: &IRNodeParam1Triple) -> Ret<()> {
        let spec =
            fin_spec_lookup(Bytecode::FINP3, n.para).map_err(|e| format!("rv64 lower: {e}"))?;
        if spec.kernel != FinKernel::AbsDiffLte {
            return errf!(
                "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                spec.name
            );
        }
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let wz = self.fin_arg_width(self.int_width(&*n.subz), spec.name)?;
        let _ = (wx, wy, wz); // per-operand staging widths only (Bool result)
        let (lx, ly, lz) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby), self.nil_val(&*n.subz));
        self.eval_fin_arg(&*n.subx, Some(wx))?;
        self.eval_fin_arg(&*n.suby, Some(wy))?;
        self.eval_fin_arg(&*n.subz, Some(wz))?;
        self.fin_nil_guards(&[lx, ly, lz])?;
        let base = self.vstack.len() - 6;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        let z = self.fin_pair_at(base + 4);
        // the diff pair above the operands
        let t0 = self.push_vreg();
        let t1 = self.push_vreg();
        self.emit_u128_cmp(Bytecode::LT, self.a0, x, y);
        let l_lt = self.asm.new_label();
        let l_join = self.asm.new_label();
        self.asm.bne(self.a0, ZERO, l_lt);
        // x >= y: diff = x - y
        self.asm.mv(t0, x.0);
        self.asm.mv(t1, x.1);
        self.emit_u128_sub(t0, t1, y.0, y.1);
        self.asm.jal_(ZERO, l_join);
        self.place(l_lt);
        // x < y: diff = y - x
        self.asm.mv(t0, y.0);
        self.asm.mv(t1, y.1);
        self.emit_u128_sub(t0, t1, x.0, x.1);
        self.place(l_join);
        // Release the staged entries top-down: the diff pair (its registers
        // keep the value), then z/y/x (registers likewise; the final compare
        // still reads z, which no later allocation clobbers at vregs[4..5]).
        let _ = self.fin_pop_pair(); // the diff pair
        let _ = self.fin_pop_pair(); // z
        let _ = self.fin_pop_pair(); // y
        let _ = self.fin_pop_pair(); // x
        let d = self.push_vreg();
        self.emit_u128_cmp(Bytecode::LE, d, (t0, t1), z);
        Ok(())
    }

    /// FINP4 lowering (within_bps / cross_lte / cross_gte / cross_eq —
    /// instruction.rs finp4_checked): Bool predicates over 256-bit cross
    /// products.
    pub(crate) fn emit_finp4(&mut self, n: &IRNodeParam1Quad) -> Ret<()> {
        let spec =
            fin_spec_lookup(Bytecode::FINP4, n.para).map_err(|e| format!("rv64 lower: {e}"))?;
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let wz = self.fin_arg_width(self.int_width(&*n.subz), spec.name)?;
        let ww = self.fin_arg_width(self.int_width(&*n.subw), spec.name)?;
        let _ = (wx, wy, wz, ww);
        let (lx, ly, lz, lw) = (
            self.nil_val(&*n.subx),
            self.nil_val(&*n.suby),
            self.nil_val(&*n.subz),
            self.nil_val(&*n.subw),
        );
        self.eval_fin_arg(&*n.subx, Some(wx))?;
        self.eval_fin_arg(&*n.suby, Some(wy))?;
        self.eval_fin_arg(&*n.subz, Some(wz))?;
        self.eval_fin_arg(&*n.subw, Some(ww))?;
        self.fin_nil_guards(&[lx, ly, lz, lw])?;
        let base = self.vstack.len() - 8;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        let z = self.fin_pair_at(base + 4);
        let wd = self.fin_pair_at(base + 6);
        match spec.kernel {
            FinKernel::WithinBps => {
                // withinbps_checked: reference == 0 or scale == 0 -> 90,
                // tolerance > scale -> 90, then diff·scale <= ref·tol.
                let a0 = self.a0;
                self.asm.or_(a0, y.0, y.1);
                self.asm.beq(a0, ZERO, self.fault_arith); // reference == 0
                self.asm.or_(a0, wd.0, wd.1);
                self.asm.beq(a0, ZERO, self.fault_arith); // scale == 0
                self.emit_u128_cmp(Bytecode::GT, a0, z, wd);
                self.asm.bne(a0, ZERO, self.fault_arith); // tolerance > scale
                // diff = |x - y| into the x regs (reference parks first)
                self.fin_park(U256WS_FINA, y);
                self.emit_u128_cmp(Bytecode::LT, a0, x, y);
                let l_lt = self.asm.new_label();
                let l_join = self.asm.new_label();
                self.asm.bne(a0, ZERO, l_lt);
                self.emit_u128_sub(x.0, x.1, y.0, y.1);
                self.asm.jal_(ZERO, l_join);
                self.place(l_lt);
                self.emit_u128_sub(y.0, y.1, x.0, x.1);
                self.asm.mv(x.0, y.0);
                self.asm.mv(x.1, y.1);
                self.place(l_join);
                let lhs = self.emit_u256_mul_wide(x, wd)?; // diff · scale
                self.fin_load(U256WS_FINA, y); // the reference back
                let rhs = self.emit_u256_mul_wide(y, z)?; // reference · tolerance
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                // Bool(lhs <= rhs) — a0 materializes the branch outcome
                let l_true = self.asm.new_label();
                let l_false = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.emit_u256_branch3(&lhs, &rhs, l_true, l_true, l_false);
                self.place(l_true);
                self.asm.addi(a0, ZERO, 1);
                self.asm.jal_(ZERO, l_end);
                self.place(l_false);
                self.asm.addi(a0, ZERO, 0);
                self.place(l_end);
                let d = self.push_vreg();
                self.asm.mv(d, a0);
            }
            FinKernel::CrossLte | FinKernel::CrossGte | FinKernel::CrossEq => {
                // crossmul_pred_checked: Bool(x·y <cmp> z·w) at 256 bits.
                let lhs = self.emit_u256_mul_wide(x, y)?;
                let rhs = self.emit_u256_mul_wide(z, wd)?;
                let _ = self.fin_pop_pair(); // w
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                let a0 = self.a0;
                let (lt_t, eq_t, gt_t) = match spec.kernel {
                    FinKernel::CrossLte => (true, true, false),
                    FinKernel::CrossGte => (false, true, true),
                    _ => (false, true, false), // CrossEq
                };
                let l_yes = self.asm.new_label();
                let l_no = self.asm.new_label();
                let l_end = self.asm.new_label();
                self.emit_u256_branch3(
                    &lhs,
                    &rhs,
                    if lt_t { l_yes } else { l_no },
                    if eq_t { l_yes } else { l_no },
                    if gt_t { l_yes } else { l_no },
                );
                self.place(l_yes);
                self.asm.addi(a0, ZERO, 1);
                self.asm.jal_(ZERO, l_end);
                self.place(l_no);
                self.asm.addi(a0, ZERO, 0);
                self.place(l_end);
                let d = self.push_vreg();
                self.asm.mv(d, a0);
            }
            _ => {
                return errf!(
                    "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                    spec.name
                );
            }
        }
        Ok(())
    }

    /// FIN3 lowering (mul_div_* / scaled_div_* / dev_scaled_* /
    /// scaled_add|sub_* / mul_shr_* / mul_div_den_add|sub_* / u64_mad —
    /// instruction.rs fin3_checked via triop_fin).
    pub(crate) fn emit_fin3(&mut self, n: &IRNodeParam1Triple) -> Ret<()> {
        let spec =
            fin_spec_lookup(Bytecode::FIN3, n.para).map_err(|e| format!("rv64 lower: {e}"))?;
        let wx = self.fin_arg_width(self.int_width(&*n.subx), spec.name)?;
        let wy = self.fin_arg_width(self.int_width(&*n.suby), spec.name)?;
        let wz = self.fin_arg_width(self.int_width(&*n.subz), spec.name)?;
        // u64_mad keeps the ORIGINAL base width (triop_fin's special case);
        // every other FIN3 kernel casts back to the three-way common width.
        let mad = spec.kernel == FinKernel::U64Mad;
        let w = if mad { wx } else { wx.max(wy).max(wz) };
        let (lx, ly, lz) = (self.nil_val(&*n.subx), self.nil_val(&*n.suby), self.nil_val(&*n.subz));
        self.fin_stage_args(
            &[(&*n.subx, wx), (&*n.suby, wy), (&*n.subz, wz)],
            lx || ly || lz,
        )?;
        self.fin_nil_guards(&[lx, ly, lz])?;
        let base = self.vstack.len() - 6;
        let x = self.fin_pair_at(base);
        let y = self.fin_pair_at(base + 2);
        let z = self.fin_pair_at(base + 4);
        match spec.kernel {
            FinKernel::MulDiv | FinKernel::ScaledDiv => {
                // muldiv_with_round_checked: q = round(x·y / z); z == 0
                // faults inside the routine (90).
                let limbs = self.emit_u256_mul_wide(x, y)?;
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.emit_u256_divmod_u128(true, &limbs, z)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, z)?;
                let _ = self.fin_pop_pair(); // r
                let (q0, q1) = self.fin_pop_pair();
                self.emit_fin_publish(q0, q1, w);
            }
            FinKernel::DevScaled => {
                // devscaled_with_round_checked: reference = y, scale = z
                // (either 0 → 90), diff = |x-y|, q = round(diff·scale /
                // reference) — the policy rounds against the REFERENCE.
                let a0 = self.a0;
                self.asm.or_(a0, y.0, y.1);
                self.asm.beq(a0, ZERO, self.fault_arith);
                self.asm.or_(a0, z.0, z.1);
                self.asm.beq(a0, ZERO, self.fault_arith);
                self.fin_park(U256WS_FINA, y); // the reference
                self.emit_u128_cmp(Bytecode::LT, a0, x, y);
                let l_lt = self.asm.new_label();
                let l_join = self.asm.new_label();
                self.asm.bne(a0, ZERO, l_lt);
                self.emit_u128_sub(x.0, x.1, y.0, y.1); // x ≥ y → x - y
                self.asm.jal_(ZERO, l_join);
                self.place(l_lt);
                self.emit_u128_sub(y.0, y.1, x.0, x.1); // x < y → y - x
                self.asm.mv(x.0, y.0);
                self.asm.mv(x.1, y.1);
                self.place(l_join);
                let limbs = self.emit_u256_mul_wide(x, z)?; // diff · scale
                // reload the reference above the operands
                let r0 = self.push_vreg();
                let r1 = self.push_vreg();
                self.fin_load(U256WS_FINA, (r0, r1));
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.emit_u256_divmod_u128(true, &limbs, (r0, r1))?;
                let q = self.fin_pair_at(base + 2);
                let r = self.fin_pair_at(base + 4);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, (r0, r1))?;
                let _ = self.fin_pop_pair(); // r
                let _ = self.fin_pop_pair(); // q
                let _ = self.fin_pop_pair(); // ref temps
                let (q0, q1) = (q.0, q.1);
                self.emit_fin_publish(q0, q1, w);
            }
            FinKernel::ScaledAdd | FinKernel::ScaledSub => {
                // scaled_addsub_checked: delta = round(x·y / z), then
                // value ± delta with the checked u128 face.
                self.fin_park(U256WS_FINA, x); // the value
                let limbs = self.emit_u256_mul_wide(x, y)?;
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.emit_u256_divmod_u128(true, &limbs, z)?;
                let q = self.fin_pair_at(base);
                let r = self.fin_pair_at(base + 2);
                self.emit_u128_round_quot(spec.round_or_exact(), q, r, z)?;
                let _ = self.fin_pop_pair(); // r → q entries remain
                self.fin_load(U256WS_FINA, z); // the value → z regs
                if spec.kernel == FinKernel::ScaledAdd {
                    self.emit_u128_add(q.0, q.1, z.0, z.1);
                    let (q0, q1) = self.fin_pop_pair();
                    self.emit_fin_publish(q0, q1, w);
                } else {
                    // value - delta: the MINUEND is the value (underflow
                    // on borrow → 90, the checked_sub face)
                    self.emit_u128_sub(z.0, z.1, q.0, q.1);
                    let _ = self.fin_pop_pair(); // q
                    self.emit_fin_publish(z.0, z.1, w);
                }
            }
            FinKernel::MulShr => {
                // mul_shr_impl: out = (x·y) >> z with the Ceil variant
                // bumping on dropped bits; the interpreter's shr faces
                // (the result would not fit u128) reproduce exactly —
                // for s < 128 the division by 2^s carries the same face
                // (N_hi >= 2^s ⟺ N >> s leaves u128).
                let ceil = spec.round_or_exact() == FinRoundPolicy::Ceil;
                let a0 = self.a0;
                let s = self.u256_save_off;
                let limbs = self.emit_u256_mul_wide(x, y)?;
                for (i, &r) in limbs.iter().enumerate() {
                    self.asm.sd(r, S0, s + U256WS_FINA + (i as i32) * 8);
                }
                let l_big = self.asm.new_label();
                let l_ge128 = self.asm.new_label();
                let l_end = self.asm.new_label();
                // three-way shift split (z's registers are live here)
                self.asm.bne(z.1, ZERO, l_big); // shift >= 2^64 => >= 256
                self.asm.li(a0, 256);
                self.asm.bgeu(z.0, a0, l_big);
                self.asm.li(a0, 128);
                self.asm.bgeu(z.0, a0, l_ge128);
                {
                    // s in [0, 127]: out = round(N / 2^s)
                    let _ = self.fin_pop_pair(); // z (registers keep the value)
                    let _ = self.fin_pop_pair(); // y
                    let _ = self.fin_pop_pair(); // x
                    let d0 = self.push_vreg();
                    let d1 = self.push_vreg();
                    let l_s_hi = self.asm.new_label();
                    let l_dmul = self.asm.new_label();
                    let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                    let _ = sc2;
                    self.asm.li(a0, 64);
                    self.asm.bgeu(z.0, a0, l_s_hi);
                    self.asm.addi(d0, ZERO, 1);
                    self.asm.r(0, z.0, d0, 1, d0, 0b0110011); // d0 = 1 << s
                    self.asm.addi(d1, ZERO, 0);
                    self.asm.jal_(ZERO, l_dmul);
                    self.place(l_s_hi);
                    self.asm.sub(sc1, z.0, a0); // s - 64 (a0 = 64)
                    self.asm.addi(d1, ZERO, 1);
                    self.asm.r(0, sc1, d1, 1, d1, 0b0110011); // d1 = 1 << (s-64)
                    self.asm.addi(d0, ZERO, 0);
                    self.place(l_dmul);
                    // the product limbs into the dead y/z registers
                    self.asm.ld(y.0, S0, s + U256WS_FINA);
                    self.asm.ld(y.1, S0, s + U256WS_FINA + 8);
                    self.asm.ld(z.0, S0, s + U256WS_FINA + 16);
                    self.asm.ld(z.1, S0, s + U256WS_FINA + 24);
                    let n256 = [y.0, y.1, z.0, z.1];
                    self.emit_u256_divmod_u128(true, &n256, (d0, d1))?;
                    let q = self.fin_pair_at(base + 2);
                    let r = self.fin_pair_at(base + 4);
                    self.emit_u128_round_quot(
                        if ceil { FinRoundPolicy::Ceil } else { FinRoundPolicy::Floor },
                        q,
                        r,
                        (d0, d1),
                    )?;
                    let _ = self.fin_pop_pair(); // r
                    let _ = self.fin_pop_pair(); // q
                    let _ = self.fin_pop_pair(); // d pair
                    let (q0, q1) = (q.0, q.1);
                    self.emit_fin_publish(q0, q1, w);
                    self.asm.jal_(ZERO, l_end);
                }
                self.place(l_ge128);
                {
                    // t = s - 128 in [0, 127]: out = hi >> t, dropped = the
                    // low 128 bits or the shifted-out hi bits. The previous
                    // arm's operand pops/publish are lowering-time only, so
                    // this runtime arm starts with that arm's published
                    // entries: discard them, then allocate the output pair.
                    for _ in 0..Self::entries_of_width(Some(w)) {
                        let _ = self.pop_vreg();
                    }
                    let o0 = self.push_vreg();
                    let o1 = self.push_vreg();
                    let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                    let t = sc1;
                    self.asm.li(a0, 128);
                    self.asm.sub(t, z.0, a0); // t
                    // lo -> y regs, hi -> z regs (dead after the parks)
                    self.asm.ld(y.0, S0, s + U256WS_FINA);
                    self.asm.ld(y.1, S0, s + U256WS_FINA + 8);
                    self.asm.ld(z.0, S0, s + U256WS_FINA + 16);
                    self.asm.ld(z.1, S0, s + U256WS_FINA + 24);
                    // a0 = the dropped-bits accumulator (nonzero suffices)
                    self.asm.or_(a0, y.0, y.1);
                    let l_t0 = self.asm.new_label();
                    let l_dtest = self.asm.new_label();
                    self.asm.beq(t, ZERO, l_t0);
                    self.asm.li(sc2, 64);
                    let l_thi = self.asm.new_label();
                    self.asm.bgeu(t, sc2, l_thi);
                    // t in [1, 63]
                    self.asm.sub(sc2, sc2, t); // 64 - t
                    self.asm.r(0, t, z.1, 5, o1, 0b0110011); // o1 = hi_hi >> t
                    self.asm.r(0, sc2, z.1, 1, z.1, 0b0110011); // hi_hi << (64-t)
                    self.asm.r(0, t, z.0, 5, o0, 0b0110011); // o0 = hi_lo >> t
                    self.asm.or_(o0, o0, z.1);
                    // dropped |= hi_lo & ((1<<t)-1) — mask via ~(~0 << t)
                    self.asm.addi(z.1, ZERO, -1);
                    self.asm.r(0, t, z.1, 1, z.1, 0b0110011);
                    self.asm.i(-1, z.1, 4, z.1, 0b0010011);
                    self.asm.and_(z.1, z.1, z.0);
                    self.asm.or_(a0, a0, z.1);
                    self.asm.jal_(ZERO, l_dtest);
                    self.place(l_thi);
                    // t in [64, 127]: out = hi_hi >> (t-64); dropped =
                    // lo | hi_lo | (hi_hi & ((1<<(t-64))-1))
                    self.asm.sub(sc2, t, sc2); // rh = t - 64
                    self.asm.r(0, sc2, z.1, 5, o0, 0b0110011); // o0 = hi_hi >> rh
                    self.asm.addi(o1, ZERO, -1);
                    self.asm.r(0, sc2, o1, 1, o1, 0b0110011); // -1 << rh
                    self.asm.i(-1, o1, 4, o1, 0b0010011); // ~ -> mask
                    self.asm.and_(o1, o1, z.1);
                    self.asm.or_(a0, a0, z.0); // hi_lo != 0
                    self.asm.or_(a0, a0, o1);
                    self.asm.addi(o1, ZERO, 0);
                    self.asm.jal_(ZERO, l_dtest);
                    self.place(l_t0);
                    self.asm.mv(o0, z.0); // out = hi
                    self.asm.mv(o1, z.1);
                    self.place(l_dtest);
                    if ceil {
                        let l_nodrop = self.asm.new_label();
                        self.asm.r(0, a0, ZERO, 3, sc2, 0b0110011); // sc2 = a0 != 0
                        self.asm.beq(sc2, ZERO, l_nodrop);
                        self.asm.addi(sc2, ZERO, 1);
                        self.emit_u128_add(o0, o1, sc2, ZERO); // checked -> 90
                        self.place(l_nodrop);
                    }
                    let (o0r, o1r) = self.fin_pop_pair();
                    self.emit_fin_publish(o0r, o1r, w);
                    self.asm.jal_(ZERO, l_end);
                }
                self.place(l_big);
                {
                    // shift >= 256: everything shifts out — out = 0, or 1
                    // under Ceil when the product was nonzero. Same join
                    // discipline: discard the previous arm's published
                    // entries, then allocate this arm's output pair.
                    for _ in 0..Self::entries_of_width(Some(w)) {
                        let _ = self.pop_vreg();
                    }
                    let o0 = self.push_vreg();
                    let o1 = self.push_vreg();
                    self.asm.ld(y.0, S0, s + U256WS_FINA);
                    self.asm.ld(y.1, S0, s + U256WS_FINA + 8);
                    self.asm.ld(z.0, S0, s + U256WS_FINA + 16);
                    self.asm.ld(z.1, S0, s + U256WS_FINA + 24);
                    self.asm.or_(self.a0, y.0, y.1);
                    self.asm.or_(self.a0, self.a0, z.0);
                    self.asm.or_(self.a0, self.a0, z.1);
                    self.asm.addi(o0, ZERO, 0);
                    self.asm.addi(o1, ZERO, 0);
                    if ceil {
                        self.asm.r(0, self.a0, ZERO, 3, o1, 0b0110011); // o1 = a0 != 0
                        self.asm.add(o0, o0, o1);
                        self.asm.addi(o1, ZERO, 0);
                    }
                    let (o0r, o1r) = self.fin_pop_pair();
                    self.emit_fin_publish(o0r, o1r, w);
                }
                self.place(l_end);
            }
            FinKernel::MulDivDenAdd | FinKernel::MulDivDenSub => {
                // muldiv_den_addsub_checked: den = z + y (u129 — the 256-bit
                // add of a u128 cannot overflow past bit 128) or z - y
                // (checked, then nonzero); q = round(x·y / den).
                if spec.kernel == FinKernel::MulDivDenAdd {
                    // den limbs = [z0, z1, t0, t1] with zero high limbs
                    let t0 = self.push_vreg();
                    self.asm.addi(t0, ZERO, 0);
                    let t1 = self.push_vreg();
                    self.asm.addi(t1, ZERO, 0);
                    let den = [z.0, z.1, t0, t1];
                    self.emit_u256_add_u128(&den, y);
                    let s = self.u256_save_off;
                    self.asm.sd(z.0, S0, s + U256WS_FINA); // den lo
                    self.asm.sd(z.1, S0, s + U256WS_FINA + 8);
                    self.asm.sd(t0, S0, s + U256WS_FINB); // den bit 128
                    let limbs = self.emit_u256_mul_wide(x, y)?;
                    let _ = self.fin_pop_pair(); // t pair
                    let _ = self.fin_pop_pair(); // z
                    let _ = self.fin_pop_pair(); // y
                    let _ = self.fin_pop_pair(); // x
                    self.fin_load(U256WS_FINA, z);
                    // den lo back into the dead z regs; the bit stays parked
                    self.emit_u256_divmod_u129(true, &limbs, z, t0)?;
                    let q = self.fin_pair_at(base);
                    let r = self.fin_pair_at(base + 2);
                    let rh = loc_reg(*self.vstack.get(base + 4).expect("fin rh entry"));
                    // Floor/Ceil only — the policy tail never reads d/dh
                    // (the FINA/FINB parks hold the true divisor)
                    self.emit_u129_round_quot(
                        spec.round_or_exact(),
                        q,
                        r,
                        rh,
                        z,
                        t0,
                        None,
                    )?;
                    let _ = self.pop_reg(); // rh (single entry)
                    let _ = self.fin_pop_pair(); // r
                    let (q0, q1) = self.fin_pop_pair();
                    self.emit_fin_publish(q0, q1, w);
                } else {
                    self.emit_u128_sub(z.0, z.1, y.0, y.1); // den = z - y
                    self.asm.or_(self.a0, z.0, z.1);
                    self.asm.beq(self.a0, ZERO, self.fault_arith); // den == 0
                    let limbs = self.emit_u256_mul_wide(x, y)?;
                    let _ = self.fin_pop_pair(); // z
                    let _ = self.fin_pop_pair(); // y
                    let _ = self.fin_pop_pair(); // x
                    self.emit_u256_divmod_u128(true, &limbs, z)?;
                    let q = self.fin_pair_at(base);
                    let r = self.fin_pair_at(base + 2);
                    self.emit_u128_round_quot(spec.round_or_exact(), q, r, z)?;
                    let _ = self.fin_pop_pair(); // r
                    let (q0, q1) = self.fin_pop_pair();
                    self.emit_fin_publish(q0, q1, w);
                }
            }
            FinKernel::U64Mad => {
                // u64_mad_checked: index·stride must fit u128 (hi ≠ 0 →
                // 90), the sum must fit u128 (checked add), and the result
                // casts back to the BASE's own width (the FIN3 exception).
                let limbs = self.emit_u256_mul_wide(y, z)?; // index · stride
                let _ = self.fin_pop_pair(); // z
                let _ = self.fin_pop_pair(); // y
                let _ = self.fin_pop_pair(); // x
                self.asm.or_(self.a0, limbs[2], limbs[3]);
                self.asm.bne(self.a0, ZERO, self.fault_arith); // hi ≠ 0
                self.emit_u128_add(limbs[0], limbs[1], x.0, x.1); // + base
                self.emit_fin_publish(limbs[0], limbs[1], w);
            }
            _ => {
                return errf!(
                    "rv64 lower: fin kernel {} is outside the translatable kernel subset",
                    spec.name
                );
            }
        }
        Ok(())
    }
}
