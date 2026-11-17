use super::*;

impl<'a> Lower<'a> {

    /* ================================================================== */
    /* ---- P1 (§3 shared prework): the u256 wide-arithmetic library ----- */
    /*                                                                    */
    /* Emit-side twins of the interpreter's 256-bit intermediate helpers  */
    /* (`vm/src/interpreter/instruction.rs`: `mul_wide_u128`,             */
    /* `add_u256_u128`/`sub_u256_u128`, `div_u256_by_u128_to_u128`,      */
    /* `div_u256_by_u129_to_u128`, `round_quot_u128_with_policy`, the    */
    /* `div_u256_by_u{128,129}_with_round` composites). The FIN-family,   */
    /* MULDIV lowering tasks (T26/T3/T4) reproduce the interpreter's      */
    /* "extract u128 → u256 intermediate → round → fit back" pipelines    */
    /* on top of THESE functions — the interpreter's wide intermediates   */
    /* are observable behavior, the compiler must not invent others.      */
    /*                                                                    */
    /* REPRESENTATION (the library's dataflow contract):                  */
    /* * a u128 value is a (lo, hi) u64 pair (the task-3 convention);    */
    /* * a u256 value in registers is 4 u64 limbs `[n0, n1, n2, n3]`,    */
    /*   n0 = bits 0..63 (little-endian limb order); on the vstack it    */
    /*   would occupy 4 entries pushed low-first (n0 deepest);           */
    /* * operand/result registers of the straight-line helpers must be   */
    /*   disjoint from the mode scratch set {a0, a1, sc1, sc2} (vstack   */
    /*   pool registers always are) and are dead-at-call-site scratch    */
    /*   INSIDE them;                                      */
    /* * the two division ROUTINES follow the u128 divmod convention:    */
    /*   all operand/result traffic through the frame workspace at       */
    /*   `u256_save_off` (reserved iff `body_needs_u256`), the caller    */
    /*   parks the kernel-entry RA at u+192 around the `jal`, and both   */
    /*   fault paths divide-by-zero → Arithmetic 90 via `fault_arith`    */
    /*   from inside the routine — identical faces to the interpreter's  */
    /*   `Option` → `err()` closes.                                       */

    /// Register the shared 256/128 soft-division routine label (lazily —
    /// the routine is emitted after the body only when a site called it).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn u256_divmod_label(&mut self) -> usize {
        *self
            .u256div_lbl
            .get_or_insert_with(|| self.asm.new_label())
    }

    /// Register the shared 256/129 soft-division routine label (lazily).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn u256_divmod129_label(&mut self) -> usize {
        *self
            .u256div129_lbl
            .get_or_insert_with(|| self.asm.new_label())
    }

    /// `n = x * y`, 128×128 → 256 (emit twin of `mul_wide_u128`). `x` and
    /// `y` are (lo, hi) u64 pairs; the 256-bit product lands back in the
    /// SAME four operand registers, returned as limbs `[n0, n1, n2, n3]`
    /// (n0 = bits 0..63), i.e. `[(x.0), (x.1), (y.0), (y.1)]` in order.
    /// Never faults (the interpreter's widening multiply cannot). Uses
    /// scratch sc1/sc2/a0 and parks the low product limb in the u256
    /// workspace's N0 slot for two instructions — the workspace must be
    /// reserved (`body_needs_u256`) and its prior contents are dead.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_mul_wide(&mut self, x: (u8, u8), y: (u8, u8)) -> Ret<[u8; 4]> {
        let (x0, x1) = x;
        let (y0, y1) = y;
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 mul_wide needs the u256 frame workspace (body_needs_u256)"
            );
        }
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let a0 = self.a0;
        let mulhu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(1, rs2, rs1, 3, rd, 0b0110011);
        };
        let mul = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(1, rs2, rs1, 0, rd, 0b0110011);
        };
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        // schoolbook 2×2 limbs: P = A..G partial products,
        //   p0 = lo(x0·y0)
        //   p1 = hh(x0·y0) + lo(x0·y1) + lo(x1·y0)
        //   p2 = hh(x0·y1) + hh(x1·y0) + lo(x1·y1) + carry(p1)
        //   p3 = hh(x1·y1) + carry(p2)
        // Register plan (3 temps + the 4 operand regs; p0 parked in the
        // workspace): sc1 = hh(x0·y0) running p1 sum, sc2 = hh(x0·y1)
        // running p2 sum, a0 = transient carry/product, x0/x1/y0/y1 are
        // progressively repurposed (each after its last original read).
        mulhu(self, sc1, x0, y0); // sc1 = hh(x0·y0)
        mul(self, sc2, x0, y0); // sc2 = p0
        self.asm.sd(sc2, S0, s + U256WS_N0); // park p0 (ws contents dead)
        mulhu(self, sc2, x0, y1); // sc2 = hh(x0·y1)
        mul(self, a0, x0, y1); // a0  = lo(x0·y1)   [x0 orig dead]
        mul(self, x0, x1, y0); // x0  = lo(x1·y0)   (x0 repurposed)
        self.asm.add(sc1, sc1, a0); // sc1 += lo(x0·y1)
        sltu(self, a0, sc1, a0); // a0 = carry ca
        self.asm.add(sc1, sc1, x0); // sc1 = p1 (mod 2^64)
        sltu(self, x0, sc1, x0); // x0 = carry cb     [old lo(x1·y0) read]
        self.asm.add(a0, a0, x0); // a0 = c1 = ca+cb ∈ {0,1,2}
        mulhu(self, x0, x1, y0); // x0  = hh(x1·y0)  [y0 orig dead]
        self.asm.add(sc2, sc2, x0); // sc2 += hh(x1·y0)
        sltu(self, x0, sc2, x0); // x0 = carry cc
        mulhu(self, y0, x1, y1); // y0  = hh(x1·y1)  (y0 repurposed)
        mul(self, x1, x1, y1); // x1  = lo(x1·y1)  [x1/y1 orig dead]
        self.asm.add(sc2, sc2, x1); // sc2 += lo(x1·y1)
        sltu(self, x1, sc2, x1); // x1 = carry cd
        self.asm.add(sc2, sc2, a0); // sc2 = p2 (mod 2^64)
        sltu(self, a0, sc2, a0); // a0 = carry ce
        self.asm.add(x0, x0, x1); // x0 = cc+cd       [x1 free]
        self.asm.add(x0, x0, a0); // x0 = c2 = cc+cd+ce
        self.asm.add(y1, y0, x0); // y1 = p3 = hh(x1·y1) + c2
        self.asm.ld(x0, S0, s + U256WS_N0); // x0 = p0 (unpark)
        self.asm.mv(x1, sc1); // x1 = p1
        self.asm.mv(y0, sc2); // y0 = p2
        Ok([x0, x1, y0, y1])
    }

    /// `n += z` at 256 bits (emit twin of `add_u256_u128`): carry out of
    /// bit 255 → Arithmetic 90, the interpreter's `None` → `err()` face.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_add_u128(&mut self, n: &[u8; 4], z: (u8, u8)) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let a0 = self.a0;
        let (n0, n1, n2, n3) = (n[0], n[1], n[2], n[3]);
        // ripple the u128 addend through the low 128 bits, then the two
        // borrows-of-carry (each ∈ {0,1}, sum ≤ 2 fits one add) into n2
        self.asm.add(n0, n0, z.0);
        self.asm.r(0, z.0, n0, 3, sc1, 0b0110011); // carry c
        self.asm.add(n1, n1, z.1);
        self.asm.r(0, z.1, n1, 3, sc2, 0b0110011); // carry d
        self.asm.add(n1, n1, sc1);
        self.asm.r(0, sc1, n1, 3, a0, 0b0110011); // fold carry e
        self.asm.add(sc2, sc2, a0); // carries into n2 (≤ 2)
        self.asm.add(n2, n2, sc2);
        self.asm.r(0, sc2, n2, 3, sc2, 0b0110011); // carry out of n2
        self.asm.add(n3, n3, sc2);
        self.asm.r(0, sc2, n3, 3, sc2, 0b0110011); // carry out of bit 255
        self.asm.bne(sc2, ZERO, self.fault_arith);
    }

    /// `n -= z` at 256 bits (emit twin of `sub_u256_u128`): borrow out of
    /// bit 255 (underflow) → Arithmetic 90.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_sub_u128(&mut self, n: &[u8; 4], z: (u8, u8)) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let a0 = self.a0;
        let (n0, n1, n2, n3) = (n[0], n[1], n[2], n[3]);
        self.asm.r(0, z.0, n0, 3, sc1, 0b0110011); // borrow c = n0 < z0
        self.asm.sub(n0, n0, z.0);
        self.asm.r(0, z.1, n1, 3, sc2, 0b0110011); // borrow d = n1 < z1
        self.asm.sub(n1, n1, z.1);
        // n1 -= c with its own borrow e (captured BEFORE the subtract)
        self.asm.r(0, sc1, n1, 3, a0, 0b0110011); // e = n1 < c
        self.asm.sub(n1, n1, sc1);
        self.asm.add(sc2, sc2, a0); // borrows into n2 (≤ 2)
        self.asm.r(0, sc2, n2, 3, a0, 0b0110011); // borrow e2 = n2 < (d+e)
        self.asm.sub(n2, n2, sc2);
        self.asm.r(0, a0, n3, 3, sc2, 0b0110011); // final borrow
        self.asm.sub(n3, n3, a0);
        self.asm.bne(sc2, ZERO, self.fault_arith);
    }

    /// `n <<= 1` at 256 bits, wrapping (the u129 rounding path's
    /// `add_u256(rem, rem)` — the doubling of a value < 2^129 can never
    /// overflow, so no fault face exists).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_double_in_place(&mut self, n: &mut [u8; 4]) {
        let (n0, n1, n2, n3) = (n[0], n[1], n[2], n[3]);
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        self.asm.srli_(sc1, n3, 63);
        self.asm.slli_(n3, n3, 1);
        self.asm.srli_(sc2, n2, 63);
        self.asm.slli_(n2, n2, 1);
        self.asm.or_(n2, n2, sc1);
        self.asm.srli_(sc1, n1, 63);
        self.asm.slli_(n1, n1, 1);
        self.asm.or_(n1, n1, sc2);
        self.asm.slli_(n0, n0, 1);
        self.asm.or_(n0, n0, sc1);
    }

    /// `d = (a CMP b)` as a 0/1 byte over full 256-bit values (emit twin
    /// of `cmp_u256`'s ordering uses — `withinbps`/`crossmul` predicates
    /// and the u129 rounding compares). `a`/`b` are 4-limb arrays.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_cmp(&mut self, op: Bytecode, d: u8, a: &[u8; 4], b: &[u8; 4]) {
        let (a0l, a1l, a2l, a3l) = (a[0], a[1], a[2], a[3]);
        let (b0l, b1l, b2l, b3l) = (b[0], b[1], b[2], b[3]);
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        match op {
            Bytecode::EQ | Bytecode::NEQ => {
                self.asm.xor(sc1, a0l, b0l);
                self.asm.xor(sc2, a1l, b1l);
                self.asm.or_(sc1, sc1, sc2);
                self.asm.xor(sc2, a2l, b2l);
                self.asm.or_(sc1, sc1, sc2);
                self.asm.xor(sc2, a3l, b3l);
                self.asm.or_(sc1, sc1, sc2);
                if op == Bytecode::EQ {
                    self.asm.i(1, sc1, 3, d, 0b0010011); // seqz
                } else {
                    self.asm.r(0, sc1, ZERO, 3, d, 0b0110011); // snez
                }
            }
            Bytecode::LT | Bytecode::GE => {
                // hi u128 pair decides unless equal, then the lo pair
                let l_lo = self.asm.new_label();
                let l_a3 = self.asm.new_label();
                let l_have = self.asm.new_label();
                let l_done = self.asm.new_label();
                // sc2 == 0 ⟺ hi pairs (a3,a2)/(b3,b2) equal
                self.asm.xor(sc2, a3l, b3l);
                self.asm.xor(sc1, a2l, b2l);
                self.asm.or_(sc2, sc2, sc1);
                self.asm.beq(sc2, ZERO, l_lo);
                // hi differs: sc1 = hi pair LT (u128 compare inline)
                self.asm.bne(a3l, b3l, l_a3);
                self.asm.r(0, b2l, a2l, 3, sc1, 0b0110011); // a2 < b2
                self.asm.jal_(ZERO, l_have);
                self.place(l_a3);
                self.asm.r(0, b3l, a3l, 3, sc1, 0b0110011); // a3 < b3
                self.place(l_have);
                if op == Bytecode::LT {
                    self.asm.mv(d, sc1);
                } else {
                    self.asm.i(1, sc1, 3, d, 0b0010011); // seqz (invert)
                }
                self.asm.jal_(ZERO, l_done);
                // hi equal → the lo u128 pair decides
                self.place(l_lo);
                self.emit_u128_cmp(op, d, (a0l, a1l), (b0l, b1l));
                self.place(l_done);
            }
            _ => {
                // GT / LE: mirror the operand order, invert for LE
                self.emit_u256_cmp(
                    if op == Bytecode::GT { Bytecode::LT } else { Bytecode::GE },
                    d,
                    b,
                    a,
                );
            }
        }
    }

    /// Stage a 256-bit value (4 limb registers, low first) into the u256
    /// workspace's dividend slots N0..N3. The division helpers stage their
    /// operands themselves; this is exposed for composite chains that keep
    /// a wide intermediate across several ops (T3/T4/T26).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_ws_store_n(&mut self, n: &[u8; 4]) -> Ret<()> {
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 workspace store needs the u256 frame workspace (body_needs_u256)"
            );
        }
        for (i, &r) in n.iter().enumerate() {
            self.asm.sd(r, S0, s + U256WS_N0 + (i as i32) * 8);
        }
        Ok(())
    }

    /// Load the u256 workspace's dividend slots into 4 fresh pool
    /// registers (low limb first).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_ws_load_n(&mut self) -> Ret<[u8; 4]> {
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 workspace load needs the u256 frame workspace (body_needs_u256)"
            );
        }
        let mut out = [ZERO; 4];
        for i in 0..4 {
            out[i] = self.push_vreg();
        }
        // push order n0..n3 == pool positions ascending; load each
        for (i, &r) in out.iter().enumerate() {
            self.asm.ld(r, S0, s + U256WS_N0 + (i as i32) * 8);
        }
        Ok(out)
    }

    /// Call the shared 256/128 routine for `N / V` (emit twin of
    /// `div_u256_by_u128_to_u128`). `n` is the 256-bit dividend (4 limbs,
    /// low first) and `v` the u128 divisor pair, both already popped off
    /// the vstack. Faults (Arithmetic 90, from inside the routine) exactly
    /// on the interpreter's `None` faces: `V == 0` or `N_hi >= V` (the
    /// quotient would not fit u128). The quotient pair lands in two fresh
    /// pool registers; with `want_rem` the u128 remainder pair follows in
    /// two more (the rounding helpers consume both).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_divmod_u128(&mut self, want_rem: bool, n: &[u8; 4], v: (u8, u8)) -> Ret<()> {
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 division needs the u256 frame workspace (body_needs_u256)"
            );
        }
        self.emit_u256_ws_store_n(n)?;
        self.asm.sd(v.0, S0, s + U256WS_V0);
        self.asm.sd(v.1, S0, s + U256WS_V0 + 8);
        let lbl = self.u256_divmod_label();
        self.asm.sd(RA, S0, s + U256WS_RA);
        self.asm.jal_(RA, lbl);
        self.asm.ld(RA, S0, s + U256WS_RA);
        let q0 = self.push_vreg();
        let q1 = self.push_vreg();
        self.asm.ld(q0, S0, s + U256WS_Q0);
        self.asm.ld(q1, S0, s + U256WS_Q0 + 8);
        if want_rem {
            let r0 = self.push_vreg();
            let r1 = self.push_vreg();
            self.asm.ld(r0, S0, s + U256WS_R0);
            self.asm.ld(r1, S0, s + U256WS_R0 + 8);
        }
        Ok(())
    }

    /// Call the shared 256-bit division routines for a u129 divisor
    /// `2^128·dbit + v` (emit twin of `div_u256_by_u129_to_u128`,
    /// INCLUDING its dispatch: `dbit == 0` delegates to the 256/128
    /// routine with `rem_hi = 0`). Runtime `dbit > 1` → Arithmetic 90
    /// (the interpreter's `d_hi > 1` → `None` face); `dbit == 0 && v == 0`
    /// and `N_hi >= v` fault inside the 256/128 routine; a quotient that
    /// would not fit u128 faults inside the 256/129 routine. The quotient
    /// pair lands in two fresh pool registers; with `want_rem` the
    /// remainder follows as three more (R0, R1 limbs then the R bit-128
    /// limb RH — the u128-delegation path always pushes RH = 0).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u256_divmod_u129(
        &mut self,
        want_rem: bool,
        n: &[u8; 4],
        v: (u8, u8),
        dbit: u8,
    ) -> Ret<()> {
        let s = self.u256_save_off;
        if s < 0 {
            return errf!(
                "rv64 lower: u256 division needs the u256 frame workspace (body_needs_u256)"
            );
        }
        self.emit_u256_ws_store_n(n)?;
        self.asm.sd(v.0, S0, s + U256WS_V0);
        self.asm.sd(v.1, S0, s + U256WS_V0 + 8);
        self.asm.sd(dbit, S0, s + U256WS_DH);
        // dbit ∈ {0,1} by construction at every real call site, but the
        // interpreter checks `d_hi > 1` — reproduce the face
        let a0 = self.a0;
        self.asm.addi(a0, ZERO, 2);
        self.asm.bgeu(dbit, a0, self.fault_arith);
        let l_128 = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.asm.beq(dbit, ZERO, l_128);
        {
            let lbl = self.u256_divmod129_label();
            self.asm.sd(RA, S0, s + U256WS_RA);
            self.asm.jal_(RA, lbl);
            self.asm.ld(RA, S0, s + U256WS_RA);
            self.asm.jal_(ZERO, l_done);
        }
        self.place(l_128);
        {
            let lbl = self.u256_divmod_label();
            self.asm.sd(RA, S0, s + U256WS_RA);
            self.asm.jal_(RA, lbl);
            self.asm.ld(RA, S0, s + U256WS_RA);
            // the delegated path's remainder has bit 128 clear
            self.asm.sd(ZERO, S0, s + U256WS_RH);
        }
        self.place(l_done);
        let q0 = self.push_vreg();
        let q1 = self.push_vreg();
        self.asm.ld(q0, S0, s + U256WS_Q0);
        self.asm.ld(q1, S0, s + U256WS_Q0 + 8);
        if want_rem {
            let r0 = self.push_vreg();
            let r1 = self.push_vreg();
            let rh = self.push_vreg();
            self.asm.ld(r0, S0, s + U256WS_R0);
            self.asm.ld(r1, S0, s + U256WS_R0 + 8);
            self.asm.ld(rh, S0, s + U256WS_RH);
        }
        Ok(())
    }

    /// Apply a FIN rounding policy to a u128 quotient/remainder/divisor
    /// triple (emit twin of `round_quot_u128_with_policy` — the tail of
    /// `div_u256_by_u128_with_round` and the plain u128÷u128 rounded
    /// division face). `q` is UPDATED IN PLACE; the checked `quo + 1`
    /// faces (carry out of bit 127) and Exact's nonzero remainder fault
    /// Arithmetic 90, exactly the interpreter's `err()` closes. `q`, `r`
    /// and `d` are (lo, hi) pairs; `r` may be clobbered.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u128_round_quot(
        &mut self,
        policy: FinRoundPolicy,
        q: (u8, u8),
        r: (u8, u8),
        d: (u8, u8),
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
                self.asm.or_(a0, r.0, r.1);
                self.asm.bne(a0, ZERO, self.fault_arith);
            }
            FinRoundPolicy::Ceil => {
                // rem == 0 → keep quo; else checked quo+1
                self.asm.or_(a0, r.0, r.1);
                self.asm.beq(a0, ZERO, l_done);
                round_up(self);
            }
            FinRoundPolicy::HalfUp => {
                // interpreter: rem >= d - d/2 ⟺ rem + d/2 >= d (integer
                // equivalence, valid for every rem)
                let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                // (sc1, a0) = d >> 1 — d_hi's bit 0 lands at bit 63 of
                // the low limb
                self.asm.slli_(sc2, d.1, 63);
                self.asm.srli_(sc1, d.0, 1);
                self.asm.srli_(a0, d.1, 1);
                self.asm.or_(sc1, sc1, sc2);
                // (r.0, r.1) += (sc1, a0) — r dead after
                self.asm.add(r.0, r.0, sc1);
                self.asm.r(0, sc1, r.0, 3, sc2, 0b0110011); // carry cA
                self.asm.add(r.1, r.1, a0);
                self.asm.r(0, a0, r.1, 3, a0, 0b0110011); // carry cB
                self.asm.add(r.1, r.1, sc2);
                self.asm.r(0, sc2, r.1, 3, sc2, 0b0110011); // carry cC
                self.asm.or_(sc2, sc2, a0); // carry out (≤ 1) → sum ≥ 2^128
                let l_up = self.asm.new_label();
                self.asm.bne(sc2, ZERO, l_up);
                self.emit_u128_cmp(Bytecode::GE, a0, r, d);
                self.asm.beq(a0, ZERO, l_done);
                self.place(l_up);
                round_up(self);
            }
            FinRoundPolicy::HalfEven => {
                // rem == 0 → keep; rem > u128::MAX/2 (msb of rem.hi) → up;
                // else 2·rem vs d decides (the doubling cannot lose a bit
                // in this branch), tie → nearest-even
                self.asm.or_(a0, r.0, r.1);
                self.asm.beq(a0, ZERO, l_done);
                self.asm.srli_(a0, r.1, 63);
                let l_up = self.asm.new_label();
                self.asm.bne(a0, ZERO, l_up);
                // r = r << 1 (128-bit, in place)
                self.asm.srli_(a0, r.0, 63);
                self.asm.slli_(r.1, r.1, 1);
                self.asm.or_(r.1, r.1, a0);
                self.asm.slli_(r.0, r.0, 1);
                self.emit_u128_cmp(Bytecode::GT, a0, r, d);
                self.asm.bne(a0, ZERO, l_up);
                self.emit_u128_cmp(Bytecode::EQ, a0, r, d);
                self.asm.beq(a0, ZERO, l_done);
                self.asm.andi(a0, q.0, 1);
                self.asm.beq(a0, ZERO, l_done);
                self.place(l_up);
                round_up(self);
            }
        }
        self.place(l_done);
        Ok(())
    }

    /// Apply a FIN rounding policy to a 256÷129 division's quotient and
    /// 256-bit remainder (emit twin of the policy tail of
    /// `div_u256_by_u129_with_round`). `q` is UPDATED IN PLACE; the
    /// remainder is the divmod helper's (R0, R1, RH) triple; the divisor
    /// is `2^128·dh + d`. `half_even_parity_offset` is the interpreter's
    /// `base` register (only bit 0 is read — FINPOW3-style wavg2 uses it
    /// to break half-even ties against the FINAL result's parity) or
    /// `None` for constant 0. Checked `quo + 1` faces and Exact's nonzero
    /// remainder fault Arithmetic 90. `r_*` may be clobbered.
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u129_round_quot(
        &mut self,
        policy: FinRoundPolicy,
        q: (u8, u8),
        r_lo: (u8, u8),
        rh: u8,
        d: (u8, u8),
        dh: u8,
        half_even_parity_offset: Option<u8>,
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
                // dbl = rem + rem (the interpreter's add_u256(rem, rem);
                // rem < 2^129 so the double cannot overflow), then the
                // 3-limb value (rh, r1, r0) vs the divisor (dh, d1, d0)
                // decides. Inline 3-limb double — both msbs captured
                // before any shift clobbers them:
                let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
                self.asm.srli_(sc1, r_lo.1, 63);
                self.asm.srli_(sc2, r_lo.0, 63);
                self.asm.slli_(rh, rh, 1);
                self.asm.or_(rh, rh, sc1);
                self.asm.slli_(r_lo.1, r_lo.1, 1);
                self.asm.or_(r_lo.1, r_lo.1, sc2);
                self.asm.slli_(r_lo.0, r_lo.0, 1);
                match policy {
                    FinRoundPolicy::HalfUp => {
                        // dbl >= divisor → up
                        let l_up = self.asm.new_label();
                        self.emit_u129v_branch3(
                            (rh, r_lo.1, r_lo.0),
                            (dh, d.1, d.0),
                            l_up,
                            l_up,
                            l_done,
                        );
                        self.place(l_up);
                        round_up(self);
                    }
                    _ => {
                        // HalfEven: dbl > divisor → up; equal → tie broken
                        // against the FINAL result's parity ((quo & 1) !=
                        // (parity_offset & 1) — wavg2's `base` addend)
                        let l_up = self.asm.new_label();
                        let l_tie = self.asm.new_label();
                        self.emit_u129v_branch3(
                            (rh, r_lo.1, r_lo.0),
                            (dh, d.1, d.0),
                            l_up,
                            l_tie,
                            l_done,
                        );
                        self.place(l_tie);
                        match half_even_parity_offset {
                            Some(p) => {
                                self.asm.xor(sc1, q.0, p);
                                self.asm.andi(sc1, sc1, 1);
                            }
                            None => {
                                self.asm.andi(sc1, q.0, 1);
                            }
                        }
                        self.asm.beq(sc1, ZERO, l_done);
                        self.place(l_up);
                        round_up(self);
                    }
                }
            }
        }
        self.place(l_done);
        Ok(())
    }

    /// Three-way branch on `(x2, x1, x0) <cmp> (y2, y1, y0)` (129-bit
    /// values, x2/y2 the single limb above bit 127): jumps `l_gt`, `l_eq`
    /// or `l_lt`. The u129 rounding path's compare — `cmp_u256` restricted
    /// to the shapes the interpreter produces (top limb ≤ 3 transiently
    /// after the doubling, a plain u64 register holds it fine).
    // P1 u256 library: consumed by the T26/T3/T4 lowering tasks and
    // the u256_lib differential tests, not yet by any opcode lowering.
    #[allow(dead_code)]
    pub(crate) fn emit_u129v_branch3(
        &mut self,
        x: (u8, u8, u8),
        y: (u8, u8, u8),
        l_gt: usize,
        l_eq: usize,
        l_lt: usize,
    ) {
        let (x2, x1, x0) = x;
        let (y2, y1, y0) = y;
        let sc1 = self.asm.sc1;
        let l_top = self.asm.new_label();
        let l_mid = self.asm.new_label();
        self.asm.bne(x2, y2, l_top);
        self.asm.bne(x1, y1, l_mid);
        // both high limbs equal: x0 vs y0 decides
        self.asm.beq(x0, y0, l_eq);
        self.asm.r(0, y0, x0, 3, sc1, 0b0110011); // x0 < y0
        self.asm.bne(sc1, ZERO, l_lt);
        self.asm.jal_(ZERO, l_gt);
        // x1 vs y1 decides
        self.place(l_mid);
        self.asm.r(0, y1, x1, 3, sc1, 0b0110011); // x1 < y1
        self.asm.bne(sc1, ZERO, l_lt);
        self.asm.jal_(ZERO, l_gt);
        // x2 vs y2 decides
        self.place(l_top);
        self.asm.r(0, y2, x2, 3, sc1, 0b0110011); // x2 < y2
        self.asm.bne(sc1, ZERO, l_lt);
        self.asm.jal_(ZERO, l_gt);
    }


    /// Three-way branch on 4-limb values: `a < b` → `l_lt`, equal →
    /// `l_eq`, `a > b` → `l_gt`. No scratch registers (the isqrt accept
    /// test and the crossmul predicates run with all 8 pool slots live).
    pub(crate) fn emit_u256_branch3(&mut self, a: &[u8; 4], b: &[u8; 4], l_lt: usize, l_eq: usize, l_gt: usize) {
        let (a0, a1, a2, a3) = (a[0], a[1], a[2], a[3]);
        let (b0, b1, b2, b3) = (b[0], b[1], b[2], b[3]);
        let t1 = self.asm.new_label();
        let t2 = self.asm.new_label();
        let t3 = self.asm.new_label();
        self.asm.bne(a3, b3, t3);
        self.asm.bne(a2, b2, t2);
        self.asm.bne(a1, b1, t1);
        self.asm.beq(a0, b0, l_eq);
        self.asm.bltu(a0, b0, l_lt);
        self.asm.jal_(ZERO, l_gt);
        self.place(t1);
        self.asm.bltu(a1, b1, l_lt);
        self.asm.jal_(ZERO, l_gt);
        self.place(t2);
        self.asm.bltu(a2, b2, l_lt);
        self.asm.jal_(ZERO, l_gt);
        self.place(t3);
        self.asm.bltu(a3, b3, l_lt);
        self.asm.jal_(ZERO, l_gt);
    }

    /// `n += m` at 256 bits, carry out of bit 255 → Arithmetic 90 (emit
    /// twin of the interpreter's `add_u256`, the lerp numerator fold; the
    /// P1 library only ships the u128-addend form). Ripple carry with the
    /// ≤1 carry-chain property (two adds per limb, `c1 | c2` — both set
    /// is arithmetically impossible).
    pub(crate) fn emit_u256_add_full(&mut self, n: &[u8; 4], m: &[u8; 4]) {
        let (sc1, sc2) = (self.asm.sc1, self.asm.sc2);
        let a0 = self.a0;
        let limbs = [(n[0], m[0]), (n[1], m[1]), (n[2], m[2]), (n[3], m[3])];
        let c = sc2;
        for (i, &(ni, mi)) in limbs.iter().enumerate() {
            self.asm.add(ni, ni, mi);
            self.asm.r(0, mi, ni, 3, sc1, 0b0110011); // carry c1
            if i > 0 {
                self.asm.add(ni, ni, c);
                self.asm.r(0, c, ni, 3, a0, 0b0110011); // carry c2
                self.asm.or_(sc1, sc1, a0);
            }
            // c = sc1 for the next limb (sc2 keeps the running carry reg)
            if i < 3 {
                self.asm.mv(c, sc1);
            } else {
                self.asm.bne(sc1, ZERO, self.fault_arith);
            }
        }
    }


    /// The shared 256/128 soft-division routine: `Q = N / V`,
    /// `R = N % V` in one 128-iteration shift-subtract loop — the emit
    /// twin of the interpreter's `div_u256_by_u128_to_u128` (a restoring
    /// division; the mathematical result equals the interpreter's loop,
    /// and the FAULT faces are aligned bit for bit: `V == 0` or `N_hi >=
    /// V` → fault_arith 90 before the loop, the interpreter's `None` →
    /// `err()` faces). Frame convention: caller sds N to u+0..32 and V to
    /// u+32..48, parks RA at u+192 around the `jal`; the routine writes Q
    /// to u+56..72 and R to u+72..88. Working registers (all saved to
    /// u+96..192 first — they may be pool or bank homes in some modes):
    /// rem = a1:a0 (mode scratch, dead at every call site), the shifting
    /// dividend low pair = T2:T1 (x7:x6), V = x24:x23, Q = x4:x3, counter
    /// = x14, wrap carry = x22, transient = x17 (a7 — per the divmod
    /// routine's precedent, only the exit epilogue writes a7 elsewhere).
    pub(crate) fn emit_u256_divmod_u128_routine(&mut self) {
        let s = self.u256_save_off;
        const VLO: u8 = 23;
        const VHI: u8 = 24;
        const QLO: u8 = 3; // gp
        const QHI: u8 = 4; // tp
        const CARRY: u8 = 22; // wrap carry (saved; s6)
        const TMP: u8 = 17; // a7 — dead mid-body in every mode
        debug_assert!(s >= 0, "u256 routine without a reserved workspace");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        let saved = [T1, T2, 14u8, VLO, VHI, CARRY, QLO, QHI];
        // save the working registers BEFORE the operand loads overwrite
        // T1/T2 (they may be pool or bank homes in some modes)
        for (i, &r) in saved.iter().enumerate() {
            sd(self, r, s + U256WS_SAVE + (i as i32) * 8);
        }
        // fetch the operands from the frame workspace
        ld(self, T1, s + U256WS_N0); // N0 (bits 0..63)
        ld(self, T2, s + U256WS_N0 + 8); // N1 (bits 64..127) — shifting pair
        ld(self, VLO, s + U256WS_V0);
        ld(self, VHI, s + U256WS_V0 + 8);
        // rem = N_hi (bits 128..255)
        ld(self, self.a0, s + U256WS_N0 + 16);
        ld(self, self.a1, s + U256WS_N0 + 24);
        // V == 0 → 90 (divide-by-zero face)
        self.asm.or_(TMP, VLO, VHI);
        self.asm.beq(TMP, ZERO, self.fault_arith);
        // N_hi >= V → 90 (the quotient would not fit u128)
        let l_hi = self.asm.new_label();
        let l_ok = self.asm.new_label();
        self.asm.bne(self.a1, VHI, l_hi);
        sltu(self, TMP, self.a0, VLO); // hi equal: rem < V iff lo < vl
        self.asm.bne(TMP, ZERO, l_ok);
        self.asm.jal_(ZERO, self.fault_arith);
        self.place(l_hi);
        sltu(self, TMP, self.a1, VHI); // hi differ: rem < V iff rh < vh
        self.asm.bne(TMP, ZERO, l_ok);
        self.asm.jal_(ZERO, self.fault_arith);
        // q = 0, i = 128
        self.place(l_ok);
        self.asm.addi(QLO, ZERO, 0);
        self.asm.addi(QHI, ZERO, 0);
        self.asm.addi(14, ZERO, 128);
        let l_loop = self.asm.new_label();
        let l_sub = self.asm.new_label();
        let l_cmp = self.asm.new_label();
        let l_q0 = self.asm.new_label();
        let l_next = self.asm.new_label();
        self.place(l_loop);
        // c = msb(rem) BEFORE the doubling: a wrapped 2·rem+bit always
        // exceeds V (V < 2^128 ≤ 2·rem+bit), so the carry forces subtract
        self.asm.srli_(CARRY, self.a1, 63);
        // rem = rem << 1 | msb(N1)
        self.asm.srli_(TMP, self.a0, 63);
        self.asm.slli_(self.a1, self.a1, 1);
        self.asm.or_(self.a1, self.a1, TMP);
        self.asm.srli_(TMP, T2, 63); // next dividend bit
        self.asm.slli_(self.a0, self.a0, 1);
        self.asm.or_(self.a0, self.a0, TMP);
        // N1:N0 <<= 1
        self.asm.srli_(TMP, T1, 63);
        self.asm.slli_(T2, T2, 1);
        self.asm.or_(T2, T2, TMP);
        self.asm.slli_(T1, T1, 1);
        // decide: carry → subtract; else compare (rem, V) as 128-bit
        self.asm.bne(CARRY, ZERO, l_sub);
        self.asm.bne(self.a1, VHI, l_cmp);
        sltu(self, TMP, self.a0, VLO); // hi equal: rem < V iff lo < vl
        self.asm.beq(TMP, ZERO, l_sub); // lo ≥ vl → subtract
        self.asm.jal_(ZERO, l_q0); // no subtract
        self.place(l_cmp);
        sltu(self, TMP, VHI, self.a1); // hi differ: subtract iff vh < rh
        self.asm.beq(TMP, ZERO, l_q0); // vh > rh → no subtract
        self.place(l_sub);
        // rem -= V (128-bit, borrow chain)
        sltu(self, TMP, self.a0, VLO);
        self.asm.sub(self.a0, self.a0, VLO);
        self.asm.sub(self.a1, self.a1, VHI);
        self.asm.sub(self.a1, self.a1, TMP);
        // q = q << 1 | 1
        self.asm.srli_(TMP, QLO, 63);
        self.asm.slli_(QHI, QHI, 1);
        self.asm.or_(QHI, QHI, TMP);
        self.asm.slli_(QLO, QLO, 1);
        self.asm.addi(QLO, QLO, 1);
        self.asm.jal_(ZERO, l_next);
        self.place(l_q0);
        // q = q << 1 (bit 0)
        self.asm.srli_(TMP, QLO, 63);
        self.asm.slli_(QHI, QHI, 1);
        self.asm.or_(QHI, QHI, TMP);
        self.asm.slli_(QLO, QLO, 1);
        self.place(l_next);
        self.asm.addi(14, 14, -1);
        self.asm.bne(14, ZERO, l_loop);
        // park both results in the frame workspace before restoring
        sd(self, QLO, s + U256WS_Q0);
        sd(self, QHI, s + U256WS_Q0 + 8);
        sd(self, self.a0, s + U256WS_R0);
        sd(self, self.a1, s + U256WS_R0 + 8);
        for (i, &r) in saved.iter().enumerate() {
            ld(self, r, s + U256WS_SAVE + (i as i32) * 8);
        }
        self.asm.ret();
    }

    /// The shared 256/129 soft-division routine (divisor bit 128 SET —
    /// `2^128 + V`, the emit twin of `div_u256_by_u129_to_u128`'s
    /// `d_hi == 1` loop; the `d_hi == 0` delegation and the `d_hi > 1` /
    /// divide-by-zero faces live in the caller-side dispatch
    /// [`Self::emit_u256_divmod_u129`]). One 256-iteration
    /// shift-subtract loop over a 256-bit remainder whose top limb RH
    /// stays ≤ 3 transiently (rem < divisor ≤ 2^129 invariant). A
    /// subtract at a quotient bit position ≥ 128 → fault_arith 90 (the
    /// interpreter's `shift >= 128 → None`, i.e. the quotient would not
    /// fit u128). Frame convention: caller sds N to u+0..32, V to
    /// u+32..48, DH to u+48, parks RA at u+192; the routine writes Q to
    /// u+56..72, R to u+72..88 and RH to u+88. Working registers (all
    /// saved to u+96..192): rem_lo = a1:a0, RH = x25, the shifting
    /// dividend N3:N2 = T2:T1 and N1:N0 = T4:T3, V = x24:x23, VH = x22,
    /// Q = x4:x3, counter = x14, transients x17/x16.
    pub(crate) fn emit_u256_divmod_u129_routine(&mut self) {
        let s = self.u256_save_off;
        const VLO: u8 = 23;
        const VHI: u8 = 24;
        const VH: u8 = 22; // divisor bit 128 (== 1 on this path)
        const QLO: u8 = 3;
        const QHI: u8 = 4;
        const RH: u8 = 25;
        const TMP: u8 = 17; // a7
        const TMP2: u8 = 16; // a6
        debug_assert!(s >= 0, "u256 routine without a reserved workspace");
        let sd = |lw: &mut Self, rs: u8, off: i32| lw.asm.sd(rs, S0, off);
        let ld = |lw: &mut Self, rd: u8, off: i32| lw.asm.ld(rd, S0, off);
        let sltu = |lw: &mut Self, rd: u8, rs1: u8, rs2: u8| {
            lw.asm.r(0, rs2, rs1, 3, rd, 0b0110011);
        };
        let saved = [T1, T2, T3, T4, 14u8, TMP2, VH, VLO, VHI, RH, QLO, QHI];
        for (i, &r) in saved.iter().enumerate() {
            sd(self, r, s + U256WS_SAVE + (i as i32) * 8);
        }
        // N3 (bits 192..255) → T2, N2 → T1, N1 → T4, N0 → T3
        ld(self, T2, s + U256WS_N0 + 24);
        ld(self, T1, s + U256WS_N0 + 16);
        ld(self, T4, s + U256WS_N0 + 8);
        ld(self, T3, s + U256WS_N0);
        ld(self, VLO, s + U256WS_V0);
        ld(self, VHI, s + U256WS_V0 + 8);
        ld(self, VH, s + U256WS_DH);
        // rem = 0, q = 0, i = 256
        self.asm.addi(RH, ZERO, 0);
        self.asm.addi(self.a0, ZERO, 0);
        self.asm.addi(self.a1, ZERO, 0);
        self.asm.addi(QLO, ZERO, 0);
        self.asm.addi(QHI, ZERO, 0);
        self.asm.addi(14, ZERO, 256);
        let l_loop = self.asm.new_label();
        let l_sub = self.asm.new_label();
        let l_cmp = self.asm.new_label();
        let l_cmp2 = self.asm.new_label();
        let l_q0 = self.asm.new_label();
        let l_next = self.asm.new_label();
        self.place(l_loop);
        // next_bit = msb(N3) — captured before N shifts
        self.asm.srli_(TMP2, T2, 63);
        // N <<= 1 (4 limbs)
        self.asm.srli_(TMP, T1, 63);
        self.asm.slli_(T2, T2, 1);
        self.asm.or_(T2, T2, TMP);
        self.asm.srli_(TMP, T4, 63);
        self.asm.slli_(T1, T1, 1);
        self.asm.or_(T1, T1, TMP);
        self.asm.srli_(TMP, T3, 63);
        self.asm.slli_(T4, T4, 1);
        self.asm.or_(T4, T4, TMP);
        self.asm.slli_(T3, T3, 1);
        // c = msb(rem_lo); RH = RH << 1 | c
        self.asm.srli_(TMP, self.a1, 63);
        self.asm.slli_(RH, RH, 1);
        self.asm.or_(RH, RH, TMP);
        // rem_lo = rem_lo << 1 | next_bit
        self.asm.srli_(TMP, self.a0, 63);
        self.asm.slli_(self.a1, self.a1, 1);
        self.asm.or_(self.a1, self.a1, TMP);
        self.asm.slli_(self.a0, self.a0, 1);
        self.asm.or_(self.a0, self.a0, TMP2);
        // decide: (RH, rem_lo) >= (VH, V) as a 3-limb compare
        self.asm.bne(RH, VH, l_cmp);
        self.asm.bne(self.a1, VHI, l_cmp2);
        sltu(self, TMP, self.a0, VLO); // top two limbs equal: lo pair decides
        self.asm.beq(TMP, ZERO, l_sub);
        self.asm.jal_(ZERO, l_q0);
        self.place(l_cmp2);
        sltu(self, TMP, VHI, self.a1); // rem_hi limb differs
        self.asm.beq(TMP, ZERO, l_q0);
        self.asm.jal_(ZERO, l_sub);
        self.place(l_cmp);
        sltu(self, TMP, VH, RH); // top limb differs: subtract iff VH < RH
        self.asm.beq(TMP, ZERO, l_q0);
        self.place(l_sub);
        // quotient-overflow face: this subtract sets a quotient bit at
        // position ≥ 128 ⟺ counter > 128 (counter counts the REMAINING
        // iterations, bit position = counter - 1)
        self.asm.addi(TMP, ZERO, 129);
        self.asm.bgeu(14, TMP, self.fault_arith);
        // rem -= divisor (3-limb subtract; compare guaranteed ge at the
        // 256-bit level, but the low 128 bits may still borrow). Ripple
        // the two 64-bit borrows exactly like the interpreter's
        // `sub_u256`: a0 -= VLO (b0), a1 -= VHI - b0 with its own borrows,
        // then RH -= VH - borrow128.
        sltu(self, TMP, self.a0, VLO); // b0
        self.asm.sub(self.a0, self.a0, VLO);
        sltu(self, TMP2, self.a1, TMP); // b1a = a1 < b0 (fold's borrow)
        self.asm.sub(self.a1, self.a1, TMP); // a1 -= b0
        sltu(self, TMP, self.a1, VHI); // b1b = a1 < VHI
        self.asm.sub(self.a1, self.a1, VHI); // a1 -= VHI
        self.asm.or_(TMP, TMP, TMP2); // borrow out of the 128-bit sub
        self.asm.sub(RH, RH, VH);
        self.asm.sub(RH, RH, TMP);
        // q = q << 1 | 1
        self.asm.srli_(TMP, QLO, 63);
        self.asm.slli_(QHI, QHI, 1);
        self.asm.or_(QHI, QHI, TMP);
        self.asm.slli_(QLO, QLO, 1);
        self.asm.addi(QLO, QLO, 1);
        self.asm.jal_(ZERO, l_next);
        self.place(l_q0);
        // q = q << 1 (bit 0)
        self.asm.srli_(TMP, QLO, 63);
        self.asm.slli_(QHI, QHI, 1);
        self.asm.or_(QHI, QHI, TMP);
        self.asm.slli_(QLO, QLO, 1);
        self.place(l_next);
        self.asm.addi(14, 14, -1);
        self.asm.bne(14, ZERO, l_loop);
        // park the results before restoring
        sd(self, QLO, s + U256WS_Q0);
        sd(self, QHI, s + U256WS_Q0 + 8);
        sd(self, self.a0, s + U256WS_R0);
        sd(self, self.a1, s + U256WS_R0 + 8);
        sd(self, RH, s + U256WS_RH);
        for (i, &r) in saved.iter().enumerate() {
            ld(self, r, s + U256WS_SAVE + (i as i32) * 8);
        }
        self.asm.ret();
    }
}
